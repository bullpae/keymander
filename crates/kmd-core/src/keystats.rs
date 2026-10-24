//! 손버릇 통계 (docs/19) — 데몬이 센 횟수를 모아 읽고, 배치에 비춰 해석한다.
//!
//! **글자는 세지 않는다.** 데몬 엔진이 남기는 지표는 넷뿐이다:
//!
//! - `move:<레이어>:<키>` — 레이어 매핑이 실행된 횟수 (오토리피트는 한 번)
//! - `raw:<키>` — 레이어 밖에서 누른 맨 화살표·Home·End·PgUp·PgDn ("화살귀")
//! - `hold:<0~10>` — 트리거를 쥔 시간, 100ms 칸 (10 = 1초 이상)
//! - `idle:<레이어>` — 트리거를 쥐었다가 아무 수도 안 쓰고 놓은 횟수
//! - `tap:<레이어>` — 트리거를 짧게 쳐서 탭 동작(`tap_action`, 예: 한영)을 낸 횟수
//! - `perf:<단계>:<칸>` — 단축키로 런처를 띄울 때 단계별 지연 (§7). `perf:hookslow` 는
//!   20ms 넘게 걸린 키 훅 콜백 수 (Windows)
//!
//! 순서도 시각도 없다. 날짜(로컬) 단위로 더할 뿐이다. 켜는 스위치는
//! `[launcher.keymap] stats = true` 이고 기본은 꺼져 있다.
//!
//! 여러 기기 합쳐 보기(docs/19 §6): 기기마다 [`Snapshot`] (날짜별 합계 + 그 기기의 배치)을
//! 공유 폴더에 `kmd-keystats-<기기>.json` 으로 내보내고, [`Compare`] 가 기기별로 나란히 놓고
//! 차이를 읽는다. 기기 사이에 오가는 것도 횟수뿐이다.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::KeymapConfig;
use crate::db::{Database, DbError};

/// 홀드 시간 칸 수 (100ms × 10 + "1초 이상")
pub const HOLD_BUCKETS: usize = 11;

/// 데몬이 Launch 로 띄운 자식에게 실행 시각(유닉스 ms)을 넘기는 환경 변수 (§7)
pub const LAUNCH_AT_ENV: &str = "KMD_LAUNCH_AT_MS";

/// 지연 칸의 위 경계 (ms) — 마지막 칸은 그 이상
pub const PERF_EDGES: [u64; 10] = [25, 50, 100, 150, 200, 300, 500, 800, 1200, 2000];
pub const PERF_BUCKETS: usize = PERF_EDGES.len() + 1;

/// 실행 지연 단계 — (지표 이름, 보고서 이름)
pub const PERF_PHASES: [(&str, &str); 5] = [
    ("wait", "트리거 대기"),
    ("spawn", "프로세스 생성"),
    ("window", "창 준비"),
    ("engine", "검색 준비"),
    ("total", "단축키 → 사용 가능"),
];

/// 지연(ms)이 드는 칸
pub fn perf_bucket(ms: u64) -> usize {
    PERF_EDGES
        .iter()
        .position(|&e| ms < e)
        .unwrap_or(PERF_EDGES.len())
}

