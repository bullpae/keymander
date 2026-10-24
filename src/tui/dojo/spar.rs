//! 대련 — 반응 수련 (docs/10 §4-2, S8).
//!
//! 두 가지 꼴이 있다.
//! - **카드**: 방향 하나를 보이고 결과 키로 받아친다. 반응시간 막대·최근 기록.
//!   처음 몇 장은 누를 키를 보여 주고, 그다음부터는 숨긴다 — "보지 말고 느껴라".
//! - **화살비**: 네 갈래로 떨어지는 화살을 판정선에서 받아친다. 박자 판정
//!   (極·妙·可)과 콤보. 화살귀 보스전의 공격 단위다.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::action::Action;
use super::content::Lesson;
use super::play::{later_tag, BossState, Rng, RoundEvent, Summary};

pub const LANES: [Action; 4] = [Action::Left, Action::Down, Action::Up, Action::Right];
/// 카드 반응시간 막대의 기준
pub const CARD_REF: Duration = Duration::from_millis(1500);
/// 키 안내를 보여 주는 카드 수
const GUIDED: usize = 6;
/// 화살비 시작 전 3·2·1
pub const COUNTDOWN: Duration = Duration::from_secs(3);

/// 박자 판정
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Judge {
    Perfect,
    Great,
    Good,
    Miss,
}

impl Judge {
    fn of(delta: Duration) -> Option<Self> {
        match delta.as_millis() {
            0..=90 => Some(Judge::Perfect),
            91..=180 => Some(Judge::Great),
            181..=300 => Some(Judge::Good),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Judge::Perfect => "極 PERFECT",
            Judge::Great => "妙 GREAT",
            Judge::Good => "可 GOOD",
            Judge::Miss => "失 MISS",
        }
    }

    fn points(self) -> u32 {
        match self {
            Judge::Perfect => 3,
            Judge::Great => 2,
            Judge::Good => 1,
            Judge::Miss => 0,
        }
    }
}

/// 떨어지는 화살 하나
#[derive(Debug, Clone)]
pub struct Note {
    pub lane: Action,
    /// 판정선에 닿는 시각 (카운트다운 끝 기준)
    pub at: Duration,
    pub judged: Option<Judge>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Card,
    Rain,
}

/// 최근 기록 한 칸
#[derive(Debug, Clone)]
pub struct Recent {
    pub ok: bool,
    pub got: Option<Action>,
    pub time: Option<Duration>,
}

pub struct Spar {
    pub lesson_id: String,
    pub style: Style,
    // ── 카드
    pub prompts: Vec<Action>,
    /// 카드에 나오는 수 묶음 — 이 밖의 수는 판정하지 않는다
    pub pool: Vec<Action>,
    pub idx: usize,
    pub prompt_at: Instant,
    pub times: Vec<Duration>,
    pub recent: VecDeque<Recent>,
    /// 이 방향을 틀렸으면 다시 키를 보여 준다
    pub reveal: bool,
    // ── 화살비
    pub notes: Vec<Note>,
    pub fall: Duration,
    pub last_judge: Option<(Judge, Instant)>,
    // ── 공통
    pub combo: u32,
    pub max_combo: u32,
    pub hits: u32,
    pub total_mistakes: u32,
    pub fail_streak: u32,
    pub boss: Option<BossState>,
    pub started: Instant,
    pub flash: Option<(bool, Instant)>,
}

impl Spar {
    pub fn new(lesson: &Lesson, now: Instant) -> Self {
        let style = match lesson.style.as_deref() {
            Some("rain") => Style::Rain,
            _ => Style::Card,
        };
        let count = lesson.count.unwrap_or(16) as usize;
        let mut rng = Rng::seeded(&lesson.id);
        // 카드는 과가 정한 수 묶음, 화살비는 늘 네 갈래
        let mut pool: Vec<Action> = match style {
            Style::Card => lesson
                .moves
                .iter()
                .filter_map(|m| Action::from_id(m))
                .collect(),
            Style::Rain => Vec::new(),
        };
        if pool.is_empty() {
            pool = LANES.to_vec();
        }
        let mut prompts = Vec::new();
        let mut notes = Vec::new();
        match style {
            Style::Card => {
                // 같은 방향이 세 번 연달아 나오지 않게
                while prompts.len() < count {
                    let a = pool[rng.below(pool.len() as u64) as usize];
                    let n = prompts.len();
                    if pool.len() > 1 && n >= 2 && prompts[n - 1] == a && prompts[n - 2] == a {
                        continue;
                    }
                    prompts.push(a);
                }
            }
            Style::Rain => {
                notes = chart(&mut rng, count, lesson.speed_ms.unwrap_or(600));
            }
        }
        let fall = Duration::from_millis(u64::from(lesson.fall_ms.unwrap_or(2000)));
        Self {
            lesson_id: lesson.id.clone(),
            style,
            prompts,
            pool,
            idx: 0,
            prompt_at: now,
            times: Vec::new(),
            recent: VecDeque::new(),
            reveal: false,
            notes,
            fall,
            last_judge: None,
            combo: 0,
            max_combo: 0,
            hits: 0,
            total_mistakes: 0,
            fail_streak: 0,
            boss: lesson.is_boss().then(|| BossState::new(None)),
            started: now,
            flash: None,
        }
    }

