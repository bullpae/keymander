//! 키 감응 — 도착한 키 이벤트를 수(Action)로 번역한다 (docs/10 §6-1, §6-2).
//!
//! 같은 수라도 OS·터미널마다 다른 이벤트로 도착한다. macOS 단어← 는 데몬이
//! `Alt+Left`를 보내지만 Terminal.app 에는 `ESC b`(= Alt+'b')로 오고, 줄 처음
//! (`Cmd+Left`)은 아예 오지 않기도 한다. 기본 별칭표로 흔한 경우를 덮고, 입문
//! 의식의 감응 시험이 이 터미널에서 실제로 도착한 키를 배워 덧붙인다.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::action::{Action, Os};

/// 감응 시험이 묻는 수 — 키 이벤트로 도착하는 것들
pub const KEY_ACTIONS: [Action; 12] = [
    Action::Left,
    Action::Down,
    Action::Up,
    Action::Right,
    Action::WordLeft,
    Action::WordRight,
    Action::LineStart,
    Action::LineEnd,
    Action::PageUp,
    Action::PageDown,
    Action::Backspace,
    Action::Delete,
];

/// 2탭 수의 두 번째 이벤트를 기다리는 시간
const SECOND_TAP_WAIT: Duration = Duration::from_millis(1200);
/// 한 수를 기다리는 시간 — 넘으면 관측 불가
const STEP_TIMEOUT: Duration = Duration::from_secs(8);

/// 이벤트의 안정적인 이름 — 보정 표의 키 (`alt+char(b)`, `left`)
pub fn key_id(key: &KeyEvent) -> String {
    let mut parts = Vec::new();
    let m = key.modifiers;
    if m.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl".to_string());
    }
    if m.contains(KeyModifiers::ALT) {
        parts.push("alt".to_string());
    }
    if m.contains(KeyModifiers::SUPER) {
        parts.push("super".to_string());
    }
    if m.contains(KeyModifiers::SHIFT) {
        parts.push("shift".to_string());
    }
    let code = match key.code {
        KeyCode::Char(c) => format!("char({})", c.to_ascii_lowercase()),
        other => format!("{other:?}").to_ascii_lowercase(),
    };
    parts.push(code);
    parts.join("+")
}

/// 기본 별칭표 — 보정 없이도 흔한 터미널은 통한다
pub fn default_action(key: &KeyEvent, os: Os) -> Option<Action> {
    let m = key.modifiers - KeyModifiers::SHIFT;
    let none = m.is_empty();
    let alt = m == KeyModifiers::ALT;
    let ctrl = m == KeyModifiers::CONTROL;
    let sup = m == KeyModifiers::SUPER;
    let mac = os == Os::Mac;
    Some(match key.code {
        KeyCode::Left if none => Action::Left,
        KeyCode::Right if none => Action::Right,
        KeyCode::Up if none => Action::Up,
        KeyCode::Down if none => Action::Down,
        KeyCode::Left if alt || ctrl => Action::WordLeft,
        KeyCode::Right if alt || ctrl => Action::WordRight,
        // Terminal.app · iTerm2 "Natural Text Editing" 은 Alt+화살표를 ESC b / ESC f 로 보낸다
        KeyCode::Char('b') if alt && mac => Action::WordLeft,
        KeyCode::Char('f') if alt && mac => Action::WordRight,
        KeyCode::Home if none => Action::LineStart,
        KeyCode::End if none => Action::LineEnd,
        KeyCode::Left if sup => Action::LineStart,
        KeyCode::Right if sup => Action::LineEnd,
        // iTerm2 Natural: Cmd+← = Ctrl+A, Cmd+→ = Ctrl+E
        KeyCode::Char('a') if ctrl && mac => Action::LineStart,
        KeyCode::Char('e') if ctrl && mac => Action::LineEnd,
        KeyCode::PageUp if none => Action::PageUp,
        KeyCode::PageDown if none => Action::PageDown,
        KeyCode::Backspace if none => Action::Backspace,
        KeyCode::Delete if none => Action::Delete,
        _ => return None,
    })
}

/// 이 터미널의 보정 결과
#[derive(Debug, Clone, Default)]
pub struct Calib {
    /// 기본 별칭표에 없는데 이 터미널에서 그 수로 도착한 키
    learned: BTreeMap<String, Action>,
    /// 이 터미널에서 도착하지 않는 수
    pub unobservable: BTreeSet<Action>,
    /// 감응 시험을 마쳤는가
    pub done: bool,
}