/// 칸 이름 — `<100ms`, `2초+`
pub fn perf_label(bucket: usize) -> String {
    match PERF_EDGES.get(bucket) {
        Some(e) => format!("<{e}ms"),
        None => "2초+".into(),
    }
}

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

    /// 최근 `days` 일의 날짜별 행 — (날짜, 지표, 횟수)
    pub fn key_stats_rows(&self, days: u32) -> Vec<(String, String, u64)> {
        let since = format!("-{} days", days.max(1) - 1);
        self.conn()
            .prepare(
                "SELECT day, metric, count FROM key_stats
                 WHERE day >= date('now', 'localtime', ?1) ORDER BY day, metric",
            )
            .and_then(|mut stmt| {
                stmt.query_map([&since], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? as u64))
                })
                .map(|rows| rows.flatten().collect())
            })
            .unwrap_or_default()
    }

    /// 오늘(로컬)에서 `offset` 일 옮긴 날짜 (`YYYY-MM-DD`)
    pub fn local_date(&self, offset: i32) -> String {
        self.conn()
            .query_row(
                "SELECT date('now', 'localtime', ?1)",
                [format!("{offset} days")],
                |r| r.get(0),
            )
            .unwrap_or_default()
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
    /// 탭 동작을 낸 트리거 탭 (레이어별)
    pub taps: Vec<(String, u64)>,
    /// 실행 지연 — 단계 → 칸별 횟수
    pub perf: BTreeMap<String, [u64; PERF_BUCKETS]>,
    /// 20ms 넘게 걸린 키 훅 콜백 (Windows)
    pub hook_slow: u64,
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
        Self::build_mapped(rows, active_days, days, mapped_of(keymap))
    }

    /// 배치 대신 (레이어, 키) → 대상 표로 만든다 — 다른 기기의 스냅숏용
    pub fn build_mapped(
        rows: &[(String, u64)],
        active_days: u32,
        days: u32,
        mapped: BTreeMap<(String, String), String>,
    ) -> Self {
        let mut r = KeyReport {
            days,
            active_days,
            ..Default::default()
        };
        let mut used = BTreeSet::new();
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
                (Some("tap"), Some(layer), None) => r.taps.push((layer.into(), *n)),
                (Some("perf"), Some("hookslow"), None) => r.hook_slow += n,
                (Some("perf"), Some(phase), Some(b)) => {
                    if let Ok(i) = b.parse::<usize>() {
                        r.perf.entry(phase.to_string()).or_insert([0; PERF_BUCKETS])
                            [i.min(PERF_BUCKETS - 1)] += n;
                    }
                }
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

    pub fn total_taps(&self) -> u64 {
        self.taps.iter().map(|r| r.1).sum()
    }

    pub fn total_idle(&self) -> u64 {
        self.idle.iter().map(|r| r.1).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.total_moves() + self.total_raw() + self.total_holds() == 0
            && self.taps.is_empty()
            && self.perf.is_empty()
            && self.hook_slow == 0
    }

    /// 그 단계의 측정 횟수
    pub fn perf_count(&self, phase: &str) -> u64 {
        self.perf.get(phase).map_or(0, |b| b.iter().sum())
    }

    /// 그 단계 지연의 `pct` 백분위가 드는 칸 (예: 50 = 중앙값)
    pub fn perf_pct(&self, phase: &str, pct: u64) -> Option<usize> {
        let b = self.perf.get(phase)?;
        let all: u64 = b.iter().sum();
        if all == 0 {
            return None;
        }
        let need = (all * pct).div_ceil(100).max(1);
        let mut acc = 0;
        b.iter().position(|n| {
            acc += n;
            acc >= need
        })
    }

    /// 프로세스 생성·창·검색 중 중앙값이 가장 늦은 단계 — (지표 이름, 칸)
    pub fn slowest_phase(&self) -> Option<(&'static str, usize)> {
        ["wait", "spawn", "window", "engine"]
            .into_iter()
            .filter_map(|p| self.perf_pct(p, 50).map(|b| (p, b)))
            .max_by_key(|x| x.1)
    }

    /// 200ms 안에 끝난 홀드 비율 (0~100)
    pub fn quick_pct(&self) -> Option<u64> {
        let holds = self.total_holds();
        (holds > 0).then(|| self.holds[..2].iter().sum::<u64>() * 100 / holds)
    }

    /// 쥐었다 그냥 놓은 비율 (0~100)
    pub fn idle_pct(&self) -> Option<u64> {
        let holds = self.total_holds();
        (holds > 0).then(|| self.total_idle() * 100 / holds)
    }

    /// (레이어, 키) 의 횟수
    pub fn count_of(&self, layer: &str, key: &str) -> u64 {
        self.moves
            .iter()
            .find(|m| m.layer == layer && m.key == key)
            .map_or(0, |m| m.count)
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
            return vec!["아직 모인 것이 없다 — 데몬이 1분마다 저장한다.".into()];
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
                "트리거를 쥐었다 그냥 놓은 일이 {idle}번 ({}%) — 스치기만 했거나 마음을 바꿨다 (탭 동작을 낸 탭은 빼고 셌다).",
                idle * 100 / holds
            ));
        }
        if let Some(quick) = self.quick_pct().filter(|_| holds >= 20) {
            out.push(format!(
                "홀드의 {quick}% 가 200ms 안에 끝났다 — 레이어는 짧게 쥐고 바로 놓는 손이다."
            ));
        }
        if let Some(say) = self.perf_insight() {
            out.push(say);
        }
        if self.hook_slow > 0 {
            out.push(format!(
                "키 훅 콜백이 20ms 넘게 걸린 일 {}번 — 많으면 키 입력 자체가 끈적인다. 데몬 CPU·백신 간섭을 보라.",
                self.hook_slow
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

impl KeyReport {
    /// 실행 지연 한 줄 해석 — 단축키 실행이 5번 넘게 모였을 때
    pub fn perf_insight(&self) -> Option<String> {
        if self.perf_count("total") < 5 {
            return None;
        }
        let p50 = perf_label(self.perf_pct("total", 50)?);
        let p90 = perf_label(self.perf_pct("total", 90)?);
        let mut say = format!("단축키 → 런처 사용 가능: 중앙값 {p50}, 90% {p90}.");
        if let Some((phase, b)) = self.slowest_phase() {
            say.push_str(&format!(
                " 가장 늦은 단계는 {} ({}) — {}",
                phase_name(phase),
                perf_label(b),
                phase_advice(phase)
            ));
        }
        Some(say)
    }
}

fn phase_name(phase: &str) -> &'static str {
    PERF_PHASES
        .iter()
        .find(|p| p.0 == phase)
        .map_or("?", |p| p.1)
}

/// 단계별 처방 — 어느 단계가 늦은지가 곧 원인의 자리다
fn phase_advice(phase: &str) -> &'static str {
    match phase {
        "wait" => "트리거 키를 뗄 때까지 실행을 미룬다. 수정자(Alt 등) 트리거라서다 — CapsLock 트리거는 바로 뜬다.",
        "spawn" => "런처 프로세스를 새로 띄우는 비용이다. 백신 실시간 검사·느린 디스크·절전 모드가 흔한 원인이고, 상주 모드가 근본 처방이다.",
        "window" => "창과 GPU 초기화다. 노트북의 절전 GPU 전환이 흔한 원인 — general.renderer = \"software\" 와 비교해 보라.",
        "engine" => "검색 인덱스를 읽는 시간이다. 색인 범위(search_paths)가 크면 늘어난다.",
        _ => "",
    }
}

/// 배치의 레이어 수 — (레이어, 키) → 대상
pub fn mapped_of(keymap: &KeymapConfig) -> BTreeMap<(String, String), String> {
    let mut mapped = BTreeMap::new();
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
    mapped
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

// ── 여러 기기 합쳐 보기 (docs/19 §6) ───────────────────────────────────────

/// 스냅숏에 담는 기간 (일)
pub const SNAPSHOT_DAYS: u32 = 180;
const SNAPSHOT_VERSION: u32 = 1;
const SNAPSHOT_PREFIX: &str = "kmd-keystats-";

/// 한 기기의 손버릇 통계 — 공유 폴더로 옮겨 다른 기기와 나란히 본다.
/// 담기는 것은 날짜별 지표 합계와 그 기기의 배치(레이어, 키, 대상)뿐이다.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub device: String,
    /// 설치마다 한 번 만드는 임의 id — 호스트 이름이 같은 두 기기를 가른다
    #[serde(default)]
    pub id: String,
    pub os: String,
    /// 내보낸 날 (그 기기의 로컬 날짜)
    pub exported: String,
    /// 그 기기의 배치 — (레이어, 키, 대상). `launch:` 대상은 가린다 (경로·명령이 담길 수 있다)
    pub mapped: Vec<(String, String, String)>,
    /// (날짜, 지표, 횟수)
    pub rows: Vec<(String, String, u64)>,
}

impl Snapshot {
    pub fn from_db(db: &Database, device: &str, id: &str, keymap: &KeymapConfig) -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            device: device.to_string(),
            id: id.to_string(),
            os: std::env::consts::OS.to_string(),
            exported: db.local_date(0),
            mapped: mapped_of(keymap)
                .into_iter()
                .map(|((l, k), t)| {
                    let t = if t.starts_with("launch:") {
                        "launch".to_string()
                    } else {
                        t
                    };
                    (l, k, t)
                })
                .collect(),
            rows: db.key_stats_rows(SNAPSHOT_DAYS),
        }
    }

    pub fn file_name(&self) -> String {
        if self.id.is_empty() {
            format!("{SNAPSHOT_PREFIX}{}.json", safe_name(&self.device))
        } else {
            format!(
                "{SNAPSHOT_PREFIX}{}-{}.json",
                safe_name(&self.device),
                safe_name(&self.id)
            )
        }
    }

    /// 같은 기기(설치)의 스냅숏인가 — id 가 없던 옛 파일은 이름으로
    pub fn same_device(&self, device: &str, id: &str) -> bool {
        if self.id.is_empty() || id.is_empty() {
            self.device == device
        } else {
            self.id == id
        }
    }

    fn mapped_map(&self) -> BTreeMap<(String, String), String> {
        self.mapped
            .iter()
            .map(|(l, k, t)| ((l.clone(), k.clone()), t.clone()))
            .collect()
    }

    /// `since`(YYYY-MM-DD, 포함) 이후의 행을 지표별로 합친다 — (합계, 데이터가 있는 날)
    fn sums(&self, since: &str) -> (BTreeMap<String, u64>, BTreeSet<String>) {
        let mut sums = BTreeMap::new();
        let mut active = BTreeSet::new();
        for (day, metric, n) in &self.rows {
            if day.as_str() >= since {
                *sums.entry(metric.clone()).or_default() += n;
                active.insert(day.clone());
            }
        }
        (sums, active)
    }

    pub fn report(&self, since: &str, days: u32) -> KeyReport {
        let (sums, active) = self.sums(since);
        let rows: Vec<(String, u64)> = sums.into_iter().collect();
        KeyReport::build_mapped(&rows, active.len() as u32, days, self.mapped_map())
    }

    /// 폴더면 그 안의 `kmd-keystats-<기기>.json` 으로, 아니면 그 파일로 쓴다.
    /// 동기화 도구가 반쯤 쓴 파일을 올리지 않게 임시 파일에 쓰고 이름을 바꾼다.
    pub fn write_to(&self, path: &Path) -> std::io::Result<PathBuf> {
        let target = if path.is_dir() {
            path.join(self.file_name())
        } else {
            path.to_path_buf()
        };
        let tmp = target.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &target)?;
        Ok(target)
    }

    /// 파일 하나, 또는 폴더 안의 스냅숏 모두. 못 읽은 파일은 (경로, 까닭) 으로 따로 돌려준다.
    pub fn load(path: &Path) -> (Vec<Snapshot>, Vec<(PathBuf, String)>) {
        let files: Vec<PathBuf> = if path.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(path)
                .map(|rd| {
                    rd.flatten()
                        .map(|e| e.path())
                        .filter(|p| {
                            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                                n.starts_with(SNAPSHOT_PREFIX) && n.ends_with(".json")
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            v.sort();
            v
        } else {
            vec![path.to_path_buf()]
        };
        let mut ok = Vec::new();
        let mut bad = Vec::new();
        for f in files {
            let parsed = std::fs::read_to_string(&f)
                .map_err(|e| e.to_string())
                .and_then(|s| serde_json::from_str::<Snapshot>(&s).map_err(|e| e.to_string()));
            match parsed {
                Ok(s) if s.version == SNAPSHOT_VERSION => ok.push(s),
                Ok(s) => bad.push((f, format!("모르는 버전 {}", s.version))),
                Err(e) => bad.push((f, e)),
            }
        }
        (ok, bad)
    }
}

/// `[launcher.keymap] stats_sync` 폴더 (`~` 를 풀어서)
pub fn sync_dir(keymap: &KeymapConfig) -> Option<PathBuf> {
    let p = keymap.stats_sync.as_ref()?;
    expand_sync_path(&p.to_string_lossy()).ok()
}

/// 공유 폴더 경로를 데몬도 같은 곳을 보도록 절대 경로로 푼다 — `~` 를 풀고,
/// 상대 경로는 지금 폴더 기준으로 붙인다(데몬의 작업 폴더는 다르다). 풀리지 않은
/// 환경 변수(`%USERPROFILE%`·`$HOME`)는 거절한다.
pub fn expand_sync_path(raw: &str) -> Result<PathBuf, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("빈 경로".into());
    }
    if s.contains('%') || s.contains('$') {
        return Err(format!(
            "환경 변수가 풀리지 않았다: {s} — 셸이 푸는 형태로(PowerShell: \"$env:USERPROFILE\\OneDrive\\kmd\") 쓰거나 전체 경로를 적어라"
        ));
    }
    let p = match s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        Some(rest) => dirs::home_dir().ok_or("홈 폴더를 찾지 못했다")?.join(rest),
        None if s == "~" => dirs::home_dir().ok_or("홈 폴더를 찾지 못했다")?,
        None => PathBuf::from(s),
    };
    std::path::absolute(&p).map_err(|e| e.to_string())
}

