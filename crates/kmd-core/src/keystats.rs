//! 손버릇 통계 (docs/19) — 데몬이 센 횟수를 모아 읽고, 배치에 비춰 해석한다.
//!
//! **글자는 세지 않는다.** 데몬 엔진이 남기는 지표는 넷뿐이다:
//!
//! - `move:<레이어>:<키>` — 레이어 매핑이 실행된 횟수 (오토리피트는 한 번)
//! - `raw:<키>` — 레이어 밖에서 누른 맨 화살표·Home·End·PgUp·PgDn ("화살귀")
//! - `hold:<0~10>` — 트리거를 쥔 시간, 100ms 칸 (10 = 1초 이상)
//! - `idle:<레이어>` — 트리거를 쥐었다가 아무 수도 안 쓰고 놓은 횟수
//!
//! 순서도 시각도 없다. 날짜(로컬) 단위로 더할 뿐이다. 켜는 스위치는
//! `[launcher.keymap] stats = true` 이고 기본은 꺼져 있다.

use std::collections::BTreeMap;

use crate::config::KeymapConfig;
use crate::db::{Database, DbError};

/// 홀드 시간 칸 수 (100ms × 10 + "1초 이상")
pub const HOLD_BUCKETS: usize = 11;

impl Database {
    /// 오늘(로컬) 칸에 횟수를 더한다
    pub fn key_stats_add(&self, counts: &[(String, u32)]) -> Result<(), DbError> {
        let tx = self.conn().unchecked_transaction()?;
        for (metric, n) in counts {
            tx.execute(
                "INSERT INTO key_stats(day, metric, count) VALUES (date('now', 'localtime'), ?1, ?2)
                 ON CONFLICT(day, metric) DO UPDATE SET count = count + ?2",
                rusqlite::params![metric, n],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 최근 `days` 일(오늘 포함)의 지표별 합계와 데이터가 있는 날 수
    pub fn key_stats_since(&self, days: u32) -> (Vec<(String, u64)>, u32) {
        let since = format!("-{} days", days.max(1) - 1);
        let rows = self
            .conn()
            .prepare(
                "SELECT metric, SUM(count) FROM key_stats
                 WHERE day >= date('now', 'localtime', ?1) GROUP BY metric",
            )
            .and_then(|mut stmt| {
                stmt.query_map([&since], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64)))
                    .map(|rows| rows.flatten().collect())
            })
            .unwrap_or_default();
        let active = self
            .conn()
            .query_row(
                "SELECT COUNT(DISTINCT day) FROM key_stats WHERE day >= date('now', 'localtime', ?1)",
                [&since],
                |r| r.get(0),
            )
            .unwrap_or(0);
        (rows, active)
    }

    /// 모두 지운다 (`kmd keymap stats --clear`)
    pub fn key_stats_clear(&self) -> Result<usize, DbError> {
        Ok(self.conn().execute("DELETE FROM key_stats", [])?)
    }
}

/// 레이어 수 하나의 사용 횟수
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveCount {
    pub layer: String,
    /// 키 이름 (데몬 표기 — `I`, `Semicolon` …)
    pub key: String,
    /// 배치상 그 키가 보내는 것 (`Ctrl+Left` …). 배치에서 빠진 키면 `None`.
    pub target: Option<String>,
    pub count: u64,
}

/// 읽어서 배치에 비춘 결과
#[derive(Debug, Clone, Default)]
pub struct KeyReport {
    pub days: u32,
    /// 그중 데이터가 있는 날
    pub active_days: u32,
    /// 많이 쓴 것부터
    pub moves: Vec<MoveCount>,
    pub raw: Vec<(String, u64)>,
    pub holds: [u64; HOLD_BUCKETS],
    pub idle: Vec<(String, u64)>,
    /// 배치에 있지만 이 기간에 한 번도 안 쓴 수
    pub unused: Vec<MoveCount>,
}

/// 데몬 VKey 표기로 맞춘 키 이름 — 배치 TOML 은 `;`·`semicolon` 처럼 쓸 수 있다
pub fn key_id(name: &str) -> String {
    let n = name.trim();
    let lower = n.to_ascii_lowercase();
    let named = match lower.as_str() {
        ";" | "semicolon" => "Semicolon",
        "'" | "quote" => "Quote",
        "," | "comma" => "Comma",
        "." | "period" | "dot" => "Period",
        "/" | "slash" => "Slash",
        "\\" | "backslash" => "Backslash",
        "[" | "lbracket" => "LBracket",
        "]" | "rbracket" => "RBracket",
        "-" | "minus" => "Minus",
        "=" | "equal" => "Equal",
        "`" | "grave" => "Grave",
        "space" | "spc" => "Space",
        "tab" => "Tab",
        _ => "",
    };
    if !named.is_empty() {
        return named.into();
    }
    if lower.len() == 1 {
        let c = lower.chars().next().unwrap_or(' ');
        if c.is_ascii_digit() {
            return format!("Num{c}");
        }
        return c.to_ascii_uppercase().to_string();
    }
    n.to_string()
}

impl KeyReport {
    /// DB 행과 (병합이 끝난) 배치로 만든다
    pub fn build(
        rows: &[(String, u64)],
        active_days: u32,
        days: u32,
        keymap: &KeymapConfig,
    ) -> Self {
        // 배치의 레이어 수 — (레이어, 키) → 대상
        let mut mapped: BTreeMap<(String, String), String> = BTreeMap::new();
        for (layer, l) in &keymap.layers {
            for (k, target) in &l.mappings {
                mapped.insert((layer.clone(), key_id(k)), target.clone());
            }
            for (k, dt) in &l.double_taps {
                mapped.insert(
                    (layer.clone(), key_id(k)),
                    format!("{} / {}", dt.single, dt.double),
                );
            }
        }
        let mut r = KeyReport {
            days,
            active_days,
            ..Default::default()
        };
        let mut used = std::collections::BTreeSet::new();
        for (metric, n) in rows {
            let mut parts = metric.splitn(3, ':');
            match (parts.next(), parts.next(), parts.next()) {
                (Some("move"), Some(layer), Some(key)) => {
                    let id = (layer.to_string(), key.to_string());
                    used.insert(id.clone());
                    r.moves.push(MoveCount {
                        layer: layer.into(),
                        key: key.into(),
                        target: mapped.get(&id).cloned(),
                        count: *n,
                    });
                }
                (Some("raw"), Some(key), None) => r.raw.push((key.into(), *n)),
                (Some("hold"), Some(b), None) => {
                    if let Ok(i) = b.parse::<usize>() {
                        r.holds[i.min(HOLD_BUCKETS - 1)] += n;
                    }
                }
                (Some("idle"), Some(layer), None) => r.idle.push((layer.into(), *n)),
                _ => {}
            }
        }
        r.moves
            .sort_by(|a, b| b.count.cmp(&a.count).then(a.key.cmp(&b.key)));
        r.raw.sort_by(|a, b| b.1.cmp(&a.1));
        r.idle.sort_by(|a, b| b.1.cmp(&a.1));
        r.unused = mapped
            .into_iter()
            .filter(|(id, _)| !used.contains(id))
            .map(|((layer, key), target)| MoveCount {
                layer,
                key,
                target: Some(target),
                count: 0,
            })
            .collect();
        r
    }