impl Calib {
    /// 시험 전 기본값 — 알려진 터미널 편차만 반영한다
    pub fn defaults_for(terminal: Option<&str>, os: Os) -> Self {
        let mut c = Self::default();
        if os == Os::Mac && terminal == Some("Apple_Terminal") {
            // Terminal.app 은 Cmd+←/→ 를 앱에 넘기지 않는다
            c.unobservable.insert(Action::LineStart);
            c.unobservable.insert(Action::LineEnd);
        }
        c
    }

    pub fn translate(&self, key: &KeyEvent, os: Os) -> Option<Action> {
        self.learned
            .get(&key_id(key))
            .copied()
            .or_else(|| default_action(key, os))
    }

    pub fn observable(&self, a: Action) -> bool {
        !self.unobservable.contains(&a)
    }

    pub fn to_json(&self) -> String {
        let learned: serde_json::Map<String, serde_json::Value> = self
            .learned
            .iter()
            .map(|(k, a)| (k.clone(), a.id().into()))
            .collect();
        let missing: Vec<&str> = self.unobservable.iter().map(|a| a.id()).collect();
        serde_json::json!({ "learned": learned, "missing": missing }).to_string()
    }

    pub fn from_json(text: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        let learned = v
            .get("learned")?
            .as_object()?
            .iter()
            .filter_map(|(k, a)| Some((k.clone(), Action::from_id(a.as_str()?)?)))
            .collect();
        let unobservable = v
            .get("missing")?
            .as_array()?
            .iter()
            .filter_map(|a| Action::from_id(a.as_str()?))
            .collect();
        Some(Self {
            learned,
            unobservable,
            done: true,
        })
    }
}

/// 감응 시험 한 수의 결과
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// 이 키로 도착했다
    Observed(String),
    /// 도착하지 않았다
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalibEvent {
    None,
    Recorded,
    /// 다른 수가 도착했다 — 다시 누르게 한다
    Wrong(Action),
    Done,
}

/// 감응 시험 진행 (S2) — 수를 하나씩 요청하고 실제 도착한 이벤트를 기록한다
pub struct CalibRun {
    pub steps: Vec<Action>,
    pub results: Vec<Step>,
    /// 2탭 수 → 그 키의 1탭 수 (줄 처음 → 단어←). 첫 탭은 1탭 수로 먼저 도착한다.
    doubles: BTreeMap<Action, Action>,
    pub since: Instant,
    first_tap: Option<Instant>,
    pub wrong: Option<(Action, Instant)>,
    os: Os,
}

impl CalibRun {
    pub fn new(steps: Vec<Action>, doubles: BTreeMap<Action, Action>, now: Instant) -> Self {
        Self {
            steps,
            results: Vec::new(),
            doubles,
            since: now,
            first_tap: None,
            wrong: None,
            os: Os::current(),
        }
    }

    pub fn current(&self) -> Option<Action> {
        self.steps.get(self.results.len()).copied()
    }

    pub fn is_double(&self, a: Action) -> bool {
        self.doubles.contains_key(&a)
    }

    /// 남은 대기 시간 (화면 막대용)
    pub fn remaining(&self, now: Instant) -> Duration {
        STEP_TIMEOUT.saturating_sub(now.duration_since(self.since))
    }

    fn advance(&mut self, step: Step, now: Instant) -> CalibEvent {
        self.results.push(step);
        self.since = now;
        self.first_tap = None;
        self.wrong = None;
        if self.current().is_none() {
            CalibEvent::Done
        } else {
            CalibEvent::Recorded
        }
    }

    /// Tab = 이 수는 관측 불가로 두고 넘어간다
    pub fn skip(&mut self, now: Instant) -> CalibEvent {
        self.advance(Step::Missing, now)
    }

    pub fn on_key(&mut self, key: &KeyEvent, now: Instant) -> CalibEvent {
        let Some(want) = self.current() else {
            return CalibEvent::Done;
        };
        let got = default_action(key, self.os);
        if let Some(&single) = self.doubles.get(&want) {
            if got == Some(single) && self.first_tap.is_none() {
                self.first_tap = Some(now);
                return CalibEvent::None;
            }
        }
        match got {
            Some(other) if other != want && self.steps.contains(&other) => {
                self.wrong = Some((other, now));
                self.first_tap = None;
                CalibEvent::Wrong(other)
            }
            _ => self.advance(Step::Observed(key_id(key)), now),
        }
    }

    pub fn tick(&mut self, now: Instant) -> CalibEvent {
        if self.current().is_none() {
            return CalibEvent::None;
        }
        let second_lost = self
            .first_tap
            .is_some_and(|t| now.duration_since(t) > SECOND_TAP_WAIT);
        if second_lost || now.duration_since(self.since) > STEP_TIMEOUT {
            return self.advance(Step::Missing, now);
        }
        CalibEvent::None
    }