/// 이 기기의 통계를 공유 폴더에 내보낸다 — 데몬이 저장할 때마다 부른다.
/// 폴더가 정해지지 않았으면 `Ok(None)`.
pub fn sync_export(
    db: &Database,
    keymap_cfg: &KeymapConfig,
    effective: &KeymapConfig,
) -> std::io::Result<Option<PathBuf>> {
    let Some(dir) = sync_dir(keymap_cfg) else {
        return Ok(None);
    };
    std::fs::create_dir_all(&dir)?;
    let id = device_id(&crate::Config::default_data_dir());
    Snapshot::from_db(db, &device_name(), &id, effective)
        .write_to(&dir)
        .map(Some)
}

/// 이 설치의 임의 id — `<데이터 폴더>/device-id` 에 한 번 만들어 둔다 (8자리 hex)
pub fn device_id(data_dir: &Path) -> String {
    let path = data_dir.join("device-id");
    if let Ok(s) = std::fs::read_to_string(&path) {
        let s = s.trim();
        if !s.is_empty() {
            return s.to_string();
        }
    }
    let mut buf = [0u8; 4];
    if getrandom::fill(&mut buf).is_err() {
        return String::new();
    }
    let id: String = buf.iter().map(|b| format!("{b:02x}")).collect();
    let _ = std::fs::create_dir_all(data_dir);
    let _ = std::fs::write(&path, &id);
    id
}