    /// 카드: 지금 방향
    pub fn prompt(&self) -> Option<Action> {
        self.prompts.get(self.idx).copied()
    }

    /// 카드: 키 안내를 보일까
    pub fn guided(&self) -> bool {
        self.idx < GUIDED || self.reveal
    }

    /// 화살비: 판정 시계 (카운트다운이 끝나기 전이면 None)
    pub fn clock(&self, now: Instant) -> Option<Duration> {
        now.duration_since(self.started).checked_sub(COUNTDOWN)
    }

    pub fn countdown_left(&self, now: Instant) -> Option<u64> {
        let e = now.duration_since(self.started);
        (self.style == Style::Rain && e < COUNTDOWN).then(|| (COUNTDOWN - e).as_secs() + 1)
    }

    pub fn animating(&self) -> bool {
        self.style == Style::Rain
    }

    fn good(&mut self) {
        self.combo += 1;
        self.max_combo = self.max_combo.max(self.combo);
        self.hits += 1;
        self.fail_streak = 0;
    }

    fn bad(&mut self, now: Instant) {
        self.combo = 0;
        self.total_mistakes += 1;
        self.fail_streak += 1;
        self.flash = Some((false, now));
    }

    pub fn on_action(&mut self, a: Action, now: Instant) -> RoundEvent {
        match self.style {
            Style::Card => self.card(a, now),
            Style::Rain => self.rain(a, now),
        }
    }

    fn card(&mut self, a: Action, now: Instant) -> RoundEvent {
        let Some(want) = self.prompt() else {
            return RoundEvent::None;
        };
        if !self.pool.contains(&a) {
            return RoundEvent::None;
        }
        let t = now.duration_since(self.prompt_at);
        if self.recent.len() >= 5 {
            self.recent.pop_front();
        }
        if a != want {
            self.recent.push_back(Recent {
                ok: false,
                got: Some(a),
                time: None,
            });
            self.reveal = true;
            self.bad(now);
            return RoundEvent::Mistake;
        }
        self.recent.push_back(Recent {
            ok: true,
            got: None,
            time: Some(t),
        });
        self.times.push(t);
        self.good();
        self.reveal = false;
        self.flash = Some((true, now));
        self.idx += 1;
        self.prompt_at = now;
        if self.idx >= self.prompts.len() {
            RoundEvent::Finished
        } else {
            RoundEvent::Hit
        }
    }

    fn rain(&mut self, a: Action, now: Instant) -> RoundEvent {
        let Some(clock) = self.clock(now) else {
            return RoundEvent::None;
        };
        if !LANES.contains(&a) {
            return RoundEvent::None;
        }
        // 그 갈래에서 판정 창 안의 가장 이른 화살
        let hit = self
            .notes
            .iter()
            .position(|n| n.judged.is_none() && n.lane == a && delta(n.at, clock) <= 300);
        let Some(i) = hit else {
            // 헛손질 — 콤보만 끊긴다
            self.bad(now);
            if let Some(b) = &mut self.boss {
                if b.hurt_me(3) {
                    return RoundEvent::Failed;
                }
            }
            return RoundEvent::Mistake;
        };
        let j =
            Judge::of(Duration::from_millis(delta(self.notes[i].at, clock))).unwrap_or(Judge::Good);
        self.notes[i].judged = Some(j);
        self.last_judge = Some((j, now));
        self.good();
        let n = self.notes.len() as f32;
        if let Some(b) = &mut self.boss {
            let mult = match j {
                Judge::Perfect => 1.3,
                Judge::Great => 1.15,
                _ => 1.0,
            };
            b.hit((100.0 / n * mult).ceil() as i32);
        }
        self.end_check()
    }