    /// 시험 결과를 보정 표로
    pub fn finish(&self) -> Calib {
        let mut c = Calib {
            done: true,
            ..Calib::default()
        };
        for (a, step) in self.steps.iter().zip(&self.results) {
            match step {
                Step::Observed(id) => {
                    c.learned.insert(id.clone(), *a);
                }
                Step::Missing => {
                    c.unobservable.insert(*a);
                }
            }
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    #[test]
    fn 기본_별칭표는_흔한_도착_형태를_덮는다() {
        let mac = Os::Mac;
        assert_eq!(
            default_action(&k(KeyCode::Left, KeyModifiers::NONE), mac),
            Some(Action::Left)
        );
        assert_eq!(
            default_action(&k(KeyCode::Left, KeyModifiers::ALT), mac),
            Some(Action::WordLeft)
        );
        assert_eq!(
            default_action(&k(KeyCode::Char('b'), KeyModifiers::ALT), mac),
            Some(Action::WordLeft)
        );
        assert_eq!(
            default_action(&k(KeyCode::Char('b'), KeyModifiers::ALT), Os::Windows),
            None
        );
        assert_eq!(
            default_action(&k(KeyCode::Char('a'), KeyModifiers::CONTROL), mac),
            Some(Action::LineStart)
        );
        assert_eq!(
            default_action(&k(KeyCode::Right, KeyModifiers::CONTROL), Os::Linux),
            Some(Action::WordRight)
        );
        assert_eq!(
            default_action(&k(KeyCode::Char('j'), KeyModifiers::NONE), mac),
            None,
            "맨 글자는 수가 아니다"
        );
    }

    #[test]
    fn 감응_시험은_2탭_수의_첫_탭을_건너뛰고_둘째를_배운다() {
        let now = Instant::now();
        let doubles = BTreeMap::from([(Action::LineStart, Action::WordLeft)]);
        let mut run = CalibRun::new(vec![Action::WordLeft, Action::LineStart], doubles, now);
        // 단어← — Terminal.app 처럼 ESC b 로 도착
        assert_eq!(
            run.on_key(&k(KeyCode::Char('b'), KeyModifiers::ALT), now),
            CalibEvent::Recorded
        );
        // 줄 처음 — 첫 탭(단어←)은 기다리고, 둘째로 온 Ctrl+A 를 배운다
        assert_eq!(
            run.on_key(&k(KeyCode::Left, KeyModifiers::ALT), now),
            CalibEvent::None
        );
        assert_eq!(
            run.on_key(&k(KeyCode::Char('x'), KeyModifiers::CONTROL), now),
            CalibEvent::Done
        );
        let c = run.finish();
        assert_eq!(
            c.translate(&k(KeyCode::Char('x'), KeyModifiers::CONTROL), Os::Mac),
            Some(Action::LineStart)
        );
        assert!(c.unobservable.is_empty());
    }

    #[test]
    fn 둘째_탭이_오지_않으면_관측_불가다() {
        let now = Instant::now();
        let doubles = BTreeMap::from([(Action::LineStart, Action::WordLeft)]);
        let mut run = CalibRun::new(vec![Action::LineStart], doubles, now);
        run.on_key(&k(KeyCode::Left, KeyModifiers::ALT), now);
        assert_eq!(run.tick(now + Duration::from_millis(500)), CalibEvent::None);
        assert_eq!(run.tick(now + Duration::from_secs(2)), CalibEvent::Done);
        assert!(run.finish().unobservable.contains(&Action::LineStart));
    }

    #[test]
    fn 다른_수가_오면_다시_묻는다() {
        let now = Instant::now();
        let mut run = CalibRun::new(vec![Action::Left, Action::Down], BTreeMap::new(), now);
        assert_eq!(
            run.on_key(&k(KeyCode::Down, KeyModifiers::NONE), now),
            CalibEvent::Wrong(Action::Down)
        );
        assert_eq!(run.results.len(), 0);
    }

    #[test]
    fn 보정_표는_json_으로_왕복한다() {
        let mut c = Calib::defaults_for(Some("Apple_Terminal"), Os::Mac);
        c.learned.insert("ctrl+char(x)".into(), Action::LineStart);
        let back = Calib::from_json(&c.to_json()).unwrap();
        assert_eq!(back.learned, c.learned);
        assert_eq!(back.unobservable, c.unobservable);
        assert!(back.done);
    }
}
