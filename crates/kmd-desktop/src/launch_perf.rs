//! 단축키 실행 지연 측정 (docs/19 §7)
//!
//! 데몬은 Launch 로 런처를 띄울 때 실행 시각을 `KMD_LAUNCH_AT_MS` 로 넘긴다.
//! 런처는 그 시각부터 프로세스가 시작될 때까지(프로세스 생성), 시작부터 창을 얻을
//! 때까지(창 준비), 시작부터 첫 검색 엔진이 붙을 때까지(검색 준비)를 잰다.
//! 손버릇 통계가 켜져 있으면 `perf:<단계>:<칸>` 횟수로 DB 에 더하고, 꺼져 있어도
//! desktop.log 에는 남긴다. 단축키로 띄운 경우(환경 변수가 있을 때)만 기록한다.

use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use kmd_core::keystats::{perf_bucket, LAUNCH_AT_ENV, LAUNCH_TRACE_ENV};

/// 이보다 긴 "프로세스 생성" 은 시계가 어긋났거나 남은 환경 변수다 — 버린다
const MAX_SPAWN_MS: u64 = 60_000;

struct Perf {
    start: Instant,
    /// 데몬 실행 → 이 프로세스 시작 (단축키로 띄웠을 때만)
    spawn_ms: Option<u64>,
    /// 데몬이 넘긴 단축키 경로 지연 (키→훅·훅→실행) — 사람이 읽는 한 줄
    trace: Option<String>,
    window_ms: Option<u64>,
    frame_logged: bool,
    engine_ms: Option<u64>,
    record: bool,
    done: bool,
}

static PERF: Mutex<Option<Perf>> = Mutex::new(None);

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// `main` 첫머리에서 부른다 — 시작 시각과 데몬이 넘긴 실행 시각을 읽는다
pub fn init() {
    let now = epoch_ms();
    let spawn_ms = std::env::var(LAUNCH_AT_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|at| now.saturating_sub(at))
        .filter(|&ms| ms <= MAX_SPAWN_MS);
    let trace = std::env::var(LAUNCH_TRACE_ENV).ok();
    // 런처가 띄우는 앱에 물려주지 않는다
    std::env::remove_var(LAUNCH_AT_ENV);
    std::env::remove_var(LAUNCH_TRACE_ENV);
    *lock() = Some(Perf {
        start: Instant::now(),
        spawn_ms,
        trace,
        window_ms: None,
        frame_logged: false,
        engine_ms: None,
        record: false,
        done: false,
    });
}

/// 설정을 읽은 뒤 — 손버릇 통계가 켜져 있을 때만 DB 에 남긴다
///
/// 로그가 준비된 뒤라 여기서 단축키 경로를 남긴다.
pub fn enable(stats: bool) {
    if let Some(p) = lock().as_mut() {
        p.record = stats;
        log_path_line(p, "창을 띄움");
    }
}

/// 이미 떠 있는 런처를 닫고 끝나는 실행 — 이 누름도 지연 기록에 남긴다.
/// 예전엔 창을 차지한 실행만 기록돼, 실행이 막혀 여러 번 누른 경우 늦게 누른 빠른 값만 남았다.
pub fn log_toggle_exit() {
    if let Some(p) = lock().as_ref() {
        log_path_line(p, "기존 창을 닫음");
    }
}

/// 단축키로 띄웠을 때만 — `단축키 경로: 키→훅 2ms · 훅→실행 1ms · … · 프로세스 생성 15ms (창을 띄움)`
fn log_path_line(p: &Perf, outcome: &str) {
    let Some(spawn) = p.spawn_ms else {
        return;
    };
    let trace = p.trace.as_deref().unwrap_or("데몬 기록 없음");
    tracing::info!("단축키 경로: {trace} · 프로세스 생성 {spawn}ms ({outcome})");
}

/// 창의 첫 프레임 — 창 id 이후에 실제로 그려지기까지 (한 번만 남긴다)
pub fn mark_frame() {
    let mut guard = lock();
    let Some(p) = guard.as_mut() else {
        return;
    };
    if p.frame_logged {
        return;
    }
    p.frame_logged = true;
    tracing::info!("첫 프레임 {}ms", p.start.elapsed().as_millis());
}

/// 창 id 를 얻었다
pub fn mark_window() {
    mark(|p, ms| p.window_ms.get_or_insert(ms));
}

/// 첫 검색 엔진(빠른 엔진 또는 전체 엔진)이 붙었다
pub fn mark_engine() {
    mark(|p, ms| p.engine_ms.get_or_insert(ms));
}

fn lock() -> std::sync::MutexGuard<'static, Option<Perf>> {
    PERF.lock().unwrap_or_else(|e| e.into_inner())
}

fn mark(set: impl FnOnce(&mut Perf, u64) -> &mut u64) {
    let mut guard = lock();
    let Some(p) = guard.as_mut() else {
        return;
    };
    if p.done {
        return;
    }
    let ms = p.start.elapsed().as_millis() as u64;
    set(p, ms);
    let (Some(window), Some(engine)) = (p.window_ms, p.engine_ms) else {
        return;
    };
    p.done = true;
    let rows = rows(p.spawn_ms, window, engine);
    tracing::info!(
        "실행 지연: 프로세스 생성 {} · 창 {window}ms · 검색 {engine}ms",
        p.spawn_ms
            .map_or("(단축키 아님)".to_string(), |ms| format!("{ms}ms"))
    );
    if !p.record || rows.is_empty() {
        return;
    }
    // UI 스레드를 막지 않는다
    std::thread::spawn(move || {
        match kmd_core::db::Database::open_default(kmd_core::db::DbRole::Shared) {
            Ok(db) => {
                if let Err(e) = db.key_stats_add(&rows) {
                    tracing::warn!("실행 지연 기록 실패: {e}");
                }
            }
            Err(e) => tracing::warn!("실행 지연 기록 DB 열기 실패: {e}"),
        }
    });
}

/// 단축키로 띄웠을 때만 — 단계별 칸 횟수
fn rows(spawn_ms: Option<u64>, window: u64, engine: u64) -> Vec<(String, u32)> {
    let Some(spawn) = spawn_ms else {
        return Vec::new();
    };
    let total = spawn + window.max(engine);
    [
        ("spawn", spawn),
        ("window", window),
        ("engine", engine),
        ("total", total),
    ]
    .into_iter()
    .map(|(phase, ms)| (format!("perf:{phase}:{}", perf_bucket(ms)), 1))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 단축키로_띄웠을_때만_단계를_남긴다() {
        assert!(rows(None, 100, 200).is_empty());
        let r = rows(Some(120), 300, 90);
        assert!(r.contains(&("perf:spawn:3".into(), 1)), "{r:?}"); // 100~150
        assert!(r.contains(&("perf:window:6".into(), 1)), "{r:?}"); // 300~500
        assert!(r.contains(&("perf:engine:2".into(), 1)), "{r:?}"); // 50~100
                                                                    // 전체 = 생성 + 창·검색 중 늦은 것 = 420
        assert!(r.contains(&("perf:total:6".into(), 1)), "{r:?}");
    }
}
