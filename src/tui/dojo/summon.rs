//! 소환술 — 런처 골프 (docs/10 §4-4, §7-4).
//!
//! 판정은 kmd-core `SearchEngine`을 연습 목록 위에서 그대로 돌린다. 런처와 같은
//! 엔진이라야 도장에서 익힌 "몇 글자면 1순위"가 실제 런처에서도 통한다.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kmd_core::config::KindWeights;
use kmd_core::query_prefix::{prefix_of, QueryPrefix};
use kmd_core::{IndexItem, ItemKind, SearchEngine, Source};

use super::content::{Content, Lesson, Request};
use super::play::{BossState, OutcomeRow, RoundEvent, Summary};
use super::score::{self, HoleResult};

/// 결과 목록에 보이는 줄 수 — 사부의 수도 이 안에서만 ↓ 를 쓴다
pub const VISIBLE: usize = 6;

/// 연습 목록 위의 검색 엔진
pub struct Fixture {
    engine: SearchEngine,
}

impl Fixture {
    pub fn new(content: &Content) -> Self {
        let items = content
            .summon
            .items
            .iter()
            .map(|i| IndexItem {
                name: i.name.clone(),
                path: i.path.clone(),
                kind: match i.kind.as_str() {
                    "app" => ItemKind::App,
                    "dir" => ItemKind::Directory,
                    _ => ItemKind::File,
                },
                source: Source::Plugin,
                icon: String::new(),
                keywords: String::new(),
                icon_path: None,
            })
            .collect();
        Self {
            engine: SearchEngine::with_items(&KindWeights::default(), items),
        }
    }

    pub fn search(&mut self, query: &str) -> Vec<IndexItem> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        let (_, results) = self.engine.search(query, VISIBLE);
        results.into_iter().map(|r| r.item).collect()
    }

    fn rank_of(&mut self, query: &str, target: &str) -> Option<usize> {
        self.search(query).iter().position(|i| i.name == target)
    }
}

/// 사부의 수 — 가장 짧은 입력 경로
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sage {
    pub query: String,
    /// 1순위가 아니면 ↓ 횟수
    pub downs: usize,
}

impl Sage {
    /// 타수 = 글자 + ↓ + Enter
    pub fn strokes(&self) -> u32 {
        (self.query.chars().count() + self.downs + 1) as u32
    }

    pub fn display(&self) -> String {
        let mut s = self.query.clone();
        for _ in 0..self.downs {
            s.push_str(" ↓");
        }
        s.push_str(" ⏎");
        s
    }
}

/// 의뢰의 사부의 수를 구한다 (docs/10 §7-4).
///
/// 후보는 사람이 떠올리는 쿼리로 한정한다 — 단어 머리에서 시작하는 조각
/// (`fire`, `stu`, `09`), 단어 머리글자 조합(`vsc`), `.확장자`. 이름 한가운데를
/// 아무 데서나 자른 조각(`ir`, `0`)은 작은 연습 목록의 우연을 노리는 수라 가르칠
/// 가치가 없다. 후보 중 글자 + ↓ + Enter 합이 가장 작은 것을 고른다.
pub fn sage_for(fixture: &mut Fixture, req: &Request) -> Sage {
    let Some(target) = &req.target else {
        let best = req
            .accept
            .iter()
            .min_by_key(|a| a.chars().count())
            .cloned()
            .unwrap_or_default();
        return Sage {
            query: best,
            downs: 0,
        };
    };
    let mut best: Option<Sage> = None;
    for q in candidates(target) {
        if best
            .as_ref()
            .is_some_and(|b| b.strokes() <= (q.chars().count() + 1) as u32)
        {
            continue;
        }
        if let Some(rank) = fixture.rank_of(&q, target) {
            let cand = Sage {
                query: q,
                downs: rank,
            };
            if best.as_ref().is_none_or(|b| cand.strokes() < b.strokes()) {
                best = Some(cand);
            }
        }
    }
    best.unwrap_or_else(|| {
        let q = target.to_lowercase();
        let downs = fixture.rank_of(&q, target).unwrap_or(0);
        Sage { query: q, downs }
    })
}

