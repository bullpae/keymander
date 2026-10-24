//! 한영 리듬 — 혼합 문장을 따라 치며 전환 지점에서 한/영을 톡 (docs/10 §4-3 S11, 파란띠).
//!
//! 한/영 전환 자체는 터미널에 도착하지 않는다(OS IME 가 삼킨다). 그래서 결과로
//! 판정한다: 한글이 와야 할 자리에 영문이 오면 "전환 놓침", 그 반대도 같다.
//! 한글은 IME 가 조합을 마친 글자로 도착하므로 글자 단위로 비교하면 된다.
//!
//! `click` 묶음은 줄 사이에 마우스 과녁을 둔다. mouse 레이어(RAlt 쥠)로 클릭한 뒤
//! 다음 줄이 같은 언어로 이어지므로, 쥠이 한/영을 건드렸다면 바로 드러난다.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};

use super::content::{Content, Lesson};
use super::hangul::{is_hangul, to_hangul, to_keys};
use super::mouse::{Rect, ARENA_W};
use super::play::{BossState, Rng, RoundEvent, Summary};

/// 줄을 마친 뒤 다음 줄로 넘어가기까지
pub const LINE_PAUSE: Duration = Duration::from_millis(700);
/// 과녁이 놓이는 과녁장 안쪽 줄 범위
const CLICK_ROWS: (u16, u16) = (8, 11);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Type,
    /// 줄 사이 과녁 — 쥠으로 클릭
    Click,
    /// 한 줄 완료 — 잠깐 보여 주고 넘어간다
    LineDone(Instant),
}

/// 잘못 친 글자 하나의 꼴
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slip {
    /// 한/영 상태가 반대였다
    Mode,
    /// 그냥 오타
    Typo,
}

/// 친 글자 하나 — 맞았는지와 함께
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Typed {
    pub ch: char,
    pub slip: Option<Slip>,
}

pub struct ImeRound {
    pub lesson_id: String,
    pub lines: Vec<String>,
    pub idx: usize,
    pub typed: Vec<Typed>,
    pub phase: Phase,
    pub click: bool,
    pub target: Option<Rect>,
    /// 전환 놓침 — 잘못된 언어로 치기 시작한 횟수 (연달아 친 글자는 한 번)
    pub mode_misses: u32,
    pub typos: u32,
    pub chars: u32,
    pub combo: u32,
    pub max_combo: u32,
    pub total_mistakes: u32,
    pub fail_streak: u32,
    pub boss: Option<BossState>,
    pub par_time: Option<Duration>,
    pub started: Instant,
    pub finished_at: Option<Instant>,
    pub flash: Option<(bool, Instant)>,
    pub feedback: Option<(String, Instant)>,
    /// 지금 잘못된 언어로 치는 중인 글자들 (되짚기 피드백용)
    wrong_run: String,
    rng: Rng,
}

/// 비교에 쓰는 글자 갈래 — 영문자 · 한글 · 그 밖(숫자·기호·공백)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Script {
    Latin,
    Hangul,
    Other,
}

fn script(c: char) -> Script {
    if is_hangul(c) {
        Script::Hangul
    } else if c.is_ascii_alphabetic() {
        Script::Latin
    } else {
        Script::Other
    }
}

impl ImeRound {
    pub fn new(content: &Content, lesson: &Lesson, now: Instant) -> Option<Self> {
        let set = content.ime_set(lesson.set.as_deref()?)?;
        let boss = lesson.is_boss().then(|| {
            BossState::new(
                lesson
                    .time_limit_s
                    .map(|s| Duration::from_secs(u64::from(s))),
            )
        });
        Some(Self {
            lesson_id: lesson.id.clone(),
            lines: lesson.window(&set.lines),
            idx: 0,
            typed: Vec::new(),
            phase: Phase::Type,
            click: set.click,
            target: None,
            mode_misses: 0,
            typos: 0,
            chars: 0,
            combo: 0,
            max_combo: 0,
            total_mistakes: 0,
            fail_streak: 0,
            boss,
            par_time: lesson.par_time_s.map(|s| Duration::from_secs(u64::from(s))),
            started: now,
            finished_at: None,
            flash: None,
            feedback: None,
            wrong_run: String::new(),
            rng: Rng::seeded(&lesson.id),
        })
    }