    fn end_check(&self) -> RoundEvent {
        if self.notes.iter().all(|n| n.judged.is_some()) {
            return match &self.boss {
                Some(b) if b.hp > 0 => RoundEvent::Failed,
                _ => RoundEvent::Finished,
            };
        }
        RoundEvent::Hit
    }

    /// 일시정지한 만큼 시계를 민다 — 화살비 악보도 시작 기준이라 함께 밀린다
    pub fn shift(&mut self, d: Duration) {
        self.started += d;
        self.prompt_at += d;
        later_tag(&mut self.last_judge, d);
        later_tag(&mut self.flash, d);
        if let Some(b) = &mut self.boss {
            b.shift(d);
        }
    }

    pub fn tick(&mut self, now: Instant) -> RoundEvent {
        if self.style != Style::Rain {
            return RoundEvent::None;
        }
        let Some(clock) = self.clock(now) else {
            return RoundEvent::None;
        };
        let mut missed = 0;
        for n in &mut self.notes {
            if n.judged.is_none() && clock > n.at + Duration::from_millis(300) {
                n.judged = Some(Judge::Miss);
                missed += 1;
            }
        }
        if missed == 0 {
            return RoundEvent::None;
        }
        self.last_judge = Some((Judge::Miss, now));
        for _ in 0..missed {
            self.bad(now);
            if let Some(b) = &mut self.boss {
                if b.hurt_me(10) {
                    return RoundEvent::Failed;
                }
            }
        }
        match self.end_check() {
            RoundEvent::Hit => RoundEvent::Mistake,
            e => e,
        }
    }

    pub fn accuracy(&self) -> f32 {
        let total = self.hits + self.total_mistakes;
        if total == 0 {
            return 0.0;
        }
        self.hits as f32 / total as f32
    }

    pub fn avg_time(&self) -> Option<Duration> {
        (!self.times.is_empty())
            .then(|| self.times.iter().sum::<Duration>() / self.times.len() as u32)
    }

    pub fn judged(&self, j: Judge) -> usize {
        self.notes.iter().filter(|n| n.judged == Some(j)).count()
    }