/// 사람이 떠올리는 후보 쿼리 — 짧은 것부터, 중복 없이
fn candidates(target: &str) -> Vec<String> {
    let chars: Vec<char> = target.chars().collect();
    let is_sep = |c: char| matches!(c, ' ' | '_' | '-' | '.' | '/');
    // 단어 머리: 처음, 구분자 다음, camelCase 대문자, 글자↔숫자 경계
    let starts: Vec<usize> = (0..chars.len())
        .filter(|&i| {
            let c = chars[i];
            if is_sep(c) {
                return false;
            }
            if i == 0 {
                return true;
            }
            let p = chars[i - 1];
            is_sep(p)
                || (p.is_lowercase() && c.is_uppercase())
                || (p.is_ascii_digit() != c.is_ascii_digit())
        })
        .collect();

    let mut out = Vec::new();
    if let Some((_, ext)) = target.rsplit_once('.') {
        if !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
            out.push(format!(".{}", ext.to_lowercase()));
        }
    }
    // 단어 머리에서 시작하는 조각 (구분자에서 끊는다, 최대 8자)
    for &s in &starts {
        let mut q = String::new();
        for &c in chars[s..].iter().take(8) {
            if is_sep(c) {
                break;
            }
            q.extend(c.to_lowercase());
            out.push(q.clone());
        }
    }
    // 단어 머리글자 조합 (연속 2~4단어)
    let initials: Vec<char> = starts
        .iter()
        .filter(|&&i| chars[i].is_alphabetic())
        .map(|&i| chars[i].to_ascii_lowercase())
        .collect();
    for s in 0..initials.len() {
        for len in 2..=4.min(initials.len() - s) {
            out.push(initials[s..s + len].iter().collect());
        }
    }
    // 한 글자는 뺀다 — 연습 목록은 작아서 한 글자로도 1순위가 되곤 하지만, 항목이
    // 수만 개인 실제 인덱스에서는 통하지 않는 수다. (사용자가 한 글자로 맞히는 것은 막지 않는다.)
    out.retain(|q| q.chars().count() >= 2);
    out.sort_by_key(|q| q.chars().count());
    out.dedup();
    out
}

/// 의뢰 하나의 결과
#[derive(Debug, Clone)]
pub struct Cleared {
    pub hole: HoleResult,
}

/// 한 판의 진행 상태
pub struct Round {
    pub lesson_id: String,
    pub requests: Vec<Request>,
    pub sages: Vec<Sage>,
    pub pars: Vec<u32>,
    pub idx: usize,
    pub query: String,
    pub results: Vec<IndexItem>,
    pub selected: usize,
    /// 지금 의뢰에서 친 타수
    pub strokes: u32,
    mistakes: u32,
    pub total_mistakes: u32,
    pub hint_shown: bool,
    pub hints: u32,
    pub cleared: Vec<Cleared>,
    pub fail_streak: u32,
    /// 한글이 들어왔는데 사부의 수는 영문 — "영문으로" 배너
    pub hangul_warn: bool,
    pub started: Instant,
    pub boss: Option<BossState>,
    /// 마지막 판정 순간 (화면 플래시용)
    pub flash: Option<(bool, Instant)>,
}

impl Round {
    pub fn new(content: &Content, lesson: &Lesson, fixture: &mut Fixture) -> Option<Self> {
        let set = content.summon_set(lesson.set.as_deref()?)?;
        let requests = set.requests.clone();
        let sages: Vec<Sage> = requests.iter().map(|r| sage_for(fixture, r)).collect();
        let pars = sages
            .iter()
            .map(|s| score::par_from_sage(s.strokes()))
            .collect();
        let boss = lesson.is_boss().then(|| {
            BossState::new(Some(Duration::from_secs(u64::from(
                lesson.time_limit_s.unwrap_or(120),
            ))))
        });
        Some(Self {
            lesson_id: lesson.id.clone(),
            requests,
            sages,
            pars,
            idx: 0,
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            strokes: 0,
            mistakes: 0,
            total_mistakes: 0,
            hint_shown: false,
            hints: 0,
            cleared: Vec::new(),
            fail_streak: 0,
            hangul_warn: false,
            started: Instant::now(),
            boss,
            flash: None,
        })
    }

