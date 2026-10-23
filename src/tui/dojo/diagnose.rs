//! 사부의 배치 진단 — 표준과 네 배치를 수 단위로 나란히 놓고 비급 규칙으로 본다
//! (docs/10 §6-5, S19).
//!
//! 진단은 꾸짖음이 아니라 이치의 설명이고, 아무것도 잠그지 않는다. 안내·시연·Par 는
//! 이미 사용자 배치로 다시 계산되어 있다.

use kmd_core::config::KeymapConfig;

use super::action::{Action, Os};
use super::binding::{hand, Bindings, Hand, Trust};
use super::content::Content;

/// 표에 올리는 수 — 커리큘럼 순서
const ROWS: [Action; 22] = [
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
    Action::PointerUp,
    Action::PointerDown,
    Action::PointerLeft,
    Action::PointerRight,
    Action::ClickLeft,
    Action::ClickRight,
    Action::WheelUp,
    Action::WheelDown,
    Action::ToggleHangul,
    Action::Launcher,
];

/// 진단 표의 한 줄
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub standard: String,
    pub mine: String,
    pub changed: bool,
    /// `→ 주황띠 3과 제외` 같은 결과
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// 이치에 어긋난다 — 그래도 막지 않는다
    Warn,
    /// 대가를 설명한다
    Info,
    /// 지켜진 이치
    Good,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub level: Level,
    /// 비급 권 번호
    pub scroll: &'static str,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct Diagnosis {
    /// 바뀐 줄이 먼저
    pub rows: Vec<Row>,
    pub findings: Vec<Finding>,
}

impl Diagnosis {
    pub fn changed(&self) -> usize {
        self.rows.iter().filter(|r| r.changed).count()
    }
}

/// 키보드 줄 — 완충지대(옆 칸) 판정용
const KEY_ROWS: [&str; 3] = ["QWERTYUIOP[]", "ASDFGHJKL;'", "ZXCVBNM,./"];

fn neighbors(key: &str) -> Vec<String> {
    let k = key.to_ascii_uppercase();
    let mut out = Vec::new();
    if k.chars().count() != 1 {
        return out;
    }
    let c = k.chars().next().unwrap_or(' ');
    for row in KEY_ROWS {
        let chars: Vec<char> = row.chars().collect();
        if let Some(i) = chars.iter().position(|x| *x == c) {
            if i > 0 {
                out.push(chars[i - 1].to_string());
            }
            if let Some(n) = chars.get(i + 1) {
                out.push(n.to_string());
            }
        }
    }
    out
}

fn norm(target: &str) -> String {
    target.to_ascii_lowercase().replace(' ', "")
}

/// 단어·줄 단위로 지우는 수 — 완충지대 규칙
fn is_word_delete(target: &str) -> bool {
    let t = norm(target);
    ["ctrl+", "alt+", "cmd+", "option+"].iter().any(|m| {
        t == format!("{m}backspace") || t == format!("{m}delete") || t == format!("{m}del")
    })
}

/// 잘못 누르면 무해하지 않은 수 — 레이어 불변식 규칙
fn is_harmful(target: &str) -> bool {
    let t = norm(target);
    t.starts_with("macro:")
        || t == "enter"
        || t == "return"
        || [
            "ctrl+v",
            "cmd+v",
            "ctrl+x",
            "cmd+x",
            "shift+insert",
            "shift+delete",
        ]
        .contains(&t.as_str())
}

fn is_modifier(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "lalt"
            | "ralt"
            | "alt"
            | "lctrl"
            | "rctrl"
            | "ctrl"
            | "lshift"
            | "rshift"
            | "shift"
            | "lcmd"
            | "rcmd"
            | "cmd"
            | "lwin"
            | "rwin"
            | "win"
            | "meta"
    )
}

fn hand_name(h: Hand) -> &'static str {
    match h {
        Hand::Left => "왼손",
        Hand::Right => "오른손",
    }
}

/// 레이어의 모든 키 (매핑 + 레이어 더블탭)
fn layer_keys(l: &kmd_core::config::LayerToml) -> Vec<(&String, &String)> {
    l.mappings
        .iter()
        .chain(l.double_taps.iter().map(|(k, d)| (k, &d.double)))
        .collect()
}

