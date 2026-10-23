//! 보이지 않는 마우스 — 터미널 마우스 캡처로 OS 포인터의 도착을 판정한다
//! (docs/10 §6-4 조준·드래그·휠·새는 글자 0, S10).
//!
//! 판정은 결과만 본다. mouse 레이어로 움직였든 진짜 마우스로 움직였든 포인터가
//! 과녁에 닿으면 통과다(정직 시스템). 좌표는 과녁장 안쪽 기준이다.

use std::time::{Duration, Instant};

use crossterm::event::{KeyEvent, MouseButton, MouseEventKind};

use super::content::Lesson;
use super::play::{BossState, Rng, RoundEvent, Summary};

/// 과녁장 안쪽 크기
pub const ARENA_W: u16 = 76;
pub const ARENA_H: u16 = 13;
/// 무대 왼쪽 위에서 과녁장 안쪽까지의 거리 (ui 와 같은 값을 쓴다)
pub const ARENA_ORIGIN: (u16, u16) = (2, 4);
/// 휠 수색 목록의 보이는 줄 수
pub const LIST_VIEW: usize = 11;
/// 시작 전 3·2·1 (쥐마왕·새는 글자)
pub const COUNTDOWN: Duration = Duration::from_secs(3);
/// 새는 글자 0 — 마구 치는 시간
pub const LEAK_WINDOW: Duration = Duration::from_secs(6);
const RATS: usize = 16;
/// 쥐마왕을 쓰러뜨리는 데 필요한 포획 수
const RATS_TO_WIN: i32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drill {
    /// 포인터를 과녁에 올린다
    Move,
    /// 좌/우 버튼을 골라 클릭
    Aim,
    /// 긴 목록을 휠로 내려 목표를 클릭
    Wheel,
    /// 파일을 폴더로 끌어다 놓는다
    Drag,
    /// 레이어를 쥔 채 마구 쳐도 한 글자도 새지 않는다
    Leak,
    /// 쥐마왕 — 달리는 쥐를 잡는다
    Rats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= i32::from(self.x)
            && y >= i32::from(self.y)
            && x < i32::from(self.x + self.w)
            && y < i32::from(self.y + self.h)
    }
}

#[derive(Debug, Clone)]
pub struct Target {
    pub rect: Rect,
    /// 조준: 눌러야 하는 버튼
    pub button: Option<MouseButton>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RatState {
    Running,
    Caught(Instant),
    Escaped,
}

#[derive(Debug, Clone)]
pub struct Rat {
    pub y: u16,
    /// +1 오른쪽으로, -1 왼쪽으로
    pub dir: i8,
    /// 칸/초
    pub speed: f32,
    /// 등장 시각 (카운트다운 끝 기준)
    pub born: Duration,
    pub state: RatState,
}

pub const RAT_W: u16 = 4;

impl Rat {
    /// 지금 x (왼쪽 끝). 과녁장 밖이면 음수이거나 폭을 넘는다.
    pub fn x(&self, clock: Duration) -> f32 {
        let t = clock.saturating_sub(self.born).as_secs_f32();
        if self.dir > 0 {
            -f32::from(RAT_W) + self.speed * t
        } else {
            f32::from(ARENA_W) - self.speed * t
        }
    }