    pub fn request(&self) -> &Request {
        &self.requests[self.idx]
    }

    pub fn sage(&self) -> &Sage {
        &self.sages[self.idx]
    }

    pub fn par(&self) -> u32 {
        self.pars[self.idx]
    }

    pub fn remaining_time(&self, now: Instant) -> Option<Duration> {
        let limit = self.boss.as_ref()?.time_limit?;
        Some(limit.saturating_sub(now.duration_since(self.started)))
    }

    /// 검색창 문자열에 따른 결과. 접두 비급 입력은 연습 목록 대신 그 도구의 한 줄을 보여 준다.
    fn refresh(&mut self, fixture: &mut Fixture) {
        self.selected = 0;
        self.results = match prefix_of(&self.query) {
            QueryPrefix::Web => vec![tool_item("웹 검색", &self.query)],
            QueryPrefix::Shell => vec![tool_item("셸 실행", &self.query)],
            _ => fixture.search(&self.query),
        };
    }

    pub fn on_paste(&mut self, text: &str, fixture: &mut Fixture) {
        for c in text.chars().filter(|c| !c.is_control()) {
            self.push_char(c);
        }
        self.refresh(fixture);
    }

    fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.strokes += 1;
        if is_hangul(c) && self.sage().query.is_ascii() {
            self.hangul_warn = true;
        }
    }

    pub fn on_key(&mut self, key: KeyEvent, fixture: &mut Fixture) -> RoundEvent {
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match key.code {
            KeyCode::Char(c) if plain => {
                self.push_char(c);
                self.refresh(fixture);
            }
            KeyCode::Backspace => {
                if self.query.pop().is_some() {
                    self.strokes += 1;
                    if !self.query.chars().any(is_hangul) {
                        self.hangul_warn = false;
                    }
                    self.refresh(fixture);
                }
            }
            KeyCode::Down if !self.results.is_empty() => {
                self.strokes += 1;
                self.selected = (self.selected + 1).min(self.results.len() - 1);
            }
            KeyCode::Up if !self.results.is_empty() => {
                self.strokes += 1;
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Tab if !self.hint_shown => {
                self.hint_shown = true;
                self.hints += 1;
            }
            KeyCode::Enter => {
                self.strokes += 1;
                return self.judge(fixture);
            }
            _ => {}
        }
        RoundEvent::None
    }

    fn judge(&mut self, fixture: &mut Fixture) -> RoundEvent {
        let req = self.request().clone();
        let ok = match &req.target {
            Some(t) => self
                .results
                .get(self.selected)
                .is_some_and(|i| &i.name == t),
            None => {
                let q = normalize_query(&self.query);
                req.accept.iter().any(|a| normalize_query(a) == q)
            }
        };
        self.flash = Some((ok, Instant::now()));
        if !ok {
            self.mistakes += 1;
            self.total_mistakes += 1;
            self.fail_streak += 1;
            if let Some(b) = &mut self.boss {
                if b.hurt_me(15) {
                    return RoundEvent::Failed;
                }
            }
            return RoundEvent::Mistake;
        }
        self.fail_streak = 0;
        let sage = self.sage().strokes();
        let hole = HoleResult {
            strokes: self.strokes,
            par: self.par(),
            sage,
            hinted: self.hint_shown,
        };
        let over_par = hole.strokes > hole.par;
        let sage_equal = hole.strokes <= sage;
        self.cleared.push(Cleared { hole });
        let total = self.requests.len() as i32;
        if let Some(b) = &mut self.boss {
            b.hit(b.hp - (100 - 100 * self.cleared.len() as i32 / total));
            if over_par && b.hurt_me(10) {
                return RoundEvent::Failed;
            }
        }
        if self.idx + 1 >= self.requests.len() {
            return RoundEvent::Finished;
        }
        self.idx += 1;
        self.query.clear();
        self.strokes = 0;
        self.mistakes = 0;
        self.hint_shown = false;
        self.hangul_warn = false;
        self.refresh(fixture);
        RoundEvent::Cleared {
            over_par,
            sage_equal,
        }
    }

    /// 보스 제한 시간 확인
    pub fn tick(&mut self, now: Instant) -> RoundEvent {
        match self.remaining_time(now) {
            Some(left) if left.is_zero() => RoundEvent::Failed,
            _ => RoundEvent::None,
        }
    }

    pub fn holes(&self) -> Vec<HoleResult> {
        self.cleared.iter().map(|c| c.hole.clone()).collect()
    }

    pub fn summary(&self, cleared: bool) -> Summary {
        let rows = self
            .requests
            .iter()
            .enumerate()
            .map(|(i, r)| OutcomeRow {
                ask: r.ask.clone(),
                sage: self.sages[i].display(),
                sage_strokes: self.sages[i].strokes(),
                par: self.pars[i],
                strokes: self.cleared.get(i).map(|c| c.hole.strokes),
            })
            .collect();
        Summary::from_holes(
            &self.holes(),
            rows,
            cleared,
            self.total_mistakes,
            self.hints,
            self.boss.as_ref(),
        )
    }
}