    pub fn line(&self) -> &str {
        &self.lines[self.idx]
    }

    /// 다음에 쳐야 할 글자 — 틀린 글자가 남아 있으면 `None` (먼저 지워야 한다)
    pub fn expected(&self) -> Option<char> {
        if self.typed.iter().any(|t| t.slip.is_some()) {
            return None;
        }
        self.line().chars().nth(self.typed.len())
    }

    /// 한/영을 톡 해야 하는 글자 위치(글자 번호) — 앞 글자와 언어가 바뀌는 곳.
    /// 첫 줄은 영문 상태에서 시작한다고 보고, 이후 줄은 앞 줄 끝 상태를 잇는다.
    pub fn switch_points(&self, idx: usize) -> Vec<usize> {
        let mut state = self.lines[..idx]
            .iter()
            .flat_map(|l| l.chars())
            .map(script)
            .rfind(|s| *s != Script::Other)
            .unwrap_or(Script::Latin);
        let mut out = Vec::new();
        for (i, c) in self.lines[idx].chars().enumerate() {
            let s = script(c);
            if s != Script::Other && s != state {
                out.push(i);
                state = s;
            }
        }
        out
    }

    /// 다음 글자가 한글인가 영문인가 — 화면의 "지금 상태" 안내
    pub fn want_hangul(&self) -> Option<bool> {
        self.line()
            .chars()
            .skip(self.typed.len())
            .map(script)
            .find(|s| *s != Script::Other)
            .map(|s| s == Script::Hangul)
    }

    pub fn on_key(&mut self, key: &KeyEvent, now: Instant) -> RoundEvent {
        if self.phase != Phase::Type {
            return RoundEvent::None;
        }
        match key.code {
            KeyCode::Backspace => {
                if self.typed.pop().is_some() {
                    self.wrong_run.pop();
                }
                RoundEvent::None
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.on_char(c, now)
            }
            _ => RoundEvent::None,
        }
    }

    fn on_char(&mut self, c: char, now: Instant) -> RoundEvent {
        let want = self.expected();
        self.chars += 1;
        if want == Some(c) {
            self.typed.push(Typed { ch: c, slip: None });
            self.wrong_run.clear();
            self.combo += 1;
            self.max_combo = self.max_combo.max(self.combo);
            self.fail_streak = 0;
            if self.typed.len() == self.line().chars().count() {
                return self.line_done(now);
            }
            return RoundEvent::Hit;
        }
        // 이미 틀린 글자 뒤에 이어 친 글자는 새 실수로 세지 않는다
        let first_slip = want.is_some();
        let slip = match want.map(script) {
            Some(Script::Hangul) if script(c) == Script::Latin => Slip::Mode,
            Some(Script::Latin) if script(c) == Script::Hangul => Slip::Mode,
            _ if !first_slip && !self.wrong_run.is_empty() => Slip::Mode,
            _ => Slip::Typo,
        };
        self.typed.push(Typed {
            ch: c,
            slip: Some(slip),
        });
        self.combo = 0;
        if !first_slip {
            if slip == Slip::Mode {
                self.wrong_run.push(c);
                self.mode_feedback(now);
            }
            return RoundEvent::None;
        }
        self.total_mistakes += 1;
        self.fail_streak += 1;
        self.flash = Some((false, now));
        let hurt = match slip {
            Slip::Mode => {
                self.mode_misses += 1;
                self.wrong_run = c.to_string();
                self.mode_feedback(now);
                15
            }
            Slip::Typo => {
                self.typos += 1;
                self.wrong_run.clear();
                self.feedback = Some((format!("'{c}' — 오타다. 지우고 다시"), now));
                3
            }
        };
        if let Some(b) = &mut self.boss {
            if b.hurt_me(hurt) {
                return RoundEvent::Failed;
            }
        }
        RoundEvent::Mistake
    }

    /// 잘못된 언어로 친 글자를 되짚어 보여 준다 — `fh` 는 한글 상태였다면 `로`
    fn mode_feedback(&mut self, now: Instant) {
        let run = &self.wrong_run;
        let msg = if run.chars().any(is_hangul) {
            format!("한글 상태였다 — {run} → {} · 지우고 톡!", to_keys(run))
        } else {
            format!("영문 상태였다 — {run} → {} · 지우고 톡!", to_hangul(run))
        };
        self.feedback = Some((msg, now));
    }

