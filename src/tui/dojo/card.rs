//! 결과 카드 (S17) — 공유용 한 장. 도장 안(C)과 `kmd dojo card` 가 같은 줄을 쓴다.
//!
//! 터미널 그대로 스크린샷을 찍거나 텍스트로 붙여 넣기 좋게 48칸 안에 담는다.

use kmd_core::dojo::{DojoProgress, DojoSeal, DojoStreak};
use std::collections::BTreeMap;

use super::action::Action;
use super::content::Content;
use super::progress::Standing;

/// 카드 안쪽 폭
pub const WIDTH: usize = 46;

/// 카드 한 줄의 꼴 — 색은 화면이 정한다
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Title,
    Text,
    Sub,
    Gold,
    Bar,
}

/// 카드의 재료 — 앱에서도, 앱 없이 DB 만으로도(`kmd dojo card`) 만든다
pub struct CardData<'a> {
    pub name: String,
    pub content: &'a Content,
    pub standing: &'a Standing,
    pub progress: &'a BTreeMap<String, DojoProgress>,
    pub scrolls: usize,
    pub streak: DojoStreak,
    pub seals: &'a [DojoSeal],
    /// 많이 쓴 수 (수 id, 횟수) — 많은 것부터
    pub keys: &'a [(String, u32)],
    /// 수 → 내 배치의 키 이름
    pub key_of: &'a dyn Fn(Action) -> String,
    /// 도장 밖 손버릇 (docs/19) — 최근 7일 레이어 수 횟수, 맨 화살표 비율
    pub real: Option<(u64, Option<u64>)>,
}

/// 지금 맨 띠 — 마친 띠 중 가장 높은 것
pub fn belt_name(content: &Content, standing: &Standing) -> (String, String) {
    let done = standing
        .belts
        .iter()
        .rposition(|b| b.completed)
        .map(|i| &content.curriculum.belt[i]);
    match done {
        Some(b) => (b.name.clone(), b.id.clone()),
        None => ("입문".into(), "none".into()),
    }
}

pub fn lines(d: &CardData) -> Vec<(String, Tone)> {
    let (belt, _) = belt_name(d.content, d.standing);
    let pad = |a: &str, b: &str| -> String {
        let w = super::ui::width(a) + super::ui::width(b);
        format!("{a}{}{b}", " ".repeat(WIDTH.saturating_sub(w)))
    };
    let mut out = vec![
        (
            pad(&format!("道場 {}", d.name), &format!("■ {belt}")),
            Tone::Title,
        ),
        (String::new(), Tone::Text),
    ];
    let total_lessons = d.content.lesson_count() as u32;
    let bosses: Vec<_> = d
        .content
        .curriculum
        .belt
        .iter()
        .filter_map(|b| b.boss.as_ref())
        .collect();
    let beaten = bosses
        .iter()
        .filter(|b| d.progress.get(&b.id).is_some_and(|p| p.best_stars > 0))
        .count();
    out.push((
        pad(
            &format!("연속 수련  {}일 (최고 {}일)", d.streak.days, d.streak.best),
            &format!("비급 {} / {}", d.scrolls, d.content.scrolls.scroll.len()),
        ),
        Tone::Text,
    ));
    out.push((
        pad(
            &format!("별  {} / {}", d.standing.stars, total_lessons * 3),
            &format!("관문 격파 {} / {}", beaten, bosses.len()),
        ),
        Tone::Text,
    ));
    if let Some((moves, pct)) = d.real {
        out.push((
            format!(
                "실전 7일  레이어 수 {moves}번{}",
                pct.map_or(String::new(), |p| format!(" · 맨 화살표 {p}%"))
            ),
            Tone::Text,
        ));
    }
    out.push((String::new(), Tone::Text));
    let seal = |os: &str, name: &str| {
        let mark = if d.seals.iter().any(|s| s.os == os) {
            "◆"
        } else {
            "◇"
        };
        format!("{mark} {name}")
    };
    out.push((
        format!(
            "인장  {}  {}  {}",
            seal("macos", "macOS"),
            seal("windows", "Windows"),
            seal("linux", "Linux")
        ),
        Tone::Gold,
    ));
    out.push((String::new(), Tone::Text));
    out.push(("많이 쓴 수".into(), Tone::Sub));
    let top: Vec<(Action, u32)> = d
        .keys
        .iter()
        .filter_map(|(id, n)| Action::from_id(id).map(|a| (a, *n)))
        .take(5)
        .collect();
    if top.is_empty() {
        out.push(("  아직 없다 — 과를 하나 넘으면 쌓인다".into(), Tone::Sub));
    }
    let max = top.first().map_or(1, |t| t.1.max(1));
    for (a, n) in &top {
        let bar = (*n as usize * 14).div_ceil(max as usize).max(1);
        let key = super::ui::fit(&(d.key_of)(*a), 9);
        let label = super::ui::fit(a.label(), 9);
        out.push((
            format!("  {key} {label} {} {n}", "█".repeat(bar)),
            Tone::Bar,
        ));
    }
    out.push((String::new(), Tone::Text));
    out.push(("keymander · 손이 떠나면 진다".into(), Tone::Sub));
    out
}

/// 상자를 씌운 텍스트 — 클립보드·`kmd dojo card`
pub fn boxed(lines: &[(String, Tone)]) -> String {
    let mut s = format!("╭─ kmd dojo {}╮\n", "─".repeat(WIDTH - 9));
    for (l, _) in lines {
        s.push_str(&format!("│ {} │\n", super::ui::fit(l, WIDTH)));
    }
    s.push_str(&format!("╰{}╯\n", "─".repeat(WIDTH + 2)));
    s
}

pub fn user_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "수련생".into())
}

/// 앱 상태로 만든 카드 텍스트
pub fn text(app: &super::app::App) -> String {
    boxed(&app_lines(app))
}

pub fn app_lines(app: &super::app::App) -> Vec<(String, Tone)> {
    let keys = app
        .db
        .as_ref()
        .map(kmd_core::Database::dojo_keys)
        .unwrap_or_default();
    let key_of = |a: Action| app.bindings.key(a);
    lines(&CardData {
        name: user_name(),
        content: app.content,
        standing: &app.standing,
        progress: &app.progress,
        scrolls: app.scrolls.len(),
        streak: app.streak,
        seals: &app.seals,
        keys: &keys,
        key_of: &key_of,
        real: app.real_usage(),
    })
}