    pub fn glyph(&self) -> &'static str {
        if self.dir > 0 {
            "~o:>"
        } else {
            "<:o~"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeakPhase {
    Ready,
    /// 카운트다운 시작 시각
    Count(Instant),
    Active(Instant),
    Done,
}

pub struct MouseRound {
    pub lesson_id: String,
    pub drill: Drill,
    pub targets: Vec<Target>,
    pub idx: usize,
    pub pointer: Option<(i32, i32)>,
    /// 이 터미널이 포인터 움직임(1003)을 보고하는가
    pub moved_seen: bool,
    pub hits: u32,
    pub total_mistakes: u32,
    pub fail_streak: u32,
    pub par_time: Duration,
    pub started: Instant,
    pub finished_at: Option<Instant>,
    pub flash: Option<(bool, Instant)>,
    pub feedback: Option<(String, Instant)>,
    /// 맞힌 자리 (터지는 효과)
    pub burst: Option<((i32, i32), Instant)>,
    pub boss: Option<BossState>,
    // ── 휠
    pub list: Vec<String>,
    pub offset: usize,
    pub wanted: Vec<usize>,
    // ── 드래그
    pub icon: Rect,
    icon_home: (u16, u16),
    pub folder: Rect,
    pub dragging: Option<(i32, i32)>,
    // ── 새는 글자
    pub leak: LeakPhase,
    pub leaks: u32,
    pub activity: u32,
    // ── 쥐
    pub rats: Vec<Rat>,
    rng: Rng,
}

impl MouseRound {
    pub fn new(lesson: &Lesson, now: Instant) -> Option<Self> {
        let drill = match lesson.set.as_deref()? {
            "move" => Drill::Move,
            "aim" => Drill::Aim,
            "wheel" => Drill::Wheel,
            "drag" => Drill::Drag,
            "leak" => Drill::Leak,
            "rats" => Drill::Rats,
            _ => return None,
        };
        let mut rng = Rng::seeded(&lesson.id);
        let count = lesson.count.unwrap_or(8) as usize;
        let mut r = Self {
            lesson_id: lesson.id.clone(),
            drill,
            targets: Vec::new(),
            idx: 0,
            pointer: None,
            moved_seen: false,
            hits: 0,
            total_mistakes: 0,
            fail_streak: 0,
            par_time: Duration::from_secs(u64::from(lesson.par_time_s.unwrap_or(45))),
            started: now,
            finished_at: None,
            flash: None,
            feedback: None,
            burst: None,
            boss: lesson.is_boss().then(|| BossState::new(None)),
            list: Vec::new(),
            offset: 0,
            wanted: Vec::new(),
            icon: Rect {
                x: 0,
                y: 0,
                w: 10,
                h: 3,
            },
            icon_home: (0, 0),
            folder: Rect {
                x: 0,
                y: 0,
                w: 16,
                h: 5,
            },
            dragging: None,
            leak: LeakPhase::Ready,
            leaks: 0,
            activity: 0,
            rats: Vec::new(),
            rng: Rng::seeded(&lesson.id),
        };
        match drill {
            Drill::Move | Drill::Aim => r.targets = targets(&mut rng, drill, count),
            Drill::Wheel => {
                r.list = scroll_list();
                // 찾을 것은 비급 줄만 — 지금 자리에서 멀리, 휠을 충분히 굴리게
                let scrolls: Vec<usize> = (0..r.list.len())
                    .filter(|i| r.list[*i].starts_with('卷'))
                    .collect();
                let mut at = 0usize;
                for _ in 0..count {
                    let far = loop {
                        let i = scrolls[rng.below(scrolls.len() as u64) as usize];
                        if i.abs_diff(at) >= 20 {
                            break i;
                        }
                    };
                    r.wanted.push(far);
                    at = far;
                }
            }
            Drill::Drag => r.place_drag(),
            Drill::Rats => r.rats = rats(&mut rng),
            Drill::Leak => {}
        }
        r.targets.truncate(count.max(1));
        Some(r)
    }

    fn place_drag(&mut self) {
        let y = 1 + self.rng.below(u64::from(ARENA_H - self.icon.h - 2)) as u16;
        let fy = self.rng.below(u64::from(ARENA_H - self.folder.h)) as u16;
        self.icon_home = (3 + self.rng.below(10) as u16, y);
        self.icon.x = self.icon_home.0;
        self.icon.y = self.icon_home.1;
        self.folder.x = ARENA_W - self.folder.w - 2 - self.rng.below(8) as u16;
        self.folder.y = fy;
    }

    /// 지금 과녁 (이동·조준)
    pub fn target(&self) -> Option<&Target> {
        self.targets.get(self.idx)
    }

    /// 목표 개수와 진행
    pub fn progress(&self) -> (usize, usize) {
        match self.drill {
            Drill::Move | Drill::Aim => (self.idx, self.targets.len()),
            Drill::Wheel => (self.idx, self.wanted.len()),
            Drill::Drag => (self.idx, 5),
            Drill::Rats => (
                self.rats
                    .iter()
                    .filter(|r| matches!(r.state, RatState::Caught(_)))
                    .count(),
                RATS,
            ),
            Drill::Leak => (usize::from(self.leak == LeakPhase::Done), 1),
        }
    }

    pub fn clock(&self, now: Instant) -> Option<Duration> {
        now.duration_since(self.started).checked_sub(COUNTDOWN)
    }

    pub fn countdown_left(&self, now: Instant) -> Option<u64> {
        match (self.drill, self.leak) {
            (Drill::Rats, _) => {
                let e = now.duration_since(self.started);
                (e < COUNTDOWN).then(|| (COUNTDOWN - e).as_secs() + 1)
            }
            (Drill::Leak, LeakPhase::Count(at)) => {
                let e = now.duration_since(at);
                (e < COUNTDOWN).then(|| (COUNTDOWN - e).as_secs() + 1)
            }
            _ => None,
        }
    }

    pub fn leak_left(&self, now: Instant) -> Option<Duration> {
        match self.leak {
            LeakPhase::Active(at) => Some(LEAK_WINDOW.saturating_sub(now.duration_since(at))),
            _ => None,
        }
    }

    pub fn animating(&self) -> bool {
        matches!(self.drill, Drill::Rats | Drill::Leak) || self.burst.is_some()
    }

    fn hit(&mut self, at: (i32, i32), now: Instant) {
        self.hits += 1;
        self.fail_streak = 0;
        self.flash = Some((true, now));
        self.burst = Some((at, now));
    }

    fn miss(&mut self, msg: &str, now: Instant) -> RoundEvent {
        self.total_mistakes += 1;
        self.fail_streak += 1;
        self.flash = Some((false, now));
        self.feedback = Some((msg.to_string(), now));
        if let Some(b) = &mut self.boss {
            if b.hurt_me(3) {
                return RoundEvent::Failed;
            }
        }
        RoundEvent::Mistake
    }

    fn advance(&mut self, total: usize, now: Instant) -> RoundEvent {
        self.idx += 1;
        if self.idx >= total {
            self.finished_at = Some(now);
            RoundEvent::Finished
        } else {
            RoundEvent::Hit
        }
    }

    /// 키 이벤트 — 새는 글자 0 에서는 이것이 곧 "샌 글자"다
    pub fn on_key(&mut self, key: &KeyEvent, now: Instant) -> RoundEvent {
        use crossterm::event::KeyCode;
        if self.drill != Drill::Leak {
            return RoundEvent::None;
        }
        match self.leak {
            LeakPhase::Ready if key.code == KeyCode::Enter => {
                self.leak = LeakPhase::Count(now);
                RoundEvent::None
            }
            LeakPhase::Active(_) => {
                self.leaks += 1;
                self.flash = Some((false, now));
                self.feedback = Some((format!("샜다! {:?}", key.code), now));
                RoundEvent::Mistake
            }
            _ => RoundEvent::None,
        }
    }

    /// 마우스 이벤트 — `x`, `y` 는 과녁장 안쪽 좌표
    pub fn on_mouse(&mut self, kind: MouseEventKind, x: i32, y: i32, now: Instant) -> RoundEvent {
        let inside = x >= 0 && y >= 0 && x < i32::from(ARENA_W) && y < i32::from(ARENA_H);
        if matches!(kind, MouseEventKind::Moved | MouseEventKind::Drag(_)) {
            self.moved_seen = true;
        }
        self.pointer = inside.then_some((x, y));
        if let LeakPhase::Active(_) = self.leak {
            self.activity += 1;
            return RoundEvent::None;
        }
        if self.finished_at.is_some() {
            return RoundEvent::None;
        }
        match self.drill {
            Drill::Move => self.on_move(kind, x, y, now),
            Drill::Aim => self.on_aim(kind, x, y, now),
            Drill::Wheel => self.on_wheel(kind, y, now),
            Drill::Drag => self.on_drag(kind, x, y, now),
            Drill::Rats => self.on_rats(kind, x, y, now),
            Drill::Leak => RoundEvent::None,
        }
    }

    fn on_move(&mut self, kind: MouseEventKind, x: i32, y: i32, now: Instant) -> RoundEvent {
        let Some(t) = self.target() else {
            return RoundEvent::None;
        };
        let on = t.rect.contains(x, y);
        match kind {
            MouseEventKind::Moved | MouseEventKind::Drag(_) if on => {
                self.hit((x, y), now);
                self.advance(self.targets.len(), now)
            }
            MouseEventKind::Down(_) if on => {
                self.hit((x, y), now);
                self.advance(self.targets.len(), now)
            }
            MouseEventKind::Down(_) => self.miss("빗나갔다 — 과녁 위에 올려라", now),
            _ => RoundEvent::None,
        }
    }

    fn on_aim(&mut self, kind: MouseEventKind, x: i32, y: i32, now: Instant) -> RoundEvent {
        let MouseEventKind::Down(btn) = kind else {
            return RoundEvent::None;
        };
        let Some(t) = self.target().cloned() else {
            return RoundEvent::None;
        };
        if !t.rect.contains(x, y) {
            return self.miss("빗나갔다", now);
        }
        if t.button.is_some_and(|b| b != btn) {
            let want = if t.button == Some(MouseButton::Right) {
                "우클릭"
            } else {
                "좌클릭"
            };
            return self.miss(&format!("버튼이 다르다 — {want}이다"), now);
        }
        self.hit((x, y), now);
        self.advance(self.targets.len(), now)
    }

    fn on_wheel(&mut self, kind: MouseEventKind, y: i32, now: Instant) -> RoundEvent {
        let max = self.list.len().saturating_sub(LIST_VIEW);
        match kind {
            MouseEventKind::ScrollDown => self.offset = (self.offset + 1).min(max),
            MouseEventKind::ScrollUp => self.offset = self.offset.saturating_sub(1),
            MouseEventKind::Down(MouseButton::Left) => {
                // 목록은 과녁장 둘째 줄부터
                let row = y - 1;
                if !(0..LIST_VIEW as i32).contains(&row) {
                    return RoundEvent::None;
                }
                let i = self.offset + row as usize;
                if Some(&i) == self.wanted.get(self.idx) {
                    self.hit((2, y), now);
                    return self.advance(self.wanted.len(), now);
                }
                return self.miss("그 줄이 아니다", now);
            }
            _ => {}
        }
        RoundEvent::None
    }

    fn on_drag(&mut self, kind: MouseEventKind, x: i32, y: i32, now: Instant) -> RoundEvent {
        match kind {
            MouseEventKind::Down(MouseButton::Left) if self.icon.contains(x, y) => {
                self.dragging = Some((x - i32::from(self.icon.x), y - i32::from(self.icon.y)));
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some((dx, dy)) = self.dragging {
                    self.icon.x = (x - dx).clamp(0, i32::from(ARENA_W - self.icon.w)) as u16;
                    self.icon.y = (y - dy).clamp(0, i32::from(ARENA_H - self.icon.h)) as u16;
                }
            }
            MouseEventKind::Up(MouseButton::Left) if self.dragging.is_some() => {
                self.dragging = None;
                let cx = i32::from(self.icon.x + self.icon.w / 2);
                let cy = i32::from(self.icon.y + self.icon.h / 2);
                if self.folder.contains(cx, cy) {
                    self.hit((cx, cy), now);
                    let ev = self.advance(5, now);
                    if ev == RoundEvent::Hit {
                        self.place_drag();
                    }
                    return ev;
                }
                self.icon.x = self.icon_home.0;
                self.icon.y = self.icon_home.1;
                return self.miss("폴더 밖에 떨어뜨렸다 — 제자리로", now);
            }
            _ => {}
        }
        RoundEvent::None
    }

    fn on_rats(&mut self, kind: MouseEventKind, x: i32, y: i32, now: Instant) -> RoundEvent {
        let MouseEventKind::Down(_) = kind else {
            return RoundEvent::None;
        };
        let Some(clock) = self.clock(now) else {
            return RoundEvent::None;
        };
        let caught = self.rats.iter().position(|r| {
            if r.state != RatState::Running || clock < r.born {
                return false;
            }
            let rx = r.x(clock);
            i32::from(r.y) == y && (x as f32) >= rx - 1.0 && (x as f32) <= rx + f32::from(RAT_W)
        });
        let Some(i) = caught else {
            return self.miss("헛손질 — 쥐는 저쪽이다", now);
        };
        self.rats[i].state = RatState::Caught(now);
        self.hit((x, y), now);
        if let Some(b) = &mut self.boss {
            b.hit((100 + RATS_TO_WIN - 1) / RATS_TO_WIN);
        }
        self.rats_end()
    }

    fn rats_end(&mut self) -> RoundEvent {
        if self.rats.iter().any(|r| r.state == RatState::Running) {
            return RoundEvent::Hit;
        }
        self.finished_at = Some(Instant::now());
        match &self.boss {
            Some(b) if b.hp > 0 => RoundEvent::Failed,
            _ => RoundEvent::Finished,
        }
    }

    pub fn tick(&mut self, now: Instant) -> RoundEvent {
        if self
            .burst
            .is_some_and(|(_, at)| now.duration_since(at) > Duration::from_millis(400))
        {
            self.burst = None;
        }
        match self.drill {
            Drill::Leak => match self.leak {
                LeakPhase::Count(at) if now.duration_since(at) >= COUNTDOWN => {
                    self.leak = LeakPhase::Active(now);
                    RoundEvent::None
                }
                LeakPhase::Active(at) if now.duration_since(at) >= LEAK_WINDOW => {
                    self.leak = LeakPhase::Done;
                    self.finished_at = Some(now);
                    if self.leaks == 0 && self.activity > 0 {
                        RoundEvent::Finished
                    } else {
                        RoundEvent::Failed
                    }
                }
                _ => RoundEvent::None,
            },
            Drill::Rats => {
                let Some(clock) = self.clock(now) else {
                    return RoundEvent::None;
                };
                let mut escaped = 0;
                for r in &mut self.rats {
                    if r.state != RatState::Running || clock < r.born {
                        continue;
                    }
                    let x = r.x(clock);
                    if x > f32::from(ARENA_W) || x < -f32::from(RAT_W) - 0.5 {
                        r.state = RatState::Escaped;
                        escaped += 1;
                    }
                }
                if escaped == 0 {
                    return RoundEvent::None;
                }
                self.feedback = Some(("쥐가 달아났다 — 기 -10".into(), now));
                for _ in 0..escaped {
                    self.total_mistakes += 1;
                    self.flash = Some((false, now));
                    if let Some(b) = &mut self.boss {
                        if b.hurt_me(10) {
                            return RoundEvent::Failed;
                        }
                    }
                }
                match self.rats_end() {
                    RoundEvent::Hit => RoundEvent::Mistake,
                    e => e,
                }
            }
            _ => RoundEvent::None,
        }
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        self.finished_at.unwrap_or(now).duration_since(self.started)
    }

    pub fn summary(&self, cleared: bool) -> Summary {
        let elapsed = self.elapsed(Instant::now());
        let mut stats = Vec::new();
        let (stars, score) = match self.drill {
            Drill::Leak => {
                stats.push(("새어 나온 글자".into(), self.leaks.to_string()));
                stats.push(("마우스 움직임".into(), format!("{}회", self.activity)));
                (if cleared { 3 } else { 0 }, if cleared { 1000 } else { 0 })
            }
            Drill::Rats => {
                let caught = self.progress().0;
                stats.push(("잡은 쥐".into(), format!("{caught} / {RATS}")));
                stats.push(("헛손질".into(), self.total_mistakes.to_string()));
                let ki = self.boss.as_ref().map_or(0, |b| b.ki);
                (
                    if cleared {
                        super::score::boss_stars(ki)
                    } else {
                        0
                    },
                    (1000 * caught / RATS) as u32,
                )
            }
            _ => {
                let par = self.par_time.as_secs_f32();
                let t = elapsed.as_secs_f32().max(0.1);
                let score =
                    ((1000.0 * (par / t).min(1.0)) as u32).saturating_sub(self.total_mistakes * 40);
                let stars = if !cleared {
                    0
                } else if self.total_mistakes == 0 && t <= par * 0.7 {
                    3
                } else if self.total_mistakes <= 2 && t <= par {
                    2
                } else {
                    1
                };
                stats.push(("걸린 시간".into(), format!("{t:.1}초")));
                stats.push(("기준 시간".into(), format!("{par:.0}초")));
                stats.push(("명중".into(), self.hits.to_string()));
                stats.push(("빗나감".into(), self.total_mistakes.to_string()));
                (stars, score)
            }
        };
        Summary {
            stars,
            score: if cleared { score } else { 0 },
            strokes: None,
            par: None,
            mistakes: self.total_mistakes,
            hints: 0,
            max_combo: None,
            rows: Vec::new(),
            stats,
            ki: self.boss.as_ref().map(|b| b.ki.max(0)),
        }
    }
}

/// 이동·조준 과녁 — 좌우를 오가며 멀리, 갈수록 작게
fn targets(rng: &mut Rng, drill: Drill, count: usize) -> Vec<Target> {
    let mut out = Vec::new();
    for i in 0..count {
        let (w, h) = match i * 3 / count.max(1) {
            0 => (10, 3),
            1 => (6, 3),
            _ => (3, 1),
        };
        let left = i % 2 == 0;
        let span = ARENA_W / 2 - w - 2;
        let x = if left {
            1 + rng.below(u64::from(span)) as u16
        } else {
            ARENA_W / 2 + 1 + rng.below(u64::from(span)) as u16
        };
        let y = rng.below(u64::from(ARENA_H - h)) as u16;
        let button = (drill == Drill::Aim).then(|| {
            if rng.below(3) == 0 {
                MouseButton::Right
            } else {
                MouseButton::Left
            }
        });
        out.push(Target {
            rect: Rect { x, y, w, h },
            button,
        });
    }
    out
}

fn rats(rng: &mut Rng) -> Vec<Rat> {
    let mut out = Vec::new();
    let mut t = 0u64;
    for i in 0..RATS {
        let phase = i * 3 / RATS;
        out.push(Rat {
            y: rng.below(u64::from(ARENA_H)) as u16,
            dir: if rng.below(2) == 0 { 1 } else { -1 },
            speed: 9.0 + phase as f32 * 4.0 + rng.below(3) as f32,
            born: Duration::from_millis(t),
            state: RatState::Running,
        });
        t += 1300 - phase as u64 * 250;
    }
    out
}

/// 휠 수색 목록 — 비급 이름 사이사이 잡동사니
fn scroll_list() -> Vec<String> {
    const SCROLLS: [&str; 12] = [
        "卷一  두 손의 법칙",
        "卷二  홈로우의 길",
        "卷四  레이어 불변식",
        "卷五  축지법",
        "卷七  완충지대",
        "卷九  CapsLock 승격",
        "卷十  화살표의 유혹",
        "卷十三  ESDF 의 이치",
        "卷十五  엄지의 드래그",
        "卷十六  휠 수색",
        "卷十九  찾지 말고 불러라",
        "卷二十二  미아귀",
    ];
    const JUNK: [&str; 8] = [
        "먼지 쌓인 장부",
        "찢어진 지도",
        "빈 두루마리",
        "젖은 부적",
        "낡은 붓",
        "깨진 벼루",
        "누군가의 일기",
        "외상 장부",
    ];
    let mut out = Vec::new();
    for i in 0..60 {
        if i % 5 == 2 {
            out.push(SCROLLS[(i / 5) % SCROLLS.len()].to_string());
        } else {
            out.push(format!("{} {}", JUNK[i % JUNK.len()], i + 1));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::dojo::content::content;

    fn round(id: &str) -> MouseRound {
        MouseRound::new(content().lesson(id).unwrap().2, Instant::now()).unwrap()
    }

    fn center(r: &Rect) -> (i32, i32) {
        (i32::from(r.x + r.w / 2), i32::from(r.y + r.h / 2))
    }

    #[test]
    fn 과녁은_과녁장_안에_있다() {
        for id in ["purple.1", "purple.2"] {
            let r = round(id);
            assert!(!r.targets.is_empty());
            for t in &r.targets {
                assert!(t.rect.x + t.rect.w <= ARENA_W, "{id}");
                assert!(t.rect.y + t.rect.h <= ARENA_H, "{id}");
            }
        }
    }

    #[test]
    fn 포인터를_과녁에_올리면_맞는다() {
        let mut r = round("purple.1");
        let now = Instant::now();
        let n = r.targets.len();
        let mut ev = RoundEvent::None;
        for _ in 0..n {
            let (x, y) = center(&r.target().unwrap().rect);
            ev = r.on_mouse(MouseEventKind::Moved, x, y, now);
        }
        assert_eq!(ev, RoundEvent::Finished);
        assert!(r.moved_seen);
        assert_eq!(r.summary(true).stars, 3);
    }

    #[test]
    fn 조준은_버튼까지_맞아야_한다() {
        let mut r = round("purple.2");
        let now = Instant::now();
        let t = r.target().unwrap().clone();
        let (x, y) = center(&t.rect);
        let wrong = if t.button == Some(MouseButton::Left) {
            MouseButton::Right
        } else {
            MouseButton::Left
        };
        assert_eq!(
            r.on_mouse(MouseEventKind::Down(wrong), x, y, now),
            RoundEvent::Mistake
        );
        assert_eq!(
            r.on_mouse(MouseEventKind::Down(t.button.unwrap()), x, y, now),
            RoundEvent::Hit
        );
    }

    #[test]
    fn 휠로_내려_목표를_클릭한다() {
        let mut r = round("purple.3");
        let now = Instant::now();
        let want = r.wanted[0];
        while r.offset + LIST_VIEW <= want {
            r.on_mouse(MouseEventKind::ScrollDown, 5, 5, now);
        }
        let row = (want - r.offset) as i32 + 1;
        assert_eq!(
            r.on_mouse(MouseEventKind::Down(MouseButton::Left), 5, row, now),
            RoundEvent::Hit
        );
    }

    #[test]
    fn 파일을_폴더에_끌어다_놓는다() {
        let mut r = round("purple.4");
        let now = Instant::now();
        let (ix, iy) = center(&r.icon);
        r.on_mouse(MouseEventKind::Down(MouseButton::Left), ix, iy, now);
        let (fx, fy) = center(&r.folder);
        r.on_mouse(MouseEventKind::Drag(MouseButton::Left), fx, fy, now);
        assert_eq!(
            r.on_mouse(MouseEventKind::Up(MouseButton::Left), fx, fy, now),
            RoundEvent::Hit
        );
        // 밖에 놓으면 제자리
        let (ix, iy) = center(&r.icon);
        let home = (r.icon.x, r.icon.y);
        r.on_mouse(MouseEventKind::Down(MouseButton::Left), ix, iy, now);
        r.on_mouse(MouseEventKind::Drag(MouseButton::Left), 1, 1, now);
        assert_eq!(
            r.on_mouse(MouseEventKind::Up(MouseButton::Left), 1, 1, now),
            RoundEvent::Mistake
        );
        assert_eq!((r.icon.x, r.icon.y), home);
    }

    #[test]
    fn 새는_글자_0_은_마우스_움직임이_있고_키가_없어야_통과다() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let now = Instant::now();
        let mut r = round("purple.5");
        r.on_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), now);
        r.tick(now + COUNTDOWN);
        r.on_mouse(MouseEventKind::Moved, 3, 3, now + COUNTDOWN);
        let ev = r.tick(now + COUNTDOWN + LEAK_WINDOW + Duration::from_millis(10));
        assert_eq!(ev, RoundEvent::Finished);

        let mut r = round("purple.5");
        r.on_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), now);
        r.tick(now + COUNTDOWN);
        r.on_key(
            &KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
            now + COUNTDOWN,
        );
        let ev = r.tick(now + COUNTDOWN + LEAK_WINDOW + Duration::from_millis(10));
        assert_eq!(ev, RoundEvent::Failed);
    }

    #[test]
    fn 쥐마왕은_쥐를_충분히_잡으면_쓰러진다() {
        let start = Instant::now();
        let mut r = round("purple.boss");
        let t0 = start + COUNTDOWN;
        let rats = r.rats.clone();
        let mut ev = RoundEvent::None;
        for rat in &rats {
            // 등장 0.8초 뒤 한가운데쯤에서 클릭
            let clock = rat.born + Duration::from_millis(800);
            let x = rat.x(clock) as i32 + 1;
            ev = r.on_mouse(
                MouseEventKind::Down(MouseButton::Left),
                x,
                i32::from(rat.y),
                t0 + clock,
            );
            assert_ne!(ev, RoundEvent::Mistake, "쥐 {rat:?} 를 놓쳤다");
        }
        assert_eq!(ev, RoundEvent::Finished);
    }
}