    fn line_done(&mut self, now: Instant) -> RoundEvent {
        self.flash = Some((true, now));
        self.feedback = None;
        let n = self.lines.len() as i32;
        if let Some(b) = &mut self.boss {
            b.hit(100 / n + 1);
        }
        if self.idx + 1 >= self.lines.len() {
            self.finished_at = Some(now);
            if let Some(b) = &mut self.boss {
                b.hp = 0;
            }
            return RoundEvent::Finished;
        }
        if self.click {
            self.phase = Phase::Click;
            let w = 9u16;
            let x = 2 + self.rng.below(u64::from(ARENA_W - w - 4)) as u16;
            let y = CLICK_ROWS.0 + self.rng.below(u64::from(CLICK_ROWS.1 - CLICK_ROWS.0)) as u16;
            self.target = Some(Rect { x, y, w, h: 1 });
        } else {
            self.phase = Phase::LineDone(now);
        }
        RoundEvent::Cleared {
            over_par: false,
            sage_equal: false,
        }
    }

    fn next_line(&mut self) {
        self.idx += 1;
        self.typed.clear();
        self.wrong_run.clear();
        self.target = None;
        self.phase = Phase::Type;
    }

    /// 과녁장 안쪽 좌표의 마우스 이벤트 (줄 사이 과녁)
    pub fn on_mouse(&mut self, kind: MouseEventKind, x: i32, y: i32, now: Instant) -> RoundEvent {
        if self.phase != Phase::Click {
            return RoundEvent::None;
        }
        let MouseEventKind::Down(MouseButton::Left) = kind else {
            return RoundEvent::None;
        };
        if self.target.is_some_and(|t| t.contains(x, y)) {
            self.flash = Some((true, now));
            self.feedback = Some((
                "쥐었다 놓았다 — 한/영은 그대로인지 다음 줄이 말해 준다".into(),
                now,
            ));
            self.next_line();
            RoundEvent::Hit
        } else {
            self.feedback = Some(("빗나갔다 — 과녁 안을 눌러라".into(), now));
            RoundEvent::None
        }
    }

    pub fn tick(&mut self, now: Instant) -> RoundEvent {
        if let Some(b) = &self.boss {
            if b.time_limit
                .is_some_and(|t| now.duration_since(self.started) > t)
            {
                return RoundEvent::Failed;
            }
        }
        if let Phase::LineDone(at) = self.phase {
            if now.duration_since(at) >= LINE_PAUSE {
                self.next_line();
            }
        }
        RoundEvent::None
    }

    pub fn wants_mouse(&self) -> bool {
        self.click
    }

    pub fn animating(&self) -> bool {
        self.boss.is_some() || matches!(self.phase, Phase::LineDone(_))
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        self.finished_at.unwrap_or(now).duration_since(self.started)
    }

    /// 분당 타수 — 한글 한 글자도 한 타로 센다(음절)
    fn cpm(&self) -> u32 {
        let secs = self.elapsed(Instant::now()).as_secs_f32().max(1.0);
        let done: usize = self.lines[..self.idx]
            .iter()
            .map(|l| l.chars().count())
            .sum::<usize>()
            + self.typed.len();
        (done as f32 * 60.0 / secs) as u32
    }