    pub fn summary(&self, cleared: bool) -> Summary {
        let mut stats = Vec::new();
        let (stars, score) = match self.style {
            Style::Card => {
                let acc = self.accuracy();
                let avg = self.avg_time().unwrap_or(CARD_REF).as_secs_f32();
                let speed = ((1.5 - avg) / 1.0).clamp(0.0, 1.0);
                let score = (acc * 600.0 + speed * 400.0) as u32;
                let stars = if !cleared {
                    0
                } else if acc >= 0.99 && avg <= 0.8 {
                    3
                } else if acc >= 0.9 && avg <= 1.2 {
                    2
                } else {
                    1
                };
                stats.push(("평균 반응".into(), format!("{avg:.2}초")));
                if let Some(best) = self.times.iter().min() {
                    stats.push(("최고 반응".into(), format!("{:.2}초", best.as_secs_f32())));
                }
                stats.push(("정확도".into(), format!("{:.0}%", acc * 100.0)));
                stats.push(("최고 콤보".into(), format!("×{}", self.max_combo)));
                (stars, score)
            }
            Style::Rain => {
                let n = self.notes.len().max(1) as u32;
                let pts: u32 = self
                    .notes
                    .iter()
                    .filter_map(|x| x.judged)
                    .map(Judge::points)
                    .sum();
                let score = 1000 * pts / (3 * n);
                let stars = match (&self.boss, cleared) {
                    (_, false) => 0,
                    (Some(b), true) => super::score::boss_stars(b.ki),
                    (None, true) if score >= 900 => 3,
                    (None, true) if score >= 700 => 2,
                    _ => 1,
                };
                for j in [Judge::Perfect, Judge::Great, Judge::Good, Judge::Miss] {
                    stats.push((j.label().into(), self.judged(j).to_string()));
                }
                stats.push(("최고 콤보".into(), format!("×{}", self.max_combo)));
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
            max_combo: Some(self.max_combo),
            rows: Vec::new(),
            stats,
            ki: self.boss.as_ref().map(|b| b.ki.max(0)),
        }
    }
}

fn delta(at: Duration, clock: Duration) -> u64 {
    if at > clock {
        (at - clock).as_millis() as u64
    } else {
        (clock - at).as_millis() as u64
    }
}

/// 악보 — 세 마디로 나눠 갈수록 빨라진다. 같은 갈래 연타는 두 번까지.
fn chart(rng: &mut Rng, count: usize, beat_ms: u32) -> Vec<Note> {
    let mut notes: Vec<Note> = Vec::with_capacity(count);
    let mut t = 600u64;
    for i in 0..count {
        let phase = i * 3 / count.max(1);
        let beat = u64::from(beat_ms) * (10 - phase as u64) / 10;
        let lane = loop {
            let a = LANES[rng.below(4) as usize];
            let n = notes.len();
            if n >= 2 && notes[n - 1].lane == a && notes[n - 2].lane == a {
                continue;
            }
            break a;
        };
        notes.push(Note {
            lane,
            at: Duration::from_millis(t),
            judged: None,
        });
        // 가끔 한 박 쉰다 — 숨 돌릴 틈
        t += if rng.below(6) == 0 { beat * 2 } else { beat };
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::dojo::content::content;

    fn lesson(id: &str) -> &'static Lesson {
        content().lesson(id).unwrap().2
    }

    #[test]
    fn 카드는_맞은_방향만_넘어가고_틀리면_키를_다시_보인다() {
        let now = Instant::now();
        let mut s = Spar::new(lesson("yellow.1"), now);
        let want = s.prompt().unwrap();
        let wrong = LANES.iter().copied().find(|a| *a != want).unwrap();
        assert_eq!(s.on_action(wrong, now), RoundEvent::Mistake);
        assert_eq!(s.idx, 0);
        assert!(s.reveal);
        assert_eq!(s.on_action(want, now), RoundEvent::Hit);
        assert_eq!(s.idx, 1);
        assert_eq!(s.combo, 1);
    }

    #[test]
    fn 카드를_빠르고_정확하게_끝내면_별_셋() {
        let mut now = Instant::now();
        let mut s = Spar::new(lesson("yellow.1"), now);
        let mut ev = RoundEvent::None;
        while let Some(a) = s.prompt() {
            now += Duration::from_millis(400);
            ev = s.on_action(a, now);
        }
        assert_eq!(ev, RoundEvent::Finished);
        assert_eq!(s.summary(true).stars, 3);
    }

    #[test]
    fn 화살비는_박자로_판정하고_놓치면_실수다() {
        let start = Instant::now();
        let mut s = Spar::new(lesson("yellow.3"), start);
        let first = s.notes[0].clone();
        let t0 = start + COUNTDOWN;
        // 정확한 박자
        let ev = s.on_action(first.lane, t0 + first.at);
        assert_eq!(ev, RoundEvent::Hit);
        assert_eq!(s.notes[0].judged, Some(Judge::Perfect));
        // 두 번째는 놓친다
        let second = s.notes[1].clone();
        let ev = s.tick(t0 + second.at + Duration::from_millis(400));
        assert_eq!(ev, RoundEvent::Mistake);
        assert_eq!(s.notes[1].judged, Some(Judge::Miss));
        assert_eq!(s.combo, 0);
    }

    #[test]
    fn 화살귀는_다_맞히면_쓰러진다() {
        let start = Instant::now();
        let mut s = Spar::new(lesson("yellow.boss"), start);
        let t0 = start + COUNTDOWN;
        let notes = s.notes.clone();
        let mut ev = RoundEvent::None;
        for n in &notes {
            ev = s.on_action(n.lane, t0 + n.at);
        }
        assert_eq!(ev, RoundEvent::Finished);
        assert_eq!(s.boss.as_ref().unwrap().hp, 0);
        assert_eq!(s.summary(true).stars, 3);
    }

    #[test]
    fn 화살귀를_반만_맞히면_살아남는다() {
        let start = Instant::now();
        let mut s = Spar::new(lesson("yellow.boss"), start);
        let t0 = start + COUNTDOWN;
        let notes = s.notes.clone();
        let mut ev = RoundEvent::None;
        for (i, n) in notes.iter().enumerate() {
            if i % 2 == 0 {
                ev = s.on_action(n.lane, t0 + n.at);
            } else {
                ev = s.tick(t0 + n.at + Duration::from_millis(350));
            }
            if ev == RoundEvent::Failed {
                break;
            }
        }
        if ev != RoundEvent::Failed {
            ev = s.tick(t0 + notes.last().unwrap().at + Duration::from_secs(1));
        }
        assert_eq!(ev, RoundEvent::Failed);
    }
}