/// 이 기기의 이름 — 스냅숏 파일 이름과 비교 표의 열 머리
pub fn device_name() -> String {
    #[cfg(windows)]
    let raw = std::env::var("COMPUTERNAME").unwrap_or_default();
    #[cfg(unix)]
    let raw = {
        let mut buf = [0u8; 256];
        // SAFETY: 버퍼 길이를 넘겨 주고, 성공하면 NUL 로 끝난 이름이 담긴다
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        if rc == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..end]).into_owned()
        } else {
            String::new()
        }
    };
    #[cfg(not(any(windows, unix)))]
    let raw = String::new();
    let short = raw.split('.').next().unwrap_or("").trim();
    if short.is_empty() {
        std::env::consts::OS.to_string()
    } else {
        short.to_string()
    }
}

fn safe_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// 비교 표의 한 열
#[derive(Debug, Clone)]
pub struct Device {
    pub name: String,
    pub os: String,
    pub exported: String,
    pub report: KeyReport,
}

impl Device {
    /// 이 기기의 배치에 그 수가 있는가
    fn maps(&self, layer: &str, key: &str) -> bool {
        let r = &self.report;
        r.moves
            .iter()
            .chain(&r.unused)
            .any(|m| m.layer == layer && m.key == key && m.target.is_some())
    }

    fn target(&self, layer: &str, key: &str) -> Option<&str> {
        let r = &self.report;
        r.moves
            .iter()
            .chain(&r.unused)
            .find(|m| m.layer == layer && m.key == key)
            .and_then(|m| m.target.as_deref())
    }

    /// 판단에 쓸 만큼 모였나
    fn ready(&self) -> bool {
        self.report.active_days >= COMPARE_MIN_DAYS
            && self.report.total_moves() >= COMPARE_MIN_MOVES
    }

    fn label(&self) -> String {
        format!("{} ({})", self.name, self.os)
    }
}

const COMPARE_MIN_DAYS: u32 = 3;
const COMPARE_MIN_MOVES: u64 = 100;
/// 새끼손가락 자리 (QWERTY) — 많이 쓰는 수가 여기 있으면 짚는다 (卷二)
const PINKY_KEYS: [&str; 9] = [
    "A",
    "Q",
    "Z",
    "P",
    "Semicolon",
    "Quote",
    "Slash",
    "LBracket",
    "RBracket",
];

/// 기기들을 나란히 놓은 비교
#[derive(Debug, Clone)]
pub struct Compare {
    pub days: u32,
    pub devices: Vec<Device>,
    /// 모든 기기를 더한 것 — 배치는 합집합(앞 기기 우선), `unused` 는 어느 기기에서도 안 쓴 수
    pub total: KeyReport,
}

impl Compare {
    pub fn new(snaps: &[Snapshot], since: &str, days: u32) -> Self {
        let devices = snaps
            .iter()
            .map(|s| Device {
                name: s.device.clone(),
                os: s.os.clone(),
                exported: s.exported.clone(),
                report: s.report(since, days),
            })
            .collect();
        let mut sums: BTreeMap<String, u64> = BTreeMap::new();
        let mut active = BTreeSet::new();
        let mut mapped = BTreeMap::new();
        for s in snaps {
            let (rows, days) = s.sums(since);
            for (m, n) in rows {
                *sums.entry(m).or_default() += n;
            }
            active.extend(days);
            for (id, t) in s.mapped_map() {
                mapped.entry(id).or_insert(t);
            }
        }
        let rows: Vec<(String, u64)> = sums.into_iter().collect();
        Self {
            days,
            devices,
            total: KeyReport::build_mapped(&rows, active.len() as u32, days, mapped),
        }
    }