    pub fn summary(&self, cleared: bool) -> Summary {
        let secs = self.elapsed(Instant::now());
        let slow = self.par_time.is_some_and(|p| secs > p);
        let score = 1000u32
            .saturating_sub(self.mode_misses * 120)
            .saturating_sub(self.typos * 25)
            .saturating_sub(if slow { 150 } else { 0 });
        let stars = match (&self.boss, cleared) {
            (_, false) => 0,
            (Some(b), true) => super::score::boss_stars(b.ki),
            (None, true) if self.mode_misses == 0 && self.typos <= 2 && !slow => 3,
            (None, true) if self.mode_misses <= 2 => 2,
            _ => 1,
        };
        let mut stats = vec![
            ("전환 놓침".into(), self.mode_misses.to_string()),
            ("오타".into(), self.typos.to_string()),
            ("타속".into(), format!("{}타/분", self.cpm())),
            ("걸린 시간".into(), format!("{:.0}초", secs.as_secs_f32())),
        ];
        if let Some(p) = self.par_time {
            stats.push(("기준 시간".into(), format!("{}초", p.as_secs())));
        }
        stats.push(("최고 콤보".into(), format!("×{}", self.max_combo)));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::dojo::content::content;

    fn round(id: &str) -> ImeRound {
        let c = content();
        let (_, _, l) = c.lesson(id).unwrap();
        ImeRound::new(c, l, Instant::now()).unwrap()
    }

    fn ch(r: &mut ImeRound, c: char) -> RoundEvent {
        r.on_key(
            &KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            Instant::now(),
        )
    }

    fn type_line(r: &mut ImeRound) -> RoundEvent {
        let line = r.line().to_string();
        let mut last = RoundEvent::None;
        for c in line.chars() {
            last = ch(r, c);
        }
        last
    }

    #[test]
    fn 바르게_치면_별_셋이다() {
        let mut r = round("blue.1");
        let far = Instant::now() + Duration::from_secs(1);
        loop {
            match type_line(&mut r) {
                RoundEvent::Finished => break,
                RoundEvent::Cleared { .. } => {
                    r.tick(far);
                }
                e => panic!("{e:?}"),
            }
        }
        let s = r.summary(true);
        assert_eq!((s.stars, s.score), (3, 1000));
    }

    #[test]
    fn 영문_상태로_한글을_치면_전환_놓침이고_되짚어_준다() {
        let mut r = round("blue.1");
        for c in "kmd ".chars() {
            ch(&mut r, c);
        }
        assert_eq!(r.want_hangul(), Some(true));
        assert_eq!(ch(&mut r, 'f'), RoundEvent::Mistake);
        ch(&mut r, 'h');
        assert_eq!(r.mode_misses, 1, "이어 친 글자는 한 번으로");
        let msg = &r.feedback.as_ref().unwrap().0;
        assert!(msg.contains("fh → 로"), "{msg}");
        // 지우기 전엔 맞는 글자를 쳐도 진행되지 않는다
        assert_eq!(r.expected(), None);
        let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        r.on_key(&bs, Instant::now());
        r.on_key(&bs, Instant::now());
        assert_eq!(r.expected(), Some('로'));
        assert_eq!(ch(&mut r, '로'), RoundEvent::Hit);
    }

    #[test]
    fn 한글_상태로_영문을_치면_자판으로_되짚는다() {
        let mut r = round("blue.1");
        ch(&mut r, 'ㅏ');
        assert_eq!(r.mode_misses, 1);
        assert!(r.feedback.as_ref().unwrap().0.contains("ㅏ → k"));
    }

    #[test]
    fn 톡_자리는_언어가_바뀌는_글자다() {
        let r = round("blue.1");
        // "kmd 로 PDF 를 찾는다." — 로(4) P(6) 를(10)
        assert_eq!(r.switch_points(0), vec![4, 6, 10]);
        // 둘째 줄 "CapsLock 을 …" 은 앞 줄이 한글로 끝났으니 C 에서 톡
        assert_eq!(r.switch_points(1)[0], 0);
    }

    #[test]
    fn 쥠_과녁을_맞혀야_다음_줄이다() {
        let mut r = round("blue.2");
        assert!(r.wants_mouse());
        assert!(matches!(type_line(&mut r), RoundEvent::Cleared { .. }));
        assert_eq!(r.phase, Phase::Click);
        let t = r.target.unwrap();
        let now = Instant::now();
        let down = MouseEventKind::Down(MouseButton::Left);
        assert_eq!(r.on_mouse(down, 0, 0, now), RoundEvent::None);
        assert_eq!(
            r.on_mouse(down, i32::from(t.x), i32::from(t.y), now),
            RoundEvent::Hit
        );
        assert_eq!((r.idx, r.phase), (1, Phase::Type));
    }

    #[test]
    fn 양설귀는_전환을_놓치면_기가_크게_준다() {
        let mut r = round("blue.boss");
        ch(&mut r, 'd');
        assert_eq!(r.boss.as_ref().unwrap().ki, 85);
    }
}