fn tool_item(label: &str, query: &str) -> IndexItem {
    IndexItem {
        name: format!("{label} — {}", query.trim()),
        path: String::new(),
        kind: ItemKind::WebSearch,
        source: Source::Plugin,
        icon: String::new(),
        keywords: String::new(),
        icon_path: None,
    }
}

/// 접두 비급 비교용 — 앞뒤 공백 제거, 연속 공백 하나로, 소문자
fn normalize_query(q: &str) -> String {
    q.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn is_hangul(c: char) -> bool {
    matches!(c, '\u{AC00}'..='\u{D7A3}' | '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::dojo::content::content;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_str(round: &mut Round, fx: &mut Fixture, s: &str) {
        for c in s.chars() {
            round.on_key(key(KeyCode::Char(c)), fx);
        }
    }

    #[test]
    fn 모든_의뢰는_사부의_수로_풀린다() {
        let c = content();
        let mut fx = Fixture::new(c);
        for set in &c.summon.set {
            for req in &set.requests {
                let sage = sage_for(&mut fx, req);
                assert!(sage.strokes() >= 2, "{}: {sage:?}", req.ask);
                if let Some(t) = &req.target {
                    let results = fx.search(&sage.query);
                    assert_eq!(
                        results.get(sage.downs).map(|i| i.name.as_str()),
                        Some(t.as_str()),
                        "{}: 사부의 수 {:?} 가 목표를 가리키지 않음",
                        req.ask,
                        sage
                    );
                }
            }
        }
    }

    #[test]
    fn 사부의_수는_결정론적이다() {
        let c = content();
        let req = &c.summon_set("names").unwrap().requests[0];
        let a = sage_for(&mut Fixture::new(c), req);
        let b = sage_for(&mut Fixture::new(c), req);
        assert_eq!(a, b);
    }

    #[test]
    fn 접두_비급은_가장_짧은_허용_입력이_사부의_수다() {
        let c = content();
        let req = &c.summon_set("prefix").unwrap().requests[0];
        let sage = sage_for(&mut Fixture::new(c), req);
        assert_eq!(sage.query, "@gh keymander");
        assert_eq!(sage.strokes(), 14);
    }

    #[test]
    fn 사부의_수대로_치면_사부와_같은_수로_통과한다() {
        let c = content();
        let mut fx = Fixture::new(c);
        let (_, _, lesson) = c.lesson("white.1").unwrap();
        let mut round = Round::new(c, lesson, &mut fx).unwrap();
        let sage = round.sage().clone();
        type_str(&mut round, &mut fx, &sage.query);
        for _ in 0..sage.downs {
            round.on_key(key(KeyCode::Down), &mut fx);
        }
        let ev = round.on_key(key(KeyCode::Enter), &mut fx);
        assert_eq!(
            ev,
            RoundEvent::Cleared {
                over_par: false,
                sage_equal: true
            }
        );
        assert_eq!(round.idx, 1);
        assert_eq!(round.cleared[0].hole.strokes, sage.strokes());
    }

    #[test]
    fn 틀린_항목에서_enter_하면_실수이고_타수는_계속_센다() {
        let c = content();
        let mut fx = Fixture::new(c);
        let (_, _, lesson) = c.lesson("white.1").unwrap();
        let mut round = Round::new(c, lesson, &mut fx).unwrap();
        // 첫 의뢰는 Firefox — "zoom" 은 다른 항목이다
        type_str(&mut round, &mut fx, "zoom");
        assert_eq!(
            round.on_key(key(KeyCode::Enter), &mut fx),
            RoundEvent::Mistake
        );
        assert_eq!(round.strokes, 5);
        assert_eq!(round.idx, 0);
    }

    #[test]
    fn 접두_비급은_공백과_대소문자를_가리지_않는다() {
        let c = content();
        let mut fx = Fixture::new(c);
        let (_, _, lesson) = c.lesson("white.4").unwrap();
        let mut round = Round::new(c, lesson, &mut fx).unwrap();
        type_str(&mut round, &mut fx, "@GitHub  keymander ");
        assert!(matches!(
            round.on_key(key(KeyCode::Enter), &mut fx),
            RoundEvent::Cleared { .. }
        ));
    }

    #[test]
    fn 영문_의뢰에_한글이_들어오면_경고하고_지우면_꺼진다() {
        let c = content();
        let mut fx = Fixture::new(c);
        let (_, _, lesson) = c.lesson("white.1").unwrap();
        let mut round = Round::new(c, lesson, &mut fx).unwrap();
        round.on_key(key(KeyCode::Char('ㄹ')), &mut fx);
        assert!(round.hangul_warn);
        round.on_key(key(KeyCode::Backspace), &mut fx);
        assert!(!round.hangul_warn);
    }

    #[test]
    fn 보스는_실수로_기가_줄고_다_떨어지면_진다() {
        let c = content();
        let mut fx = Fixture::new(c);
        let (_, _, boss) = c.lesson("white.boss").unwrap();
        let mut round = Round::new(c, boss, &mut fx).unwrap();
        type_str(&mut round, &mut fx, "zzzz");
        let mut last = RoundEvent::None;
        for _ in 0..7 {
            last = round.on_key(key(KeyCode::Enter), &mut fx);
            if last == RoundEvent::Failed {
                break;
            }
        }
        assert_eq!(last, RoundEvent::Failed);
        assert!(round.boss.as_ref().unwrap().ki <= 0);
    }
}

/// 콘텐츠 작성용 진단 — 의뢰별 사부의 수를 출력한다.
/// `cargo test --bin kmd dump_sages -- --ignored --nocapture`
#[cfg(test)]
mod dump {
    #[test]
    #[ignore]
    fn dump_sages() {
        let c = crate::tui::dojo::content::content();
        let mut fx = super::Fixture::new(c);
        for set in &c.summon.set {
            for r in &set.requests {
                let s = super::sage_for(&mut fx, r);
                println!(
                    "{:8} {:40} {:20} {}",
                    set.id,
                    r.target.clone().unwrap_or_default(),
                    s.display(),
                    s.strokes()
                );
            }
        }
    }
}