    /// 기기 사이의 차이를 읽은 해석
    pub fn insights(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.devices.len() < 2 {
            out.push(
                "기기가 하나뿐이다 — 다른 기기에서도 `kmd keymap stats sync <공유 폴더>` 를 켜면 나란히 본다."
                    .into(),
            );
        }
        for d in self.devices.iter().filter(|d| !d.ready()) {
            out.push(format!(
                "{}: 아직 표본이 적다 ({}일, 레이어 수 {}번) — 이 기기에 대한 판단은 보류 (최소 {COMPARE_MIN_DAYS}일·{COMPARE_MIN_MOVES}번).",
                d.name,
                d.report.active_days,
                d.report.total_moves()
            ));
        }
        let ready: Vec<&Device> = self.devices.iter().filter(|d| d.ready()).collect();

        // 1. 맨 화살표 — 기기 사이 차이는 대개 배치가 아니라 환경(앱·원격·키보드) 문제다
        let raws: Vec<(&Device, u64)> = ready
            .iter()
            .filter_map(|d| d.report.raw_pct().map(|p| (*d, p)))
            .collect();
        if let (Some(hi), Some(lo)) = (
            raws.iter().max_by_key(|x| x.1),
            raws.iter().min_by_key(|x| x.1),
        ) {
            if hi.1 >= lo.1 + 10 {
                out.push(format!(
                    "맨 화살표: {} {}% vs {} {}% — {} 쪽에서 레이어를 놓는 상황(원격 데스크톱·게임·특정 앱·외장 키보드)을 찾아라. 배치보다 환경 문제일 가능성이 크다.",
                    hi.0.label(), hi.1, lo.0.label(), lo.1, hi.0.name
                ));
            } else if raws.len() >= 2 && hi.1 < 5 {
                out.push("맨 화살표가 모든 기기에서 5% 미만 — 이동은 어디서나 몸에 붙었다.".into());
            }
        }

        // 2. 어디서나 많이 쓰는 수 — 가장 좋은 자리에 있어야 한다
        if !ready.is_empty() {
            let common: Vec<&MoveCount> = self
                .total
                .moves
                .iter()
                .filter(|m| {
                    ready
                        .iter()
                        .all(|d| d.report.count_of(&m.layer, &m.key) > 0)
                })
                .take(5)
                .collect();
            if !common.is_empty() {
                let names: Vec<String> = common
                    .iter()
                    .map(|m| format!("{} {}", m.layer, m.key))
                    .collect();
                let who = match ready.as_slice() {
                    [one] => format!("{} 에서", one.name),
                    _ => "어느 기기에서나".into(),
                };
                out.push(format!(
                    "{who} 많이 쓰는 수: {} — 이 수들이 검지·중지 자리에 있는지가 배치의 첫 기준이다.",
                    names.join(", ")
                ));
                for m in common
                    .iter()
                    .filter(|m| PINKY_KEYS.contains(&m.key.as_str()))
                {
                    out.push(format!(
                        "{} {} 는 많이 쓰는데 새끼손가락 자리다 — 卷二 규칙대로 센 손가락으로 옮길 후보.",
                        m.layer, m.key
                    ));
                }
            }
        }

        // 3. 한쪽에서만 쓰는 수 — 같은 수가 그 OS 에서 안 먹거나, 다른 길로 하고 있다
        if ready.len() >= 2 {
            for m in self.total.moves.iter().filter(|m| m.count >= 30) {
                let shares: Vec<(&Device, f64)> = ready
                    .iter()
                    .filter(|d| d.maps(&m.layer, &m.key))
                    .map(|d| {
                        let n = d.report.count_of(&m.layer, &m.key) as f64;
                        (*d, n * 100.0 / d.report.total_moves().max(1) as f64)
                    })
                    .collect();
                let (Some(hi), Some(lo)) = (
                    shares.iter().max_by(|a, b| a.1.total_cmp(&b.1)),
                    shares.iter().min_by(|a, b| a.1.total_cmp(&b.1)),
                ) else {
                    continue;
                };
                if hi.1 >= 3.0 && lo.1 < 0.5 {
                    let (ht, lt) = (
                        hi.0.target(&m.layer, &m.key).unwrap_or("?"),
                        lo.0.target(&m.layer, &m.key).unwrap_or("?"),
                    );
                    let why = if ht == lt {
                        format!(
                            "대상({ht})이 같은데도 — {} 의 앱에서 안 먹거나 다른 길로 하고 있다",
                            lo.0.name
                        )
                    } else {
                        format!(
                            "대상이 다르다({}: {ht}, {}: {lt}) — {} 쪽 대상이 그 OS 에서 제 일을 하는지 보라",
                            hi.0.name, lo.0.name, lo.0.name
                        )
                    };
                    out.push(format!(
                        "{} {}: {} {:.1}% vs {} {:.1}% — {why}.",
                        m.layer, m.key, hi.0.name, hi.1, lo.0.name, lo.1
                    ));
                }
            }
        }

        // 4. 어디서도 안 쓴 수
        let unused: Vec<String> = self
            .total
            .unused
            .iter()
            .filter(|m| m.layer == "nav")
            .map(|m| m.key.clone())
            .collect();
        if !unused.is_empty() && self.total.total_moves() >= 200 {
            out.push(format!(
                "어느 기기에서도 안 쓴 nav 수: {} — 빼서 자리를 비울지, 도장에서 익힐지 가를 후보.",
                unused.join(" ")
            ));
        }

        // 5. 홀드 — 기기마다 손(키보드)이 다르면 tap_hold 판정도 따로 볼 근거
        let quick: Vec<(&Device, u64)> = ready
            .iter()
            .filter(|d| d.report.total_holds() >= 20)
            .filter_map(|d| d.report.quick_pct().map(|p| (*d, p)))
            .collect();
        if let (Some(hi), Some(lo)) = (
            quick.iter().max_by_key(|x| x.1),
            quick.iter().min_by_key(|x| x.1),
        ) {
            if hi.1 >= lo.1 + 15 {
                out.push(format!(
                    "200ms 안에 놓은 홀드: {} {}% vs {} {}% — 키보드나 손 습관이 다르다. 판정 시간(tap_hold_ms)을 기기별로 볼 근거.",
                    hi.0.name, hi.1, lo.0.name, lo.1
                ));
            }
        }
        for d in &ready {
            if let Some(p) = d.report.idle_pct().filter(|p| *p >= 15) {
                if d.report.total_holds() >= 20 {
                    out.push(format!(
                        "{}: 트리거를 쥐었다 그냥 놓음 {p}% — 오발이 잦다. 트리거 자리와 tap_action 을 점검.",
                        d.name
                    ));
                }
            }
        }

        // 7. 실행 지연 — 엔진은 같아도 프로세스·GPU·디스크는 기기마다 다르다
        let lat: Vec<(&Device, usize)> = self
            .devices
            .iter()
            .filter(|d| d.report.perf_count("total") >= 5)
            .filter_map(|d| d.report.perf_pct("total", 50).map(|b| (d, b)))
            .collect();
        if let (Some(hi), Some(lo)) = (
            lat.iter().max_by_key(|x| x.1),
            lat.iter().min_by_key(|x| x.1),
        ) {
            if hi.1 >= lo.1 + 2 {
                // 두 기기 사이 중앙값 차이가 가장 큰 단계가 원인의 자리
                let edge = |b: usize| PERF_EDGES.get(b).copied().unwrap_or(4000);
                let gap = ["wait", "spawn", "window", "engine"]
                    .into_iter()
                    .filter_map(|p| {
                        let a = hi.0.report.perf_pct(p, 50)?;
                        let b = lo.0.report.perf_pct(p, 50)?;
                        Some((p, edge(a).saturating_sub(edge(b))))
                    })
                    .max_by_key(|x| x.1);
                let mut say = format!(
                    "런처가 {} 에서 늦게 뜬다 (중앙값 {} vs {} {}).",
                    hi.0.name,
                    perf_label(hi.1),
                    lo.0.name,
                    perf_label(lo.1)
                );
                if let Some((phase, _)) = gap {
                    say.push_str(&format!(
                        " 차이가 가장 큰 단계: {} — {}",
                        phase_name(phase),
                        phase_advice(phase)
                    ));
                }
                out.push(say);
            }
        }
        for d in &self.devices {
            let n = d.report.perf_count("total");
            if (1..5).contains(&n) {
                out.push(format!(
                    "{}: 실행 지연 표본이 적다 ({n}번) — 5번은 모여야 읽는다.",
                    d.name
                ));
            }
        }

        // 6. 기기마다 대상이 다른 수 (대개 ⌥/Ctrl 관례)
        if self.devices.len() >= 2 {
            let differ = self.differing_targets();
            if differ > 0 {
                out.push(format!(
                    "기기마다 대상이 다른 수 {differ}개 (표의 대상 열) — 대개 OS 관례(⌥/Ctrl) 차이다. 같은 손짓이 같은 일을 하는지만 확인하면 된다."
                ));
            }
        }
        out
    }