pub fn run(
    effective: &KeymapConfig,
    standard: &KeymapConfig,
    mine: &Bindings,
    std: &Bindings,
    content: &Content,
    os: Os,
) -> Diagnosis {
    let mut rows = Vec::new();
    for a in ROWS {
        let s = if std.has(a) {
            std.label(a)
        } else {
            "없음".into()
        };
        let m = if mine.has(a) {
            mine.label(a)
        } else {
            "없음".into()
        };
        let changed = s != m;
        let note = (!mine.has(a))
            .then(|| excluded_lessons(content, a))
            .flatten();
        rows.push(Row {
            name: a.label().to_string(),
            standard: s,
            mine: m,
            changed,
            note,
        });
    }
    // 레이어 트리거
    for (name, layer) in &effective.layers {
        let s = standard.layers.get(name).map(|l| l.trigger.clone());
        let changed = s.as_deref() != Some(layer.trigger.as_str());
        rows.push(Row {
            name: format!("{name} 레이어"),
            standard: s.map_or("—".into(), |t| format!("{t} 쥠")),
            mine: format!("{} 쥠", layer.trigger),
            changed,
            note: None,
        });
    }
    // 수가 아닌 매핑 — 표준에 없는 것만
    for (name, layer) in &effective.layers {
        for (key, target) in &layer.mappings {
            if Action::from_target(target, os).is_some() {
                continue;
            }
            let same = standard
                .layers
                .get(name)
                .and_then(|l| l.mappings.get(key))
                .is_some_and(|t| t == target);
            if !same {
                rows.push(Row {
                    name: "(수 아님)".into(),
                    standard: "—".into(),
                    mine: format!("{}+{key} {target}", layer.trigger),
                    changed: true,
                    note: None,
                });
            }
        }
    }
    rows.sort_by_key(|r| !r.changed);

    let findings = if mine.trust == Trust::Unknown {
        Vec::new()
    } else {
        findings(effective, standard, os)
    };
    Diagnosis { rows, findings }
}

/// 그 수가 주제인 과 — 승급 조건에서 빠진다
fn excluded_lessons(content: &Content, a: Action) -> Option<String> {
    let mut names = Vec::new();
    for b in &content.curriculum.belt {
        for (i, l) in b.lesson.iter().enumerate() {
            if l.needed_actions().contains(&a) {
                names.push(format!("{} {}과", b.name, i + 1));
            }
        }
    }
    (!names.is_empty()).then(|| format!("→ {} 제외", names.join(", ")))
}

fn findings(effective: &KeymapConfig, standard: &KeymapConfig, os: Os) -> Vec<Finding> {
    let mut out = Vec::new();
    for (name, layer) in &effective.layers {
        let keys = layer_keys(layer);
        // 卷一 두 손의 법칙 — 쥐는 손 ≠ 움직이는 손
        if let Some(th) = hand(&layer.trigger) {
            let hands: Vec<Hand> = keys.iter().filter_map(|(k, _)| hand(k)).collect();
            let same = hands.iter().filter(|h| **h == th).count();
            if !hands.is_empty() {
                if same * 2 > hands.len() {
                    out.push(Finding {
                        level: Level::Warn,
                        scroll: "一",
                        text: format!(
                            "{name}: 트리거 {} 와 매핑 키 대부분이 {} — 한 손으로 쥐어짠다",
                            layer.trigger,
                            hand_name(th)
                        ),
                    });
                } else {
                    let other = match th {
                        Hand::Left => Hand::Right,
                        Hand::Right => Hand::Left,
                    };
                    out.push(Finding {
                        level: Level::Good,
                        scroll: "一",
                        text: format!(
                            "{name}: 쥐는 손({})과 움직이는 손({})이 다르다",
                            hand_name(th),
                            hand_name(other)
                        ),
                    });
                }
            }
        }
        // 卷四 레이어 불변식 · 卷七 완충지대
        let deletes: Vec<&String> = keys
            .iter()
            .filter(|(_, t)| {
                matches!(
                    Action::from_target(t, os),
                    Some(Action::Backspace | Action::Delete)
                )
            })
            .map(|(k, _)| *k)
            .collect();
        for (key, target) in &keys {
            if is_harmful(target) {
                out.push(Finding {
                    level: Level::Warn,
                    scroll: "四",
                    text: format!("{name} {key} = {target} — 잘못 누르면 무해하지 않다"),
                });
            } else if is_word_delete(target) {
                let beside = deletes
                    .iter()
                    .find(|d| neighbors(d).iter().any(|n| n.eq_ignore_ascii_case(key)));
                let (scroll, why) = match beside {
                    Some(d) => (
                        "七",
                        format!("지우기 {d} 바로 옆 — 손이 미끄러지면 단어가 사라진다"),
                    ),
                    None => ("四", "잘못 누르면 단어가 사라진다".to_string()),
                };
                out.push(Finding {
                    level: Level::Warn,
                    scroll,
                    text: format!("{name} {key} = {target} — {why}"),
                });
            }
        }
        // 卷九 CapsLock 승격 — 표준과 다른 modifier 트리거
        let std_trigger = standard.layers.get(name).map(|l| l.trigger.as_str());
        if is_modifier(&layer.trigger) && std_trigger != Some(layer.trigger.as_str()) {
            out.push(Finding {
                level: Level::Info,
                scroll: "九",
                text: format!(
                    "{name}: 트리거 {} 는 modifier — 앱의 {0} 단축키를 가리는 대신 CapsLock 이 다른 일(모드탭 Ctrl 등)에 쓰인다",
                    layer.trigger
                ),
            });
        }
        // 卷十三 ESDF 의 이치
        let wasd = [
            ("W", "mouse:up"),
            ("A", "mouse:left"),
            ("S", "mouse:down"),
            ("D", "mouse:right"),
        ];
        if wasd
            .iter()
            .all(|(k, t)| layer.mappings.get(*k).is_some_and(|v| norm(v) == *t))
        {
            out.push(Finding {
                level: Level::Info,
                scroll: "十三",
                text: format!(
                    "{name}: 포인터가 WASD — 왼손이 홈로우에서 한 칸 밀린다. ESDF 는 검지가 F 에 머문다"
                ),
            });
        }
    }
    out.sort_by_key(|f| f.level);
    out
}