    pub fn total_moves(&self) -> u64 {
        self.moves.iter().map(|m| m.count).sum()
    }

    pub fn total_raw(&self) -> u64 {
        self.raw.iter().map(|r| r.1).sum()
    }

    pub fn total_holds(&self) -> u64 {
        self.holds.iter().sum()
    }

    pub fn total_idle(&self) -> u64 {
        self.idle.iter().map(|r| r.1).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.total_moves() + self.total_raw() + self.total_holds() == 0
    }

    /// 이동 중 맨 화살표 비율 (0~100) — 레이어를 두고도 손이 화살표로 간 정도
    pub fn raw_pct(&self) -> Option<u64> {
        let nav_moves: u64 = self
            .moves
            .iter()
            .filter(|m| m.target.as_deref().is_some_and(is_nav_target))
            .map(|m| m.count)
            .sum();
        let raw = self.total_raw();
        let all = raw + nav_moves;
        (all > 0).then(|| raw * 100 / all)
    }

    /// 사부의 해석 — 숫자를 읽어 줄 한두 마디
    pub fn insights(&self) -> Vec<String> {
        if self.is_empty() {
            return vec!["아직 모인 것이 없다 — 데몬이 몇 분마다 저장한다.".into()];
        }
        let mut out = Vec::new();
        if let Some(pct) = self.raw_pct() {
            let raw = self.total_raw();
            out.push(match pct {
                0..=4 => format!("맨 화살표 {pct}% ({raw}번) — 손이 홈로우를 떠나지 않는다."),
                5..=19 => format!(
                    "맨 화살표 {pct}% ({raw}번) — 아직 화살표로 손이 간다. kmd dojo 의 노란띠 대련이 약이다."
                ),
                _ => format!(
                    "맨 화살표 {pct}% ({raw}번) — 화살귀에게 지고 있다. 레이어 이동부터 다시 익혀라."
                ),
            });
        }
        let holds = self.total_holds();
        let idle = self.total_idle();
        if holds >= 20 && idle * 100 / holds >= 15 {
            out.push(format!(
                "트리거를 쥐었다 그냥 놓은 일이 {idle}번 ({}%) — 스치기만 했거나 마음을 바꿨다. 탭에 동작이 없는 지금 배치라 사고는 없다.",
                idle * 100 / holds
            ));
        }
        if holds >= 20 {
            let quick: u64 = self.holds[..2].iter().sum();
            out.push(format!(
                "홀드의 {}% 가 200ms 안에 끝났다 — 레이어는 짧게 쥐고 바로 놓는 손이다.",
                quick * 100 / holds
            ));
        }
        if let Some(top) = self.moves.first() {
            out.push(format!(
                "가장 많이 쓴 수: {} {} → {} ({}번)",
                top.layer,
                top.key,
                top.target.as_deref().unwrap_or("?"),
                top.count
            ));
        }
        let unused: Vec<String> = self
            .unused
            .iter()
            .filter(|m| m.layer == "nav")
            .map(|m| m.key.clone())
            .collect();
        if !unused.is_empty() && self.total_moves() >= 50 {
            out.push(format!(
                "nav 에서 이 기간 한 번도 안 쓴 수: {} — 몰라서 안 쓰는지, 필요 없는지 가려 보라.",
                unused.join(" ")
            ));
        }
        out
    }
}

/// 맨 화살표와 같은 일을 하는 레이어 대상인가 (비율 계산용)
fn is_nav_target(target: &str) -> bool {
    let t = target.to_ascii_lowercase();
    [
        "left", "right", "up", "down", "home", "end", "pageup", "pagedown",
    ]
    .iter()
    .any(|k| t == *k || t.ends_with(&format!("+{k}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LayerToml;

    fn keymap() -> KeymapConfig {
        let mut km = KeymapConfig::default();
        let mut nav = LayerToml {
            trigger: "CapsLock".into(),
            tap_action: None,
            tap_hold_ms: None,
            unmapped: None,
            mappings: BTreeMap::new(),
            double_taps: BTreeMap::new(),
        };
        for (k, t) in [
            ("H", "Left"),
            ("L", "Right"),
            ("I", "Alt+Left"),
            (".", "Backspace"),
        ] {
            nav.mappings.insert(k.into(), t.into());
        }
        km.layers.insert("nav".into(), nav);
        km
    }

    #[test]
    fn 지표를_읽어_배치에_비춘다() {
        let rows = vec![
            ("move:nav:H".to_string(), 30),
            ("move:nav:I".to_string(), 50),
            ("move:nav:Period".to_string(), 10),
            ("raw:Left".to_string(), 20),
            ("hold:1".to_string(), 40),
            ("hold:3".to_string(), 10),
            ("hold:99".to_string(), 1),
            ("idle:nav".to_string(), 12),
        ];
        let r = KeyReport::build(&rows, 3, 7, &keymap());
        assert_eq!(r.moves[0].key, "I");
        assert_eq!(r.moves[0].target.as_deref(), Some("Alt+Left"));
        assert_eq!(r.total_moves(), 90);
        assert_eq!(r.holds[1], 40);
        assert_eq!(r.holds[10], 1, "넘치는 칸은 마지막 칸");
        // 이동 대상(H·I)만 비율에 든다: 20 / (20 + 80)
        assert_eq!(r.raw_pct(), Some(20));
        assert_eq!(r.unused.len(), 1);
        assert_eq!(r.unused[0].key, "L");
        let say = r.insights().join("\n");
        assert!(say.contains("맨 화살표 20%"), "{say}");
        assert!(say.contains("그냥 놓은 일이 12번"), "{say}");
    }

    #[test]
    fn 키_이름은_데몬_표기로_맞춘다() {
        assert_eq!(key_id(";"), "Semicolon");
        assert_eq!(key_id("h"), "H");
        assert_eq!(key_id("1"), "Num1");
        assert_eq!(key_id("Period"), "Period");
    }

    #[test]
    fn 날마다_더하고_기간으로_읽는다() {
        let db = Database::open_in_memory().unwrap();
        assert!(db.key_stats_since(7).0.is_empty());
        db.key_stats_add(&[("move:nav:H".into(), 3), ("raw:Left".into(), 1)])
            .unwrap();
        db.key_stats_add(&[("move:nav:H".into(), 2)]).unwrap();
        let (rows, days) = db.key_stats_since(7);
        assert_eq!(days, 1);
        let h = rows.iter().find(|r| r.0 == "move:nav:H").unwrap();
        assert_eq!(h.1, 5);
        assert_eq!(db.key_stats_clear().unwrap(), 2);
    }

    #[test]
    fn 빈_통계는_기다리라고_한다() {
        let r = KeyReport::build(&[], 0, 7, &keymap());
        assert!(r.is_empty());
        assert!(r.insights()[0].contains("아직"));
    }
}