    fn differing_targets(&self) -> usize {
        let mut ids: BTreeSet<(String, String)> = BTreeSet::new();
        for d in &self.devices {
            for m in d.report.moves.iter().chain(&d.report.unused) {
                ids.insert((m.layer.clone(), m.key.clone()));
            }
        }
        ids.iter()
            .filter(|(l, k)| {
                let ts: BTreeSet<&str> =
                    self.devices.iter().filter_map(|d| d.target(l, k)).collect();
                ts.len() > 1
            })
            .count()
    }

    /// 마크다운 보고서 — 터미널에서 읽고, 그대로 붙여 넣어 함께 분석한다
    pub fn markdown(&self) -> String {
        use std::fmt::Write;
        let mut o = String::new();
        let devs = &self.devices;
        let _ = writeln!(o, "# 손버릇 통계 — 기기 비교 (최근 {}일)\n", self.days);
        let _ = writeln!(o, "> 횟수만 담겼다 — 글자·순서·시각은 없다.\n");

        let head: Vec<String> = devs.iter().map(Device::label).collect();
        let _ = writeln!(o, "| | {} | 합계 |", head.join(" | "));
        let _ = writeln!(o, "|---|{}---:|", "---:|".repeat(devs.len()));
        let pct = |p: Option<u64>| p.map_or("—".to_string(), |p| format!("{p}%"));
        let mut row = |name: &str, f: &dyn Fn(&KeyReport) -> String, total: String| {
            let cells: Vec<String> = devs.iter().map(|d| f(&d.report)).collect();
            let _ = writeln!(o, "| {name} | {} | {total} |", cells.join(" | "));
        };
        row(
            "데이터 일수",
            &|r| r.active_days.to_string(),
            self.total.active_days.to_string(),
        );
        row(
            "레이어 수",
            &|r| r.total_moves().to_string(),
            self.total.total_moves().to_string(),
        );
        row(
            "하루 평균",
            &|r| (r.total_moves() / u64::from(r.active_days.max(1))).to_string(),
            "".into(),
        );
        row(
            "맨 이동 키",
            &|r| r.total_raw().to_string(),
            self.total.total_raw().to_string(),
        );
        row(
            "맨 화살표 비율",
            &|r| pct(r.raw_pct()),
            pct(self.total.raw_pct()),
        );
        row(
            "트리거 홀드",
            &|r| r.total_holds().to_string(),
            self.total.total_holds().to_string(),
        );
        row(
            "200ms 안에 놓음",
            &|r| pct(r.quick_pct()),
            pct(self.total.quick_pct()),
        );
        row(
            "그냥 놓음",
            &|r| pct(r.idle_pct()),
            pct(self.total.idle_pct()),
        );
        let lat =
            |r: &KeyReport, q: u64| r.perf_pct("total", q).map_or("—".to_string(), perf_label);
        row("실행 지연 중앙값", &|r| lat(r, 50), lat(&self.total, 50));
        row("실행 지연 90%", &|r| lat(r, 90), lat(&self.total, 90));
        row(
            "느린 키 훅 (20ms+)",
            &|r| r.hook_slow.to_string(),
            self.total.hook_slow.to_string(),
        );
        let exported: Vec<String> = devs.iter().map(|d| d.exported.clone()).collect();
        let _ = writeln!(o, "| 내보낸 날 | {} | |", exported.join(" | "));

        if !self.total.moves.is_empty() {
            let _ = writeln!(
                o,
                "\n## 레이어 수 (합계 순, 상위 20 · 괄호는 그 기기 레이어 수 중 비율)\n"
            );
            let names: Vec<&str> = devs.iter().map(|d| d.name.as_str()).collect();
            let _ = writeln!(o, "| 레이어 | 키 | 대상 | {} | 합계 |", names.join(" | "));
            let _ = writeln!(o, "|---|---|---|{}---:|", "---:|".repeat(devs.len()));
            for m in self.total.moves.iter().take(20) {
                let _ = writeln!(
                    o,
                    "| {} | {} | {} | {} | {} |",
                    m.layer,
                    m.key,
                    self.target_cell(&m.layer, &m.key),
                    self.count_cells(&m.layer, &m.key).join(" | "),
                    m.count
                );
            }
        }

        let raw_keys: BTreeSet<&str> = devs
            .iter()
            .flat_map(|d| d.report.raw.iter().map(|r| r.0.as_str()))
            .collect();
        if !raw_keys.is_empty() {
            let names: Vec<&str> = devs.iter().map(|d| d.name.as_str()).collect();
            let _ = writeln!(o, "\n## 레이어 밖 맨 이동 키\n");
            let _ = writeln!(o, "| 키 | {} |", names.join(" | "));
            let _ = writeln!(o, "|---|{}", "---:|".repeat(devs.len()));
            for k in raw_keys {
                let cells: Vec<String> = devs
                    .iter()
                    .map(|d| {
                        d.report
                            .raw
                            .iter()
                            .find(|r| r.0 == k)
                            .map_or("0".into(), |r| r.1.to_string())
                    })
                    .collect();
                let _ = writeln!(o, "| {k} | {} |", cells.join(" | "));
            }
        }

        if self.total.total_holds() > 0 {
            let names: Vec<&str> = devs.iter().map(|d| d.name.as_str()).collect();
            let _ = writeln!(o, "\n## 트리거 홀드 시간 (그 기기 홀드 중 비율)\n");
            let _ = writeln!(o, "| 칸 | {} |", names.join(" | "));
            let _ = writeln!(o, "|---|{}", "---:|".repeat(devs.len()));
            for i in 0..HOLD_BUCKETS {
                if self.total.holds[i] == 0 {
                    continue;
                }
                let label = if i + 1 == HOLD_BUCKETS {
                    "1초 이상".to_string()
                } else {
                    format!("{}–{}ms", i * 100, (i + 1) * 100)
                };
                let cells: Vec<String> = devs
                    .iter()
                    .map(|d| {
                        let all = d.report.total_holds();
                        if all == 0 {
                            "—".into()
                        } else {
                            format!("{}%", d.report.holds[i] * 100 / all)
                        }
                    })
                    .collect();
                let _ = writeln!(o, "| {label} | {} |", cells.join(" | "));
            }
        }

        if !self.total.perf.is_empty() {
            let names: Vec<&str> = devs.iter().map(|d| d.name.as_str()).collect();
            let _ = writeln!(o, "\n## 단축키 실행 지연 (중앙값 / 90% · 횟수)\n");
            let _ = writeln!(o, "| 단계 | {} |", names.join(" | "));
            let _ = writeln!(o, "|---|{}", "---:|".repeat(devs.len()));
            for (phase, name) in PERF_PHASES {
                if self.total.perf_count(phase) == 0 {
                    continue;
                }
                let cells: Vec<String> = devs
                    .iter()
                    .map(|d| {
                        let r = &d.report;
                        match (r.perf_pct(phase, 50), r.perf_pct(phase, 90)) {
                            (Some(a), Some(b)) => format!(
                                "{} / {} · {}",
                                perf_label(a),
                                perf_label(b),
                                r.perf_count(phase)
                            ),
                            _ => "—".into(),
                        }
                    })
                    .collect();
                let _ = writeln!(o, "| {name} | {} |", cells.join(" | "));
            }
        }

        let _ = writeln!(o, "\n## 해석\n");
        for s in self.insights() {
            let _ = writeln!(o, "- {s}");
        }
        o
    }