/// 현재 config 로 진단한다
pub fn from_config(config: &kmd_core::Config, mine: &Bindings, content: &Content) -> Diagnosis {
    let effective = kmd_core::keymap::effective_keymap(&config.launcher.keymap);
    let standard = kmd_core::keymap::effective_keymap(&KeymapConfig::default());
    let os = Os::current();
    let std = Bindings::resolve(&standard, &standard, os, Trust::Full);
    run(&effective, &standard, mine, &std, content, os)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::dojo::content::content;
    use kmd_core::config::LayerToml;
    use kmd_core::keymap::effective_keymap;

    fn diagnose(user: KeymapConfig) -> Diagnosis {
        let os = Os::current();
        let eff = effective_keymap(&user);
        let std = effective_keymap(&KeymapConfig::default());
        let mine = Bindings::resolve(&eff, &std, os, Trust::Full);
        let sb = Bindings::resolve(&std, &std, os, Trust::Full);
        run(&eff, &std, &mine, &sb, content(), os)
    }

    #[test]
    fn 표준_배치는_바뀐_수도_경고도_없다() {
        let d = diagnose(KeymapConfig::default());
        assert_eq!(
            d.changed(),
            0,
            "{:?}",
            d.rows.iter().filter(|r| r.changed).collect::<Vec<_>>()
        );
        assert!(
            d.findings.iter().all(|f| f.level == Level::Good),
            "{:?}",
            d.findings
        );
        assert!(d.findings.iter().any(|f| f.scroll == "一"));
    }

    #[test]
    fn 붙여넣기와_같은_손_트리거와_완충지대를_짚는다() {
        let mut km = KeymapConfig::default();
        let mut mappings = std::collections::BTreeMap::new();
        for (k, v) in [
            ("H", "Left"),
            ("J", "Down"),
            ("K", "Up"),
            ("L", "Right"),
            ("P", "Ctrl+V"),
            (".", "Backspace"),
            (",", "Ctrl+Backspace"),
        ] {
            mappings.insert(k.to_string(), v.to_string());
        }
        km.layers.insert(
            "nav".into(),
            LayerToml {
                trigger: "RAlt".into(),
                mappings,
                ..Default::default()
            },
        );
        let d = diagnose(km);
        let has = |scroll: &str, level: Level| {
            d.findings
                .iter()
                .any(|f| f.scroll == scroll && f.level == level)
        };
        assert!(
            has("一", Level::Warn),
            "RAlt + HJKL = 같은 손 {:?}",
            d.findings
        );
        assert!(has("四", Level::Warn), "P = 붙여넣기");
        assert!(has("七", Level::Warn), ", = 단어 지우기가 . 옆");
        assert!(has("九", Level::Info), "트리거가 modifier");
        assert!(d
            .rows
            .iter()
            .any(|r| r.name == "(수 아님)" && r.mine.contains("Ctrl+V")));
        assert_eq!(d.findings[0].level, Level::Warn, "경고가 먼저");
    }

    #[test]
    fn 없는_수는_그_수가_주제인_과를_승급에서_뺀다() {
        let note = excluded_lessons(content(), Action::PageUp).unwrap();
        assert!(note.contains("주황띠 3과"), "{note}");
        assert_eq!(excluded_lessons(content(), Action::Launcher), None);
    }

    #[test]
    fn wasd_포인터는_정보로_알린다() {
        let mut km = KeymapConfig::default();
        let mut mappings = std::collections::BTreeMap::new();
        for (k, v) in [
            ("W", "mouse:up"),
            ("A", "mouse:left"),
            ("S", "mouse:down"),
            ("D", "mouse:right"),
        ] {
            mappings.insert(k.to_string(), v.to_string());
        }
        km.layers.insert(
            "mouse".into(),
            LayerToml {
                trigger: "RAlt".into(),
                mappings,
                ..Default::default()
            },
        );
        let d = diagnose(km);
        assert!(
            d.findings.iter().any(|f| f.scroll == "十三"),
            "{:?}",
            d.findings
        );
    }
}
