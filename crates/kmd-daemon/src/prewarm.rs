//! 런처 예열 — 단축키를 누르기 전에 런처의 첫 실행 비용을 치른다 (docs/19 §7.4)
//!
//! 런처는 누를 때마다 새 프로세스다. 새 바이너리를 처음 실행하면 macOS 는 서명 검사(amfid)와
//! Gatekeeper 스캔을, Windows 는 백신 검사를 한다 — 맥 실측 0.4~0.6초(이후 5~14ms).
//! 디스크 캐시가 빈 직후에도 바이너리를 다시 읽는다. 그 비용을 사용자가 아니라 데몬이 치르도록
//! 데몬이 시작할 때(업데이트 직후 포함)와 절전에서 깬 직후 `kmd-desktop --version` 을 한 번 띄운다.
//! `--version` 은 창·GPU·로그 없이 버전만 찍고 끝난다.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::server::lower_indexer_thread_priority;

/// 데몬 시작 직후 — 훅·IPC 준비와 겹치지 않게 조금 기다린다
const START_DELAY: Duration = Duration::from_secs(3);
/// 절전 감지 주기
const POLL: Duration = Duration::from_secs(10);
/// 깨어 있던 시간보다 흐른 시간이 이만큼 더 길면 그 사이 잠들었던 것이다
const SLEEP_GAP_MS: u64 = 60_000;
/// 절전 복귀 예열 최소 간격 — 덮개를 닫아 둔 동안 macOS 는 화면 없이 수 분마다 잠깐 깬다
/// (DarkWake, 실측 하루 50~340회). 그 사이 캐시는 그대로라 매번 데울 필요가 없다.
const REWARM_MIN_MS: u64 = 30 * 60_000;

pub(crate) fn spawn(shutdown: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        lower_indexer_thread_priority();
        if !sleep_unless_shutdown(START_DELAY, &shutdown) {
            return;
        }
        warm("데몬 시작");
        let mut prev = clocks();
        let mut last_warm = prev.0;
        while sleep_unless_shutdown(POLL, &shutdown) {
            let now = clocks();
            if slept(prev, now) && due(last_warm, now.0) {
                warm("절전 복귀");
                last_warm = now.0;
            }
            prev = now;
        }
    });
}

/// 끝까지 잤으면 true, 도중에 종료 요청이 오면 false
fn sleep_unless_shutdown(total: Duration, shutdown: &AtomicBool) -> bool {
    let until = Instant::now() + total;
    while Instant::now() < until {
        if shutdown.load(Ordering::Relaxed) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(500).min(until - Instant::now()));
    }
    !shutdown.load(Ordering::Relaxed)
}

fn warm(why: &str) {
    use kmd_core::process::HideConsole;
    let exe = crate::keybind::resolve_launch_cmd("kmd-desktop");
    let started = Instant::now();
    let status = std::process::Command::new(&exe)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .hide_console()
        .status();
    match status {
        Ok(_) => tracing::info!("런처 예열({why}): {}ms", started.elapsed().as_millis()),
        Err(e) => tracing::debug!("런처 예열 생략({why}) — {exe}: {e}"),
    }
}

/// 마지막 예열(벽시계 ms) 뒤로 충분히 지났는지. 시계가 뒤로 가면 데운다.
fn due(last_wall_ms: u64, now_wall_ms: u64) -> bool {
    now_wall_ms < last_wall_ms || now_wall_ms - last_wall_ms >= REWARM_MIN_MS
}

/// (절전 포함 ms, 절전 제외 ms) — 둘의 차이가 벌어지면 잠들었던 것
type Clocks = (u64, u64);

fn slept(prev: Clocks, now: Clocks) -> bool {
    let with_sleep = now.0.saturating_sub(prev.0);
    let awake = now.1.saturating_sub(prev.1);
    with_sleep.saturating_sub(awake) > SLEEP_GAP_MS
}

/// macOS·Linux 의 `Instant` 는 절전 중 멈춘다(mach_absolute_time·CLOCK_MONOTONIC) —
/// 벽시계와 비교한다. 시계 조정으로 틀려도 예열 한 번 더 할 뿐이다.
#[cfg(not(windows))]
fn clocks() -> Clocks {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let awake = START.get_or_init(Instant::now).elapsed().as_millis() as u64;
    (wall, awake)
}

/// Windows 의 `Instant`(QPC)는 절전 시간을 포함하므로 전용 시계를 쓴다
#[cfg(windows)]
fn clocks() -> Clocks {
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    use windows_sys::Win32::System::WindowsProgramming::QueryUnbiasedInterruptTime;
    let mut unbiased = 0u64; // 100ns 단위, 절전 제외
                             // SAFETY: 지역 변수에 쓰는 시계 조회
    unsafe {
        QueryUnbiasedInterruptTime(&mut unbiased);
        (GetTickCount64(), unbiased / 10_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 깨어_있던_만큼만_흐르면_잠든_게_아니다() {
        assert!(!slept((1_000, 500), (11_000, 10_500)));
        // 조금의 어긋남은 무시
        assert!(!slept((1_000, 500), (40_000, 10_500)));
    }

    #[test]
    fn 흐른_시간이_1분_넘게_길면_잠들었던_것() {
        assert!(slept((1_000, 500), (3_600_000, 10_500)));
    }

    #[test]
    fn 잦은_짧은_깸에는_다시_데우지_않는다() {
        let t = 10 * 60_000;
        assert!(!due(t, t + 5 * 60_000));
        assert!(due(t, t + REWARM_MIN_MS));
        assert!(due(t, t - 1));
    }

    #[test]
    fn 시계가_뒤로_가도_잠든_것으로_보지_않는다() {
        assert!(!slept((10_000, 500), (1_000, 10_500)));
    }
}