    fn target_cell(&self, layer: &str, key: &str) -> String {
        let ts: Vec<(&str, &str)> = self
            .devices
            .iter()
            .filter_map(|d| d.target(layer, key).map(|t| (d.name.as_str(), t)))
            .collect();
        let uniq: BTreeSet<&str> = ts.iter().map(|x| x.1).collect();
        match uniq.len() {
            0 => "(배치에 없음)".into(),
            1 => uniq.into_iter().next().unwrap_or_default().to_string(),
            _ => ts
                .iter()
                .map(|(d, t)| format!("{d}: {t}"))
                .collect::<Vec<_>>()
                .join("; "),
        }
    }

    fn count_cells(&self, layer: &str, key: &str) -> Vec<String> {
        self.devices
            .iter()
            .map(|d| {
                let n = d.report.count_of(layer, key);
                if n == 0 && !d.maps(layer, key) {
                    return "—".into();
                }
                let all = d.report.total_moves().max(1);
                format!("{n} ({:.1}%)", n as f64 * 100.0 / all as f64)
            })
            .collect()
    }
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

    fn snap(device: &str, os: &str, rows: &[(&str, &str, u64)], km: &KeymapConfig) -> Snapshot {
        Snapshot {
            version: 1,
            device: device.into(),
            id: String::new(),
            os: os.into(),
            exported: "2026-10-24".into(),
            mapped: mapped_of(km)
                .into_iter()
                .map(|((l, k), t)| (l, k, t))
                .collect(),
            rows: rows
                .iter()
                .map(|(d, m, n)| (d.to_string(), m.to_string(), *n))
                .collect(),
        }
    }

    fn snap_days(
        device: &str,
        os: &str,
        rows: &[(&str, String, u64)],
        km: &KeymapConfig,
    ) -> Snapshot {
        let rows: Vec<(&str, &str, u64)> =
            rows.iter().map(|(d, m, n)| (*d, m.as_str(), *n)).collect();
        snap(device, os, &rows, km)
    }

    fn days(rows: &[(&str, u64)]) -> Vec<(&'static str, String, u64)> {
        // 같은 행을 사흘에 나눠 담는다 (판단 하한 3일)
        ["2026-10-22", "2026-10-23", "2026-10-24"]
            .iter()
            .flat_map(|d| rows.iter().map(move |(m, n)| (*d, m.to_string(), *n)))
            .collect()
    }

    #[test]
    fn 스냅숏은_기간을_자르고_기기_배치로_읽는다() {
        let s = snap(
            "mac",
            "macos",
            &[
                ("2026-09-01", "move:nav:H", 100),
                ("2026-10-23", "move:nav:H", 3),
                ("2026-10-24", "move:nav:H", 2),
                ("2026-10-24", "raw:Left", 1),
            ],
            &keymap(),
        );
        let r = s.report("2026-10-18", 7);
        assert_eq!(r.active_days, 2);
        assert_eq!(r.count_of("nav", "H"), 5, "기간 밖 행은 뺀다");
        assert_eq!(r.moves[0].target.as_deref(), Some("Left"));

        let dir = tempfile::tempdir().unwrap();
        let path = s.write_to(dir.path()).unwrap();
        assert!(path.ends_with("kmd-keystats-mac.json"));
        std::fs::write(dir.path().join("kmd-keystats-bad.json"), "{").unwrap();
        std::fs::write(dir.path().join("other.json"), "{}").unwrap();
        let (ok, bad) = Snapshot::load(dir.path());
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].rows.len(), 4);
        assert_eq!(bad.len(), 1, "이름이 맞는데 못 읽은 파일만 알린다");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("\"key\""), "횟수와 배치만 담긴다");
    }

    #[test]
    fn 기기를_나란히_놓고_차이를_읽는다() {
        let mac_km = keymap();
        let mut win_km = keymap();
        win_km
            .layers
            .get_mut("nav")
            .unwrap()
            .mappings
            .insert("I".into(), "Ctrl+Left".into());
        let mac_rows = days(&[
            ("move:nav:H", 40),
            ("move:nav:I", 30),
            ("raw:Left", 1),
            ("hold:1", 30),
        ]);
        let win_rows = days(&[
            ("move:nav:H", 50),
            ("move:nav:I", 0),
            ("raw:Left", 20),
            ("raw:Up", 5),
            ("hold:4", 30),
            ("idle:nav", 9),
        ]);
        let mac = snap_days("mac", "macos", &mac_rows, &mac_km);
        let win = snap_days("WIN", "windows", &win_rows, &win_km);
        let c = Compare::new(&[mac, win], "2026-10-18", 7);
        assert_eq!(c.total.count_of("nav", "H"), 270);
        assert_eq!(c.total.active_days, 3);
        let say = c.insights().join("\n");
        assert!(say.contains("맨 화살표: WIN (windows)"), "{say}");
        assert!(say.contains("nav I: mac"), "한쪽에서만 쓰는 수: {say}");
        assert!(
            say.contains("대상이 다르다(mac: Alt+Left, WIN: Ctrl+Left)"),
            "{say}"
        );
        assert!(
            say.contains("200ms 안에 놓은 홀드: mac 100% vs WIN 0%"),
            "{say}"
        );
        assert!(say.contains("WIN: 트리거를 쥐었다 그냥 놓음 30%"), "{say}");
        assert!(say.contains("대상이 다른 수 1개"), "{say}");
        let md = c.markdown();
        assert!(
            md.contains("| | mac (macos) | WIN (windows) | 합계 |"),
            "{md}"
        );
        assert!(md.contains("mac: Alt+Left; WIN: Ctrl+Left"), "{md}");
        assert!(md.contains("| Up | 0 | 15 |"), "{md}");
    }

    #[test]
    fn 표본이_적으면_판단을_보류한다() {
        let s = snap(
            "mac",
            "macos",
            &[("2026-10-24", "move:nav:H", 5)],
            &keymap(),
        );
        let c = Compare::new(&[s], "2026-10-18", 7);
        let say = c.insights().join("\n");
        assert!(say.contains("기기가 하나뿐"), "{say}");
        assert!(say.contains("표본이 적다"), "{say}");
        assert!(!say.contains("맨 화살표:"), "{say}");
    }

    #[test]
    fn 기기_이름은_파일에_쓸_수_있게() {
        assert!(!device_name().is_empty());
        assert_eq!(safe_name("내 맥.local"), "----local");
    }

    #[test]
    fn 실행_지연은_단계별_백분위로_읽는다() {
        assert_eq!(perf_bucket(0), 0);
        assert_eq!(perf_bucket(99), 2);
        assert_eq!(perf_bucket(100), 3);
        assert_eq!(perf_bucket(9999), PERF_BUCKETS - 1);
        assert_eq!(perf_label(2), "<100ms");
        assert_eq!(perf_label(PERF_BUCKETS - 1), "2초+");
        let rows = vec![
            ("perf:total:3".to_string(), 6),
            ("perf:total:7".to_string(), 4),
            ("perf:spawn:2".to_string(), 10),
            ("perf:window:6".to_string(), 10),
            ("perf:hookslow".to_string(), 3),
            ("tap:mouse".to_string(), 40),
        ];
        let r = KeyReport::build(&rows, 1, 7, &keymap());
        assert_eq!(r.perf_count("total"), 10);
        assert_eq!(r.perf_pct("total", 50), Some(3));
        assert_eq!(r.perf_pct("total", 90), Some(7));
        assert_eq!(r.slowest_phase(), Some(("window", 6)));
        assert_eq!(r.hook_slow, 3);
        assert_eq!(r.taps, vec![("mouse".to_string(), 40)]);
        assert_eq!(r.total_idle(), 0, "탭 동작은 스침이 아니다");
        let say = r.insights().join("\n");
        assert!(say.contains("중앙값 <150ms, 90% <800ms"), "{say}");
        assert!(say.contains("가장 늦은 단계는 창 준비"), "{say}");
        assert!(say.contains("20ms 넘게 걸린 일 3번"), "{say}");
    }

    #[test]
    fn 런처가_늦은_기기와_단계를_짚는다() {
        let km = keymap();
        let fast = days(&[
            ("move:nav:H", 50),
            ("perf:total:3", 2),
            ("perf:spawn:1", 2),
            ("perf:window:3", 2),
        ]);
        let slow = days(&[
            ("move:nav:H", 50),
            ("perf:total:7", 2),
            ("perf:spawn:3", 2),
            ("perf:window:7", 2),
        ]);
        let c = Compare::new(
            &[
                snap_days("mac", "macos", &fast, &km),
                snap_days("WIN", "windows", &slow, &km),
            ],
            "2026-10-18",
            7,
        );
        let say = c.insights().join("\n");
        assert!(
            say.contains("런처가 WIN 에서 늦게 뜬다 (중앙값 <800ms vs mac <150ms)"),
            "{say}"
        );
        assert!(say.contains("차이가 가장 큰 단계: 창 준비"), "{say}");
        let md = c.markdown();
        assert!(
            md.contains("| 실행 지연 중앙값 | <150ms | <800ms |"),
            "{md}"
        );
        assert!(
            md.contains("| 창 준비 | <150ms / <150ms · 6 | <800ms / <800ms · 6 |"),
            "{md}"
        );
    }

    #[test]
    fn 스냅숏은_launch_대상을_가리고_설치_id로_가른다() {
        let db = Database::open_in_memory().unwrap();
        let mut km = keymap();
        km.layers
            .get_mut("nav")
            .unwrap()
            .mappings
            .insert("Space".into(), "launch:/Users/me/secret.sh".into());
        let s = Snapshot::from_db(&db, "mac", "ab12", &km);
        assert!(s.mapped.iter().any(|m| m.1 == "Space" && m.2 == "launch"));
        assert!(!serde_json::to_string(&s).unwrap().contains("secret"));
        assert_eq!(s.file_name(), "kmd-keystats-mac-ab12.json");
        assert!(s.same_device("mac", "ab12"));
        assert!(
            !s.same_device("mac", "ffff"),
            "이름이 같아도 설치가 다르면 다른 기기"
        );

        let dir = tempfile::tempdir().unwrap();
        let id = device_id(dir.path());
        assert_eq!(id.len(), 8);
        assert_eq!(device_id(dir.path()), id, "한 번 만든 id 를 계속 쓴다");
    }

    #[test]
    fn 공유_폴더_경로는_절대_경로로_풀고_환경_변수는_거절한다() {
        assert!(expand_sync_path("%USERPROFILE%\\OneDrive").is_err());
        assert!(expand_sync_path("$HOME/x").is_err());
        assert!(expand_sync_path("  ").is_err());
        assert!(expand_sync_path("rel/dir").unwrap().is_absolute());
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_sync_path("~/kmd").unwrap(), home.join("kmd"));
    }
}
