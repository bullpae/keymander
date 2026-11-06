//! 클립보드 히스토리 — 다중 버퍼 붙여넣기 (docs/12 P1)
//!
//! 데몬이 상주하며 시스템 클립보드 변화를 수집해 링 버퍼에 쌓고, 레이어
//! 바인딩(`clip:N`)이 n번째 최근 항목을 현재 전경 앱에 붙여넣는다.
//!
//! 히스토리가 데몬에 사는 이유: 클립보드 감시는 상주 프로세스만 가능하고
//! 상주 프로세스는 데몬뿐이다 (docs/11 R3 배경).
//!
//! 붙여넣기 전략 (설계 불변식):
//! - **Windows**: 시스템 클립보드를 **변형하지 않는다** — 항목 텍스트를
//!   SendInput KEYEVENTF_UNICODE 타이핑으로 직접 주입한다. 임시 교체·복원이
//!   없으므로 "우리 값" 소유권 식별 레이스, 다중 SetClipboardData 부분 실패,
//!   stale 스냅샷 복원이 **원천 불가능**하다 (근거: winclip.rs 모듈 문서).
//! - **macOS/기타**: 텍스트 스냅샷 → 교체 → Cmd+V 주입 → "우리 값이 아직
//!   현재일 때만" 복원 (기존 계약 유지).
//!
//! 프라이버시 (키 훅 프로그램의 의무):
//! - **메모리에만** 산다. 디스크 비저장.
//! - 1MB 초과 텍스트는 수집 제외 (메모리 보호).
//! - 내용은 로그에 절대 남기지 않는다 (keycode 불로깅 원칙과 동일).
//! - **비밀번호 제외**: macOS는 `org.nspasteboard.ConcealedType` 마크(1Password
//!   등 비번 관리자 관례), Windows는 `ExcludeClipboardContentFromMonitorProcessing`
//!   등 수집 제외 마커가 붙은 항목을 수집하지 않는다 (P1.1).
//!
//! 항목 식별: 슬롯 번호(1-기반, 최신순)는 새 복사가 들어올 때마다 밀리므로
//! 검색→선택 사이에 다른 항목을 가리킬 수 있다. 각 항목은 데몬 수명 동안
//! 변하지 않는 안정 ID를 갖고, 런처 선택은 ID로 붙여넣는다 (`paste_item`).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// 수집 상한 (텍스트 1건). 초과 시 수집하지 않는다.
const MAX_ITEM_BYTES: usize = 1024 * 1024;
/// 감시 폴링 주기. macOS changeCount / Windows sequence number만 읽는 값싼
/// 폴링이라 유휴 비용은 무시 가능한 수준 — 클립보드는 변화가 있을 때만 연다.
const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// 붙여넣기 주입 후 원래 클립보드를 복원하기까지의 지연 (클립보드 스왑 경로).
/// 대상 앱이 Cmd+V를 처리할 시간을 준다 (클립보드 관리자 표준 기법).
#[cfg(not(windows))]
const RESTORE_DELAY: Duration = Duration::from_millis(300);

/// 히스토리 항목. `id`는 데몬 수명 동안 안정적 — 같은 내용을 다시 복사해
/// 맨 앞으로 이동해도 유지된다 (런처가 선택 시점의 항목을 정확히 집는 근거).
#[derive(Debug, Clone)]
struct ClipEntry {
    id: u64,
    text: String,
}

/// 히스토리 링 버퍼. 맨 앞(index 0)이 최신.
static HISTORY: Mutex<VecDeque<ClipEntry>> = Mutex::new(VecDeque::new());
/// 안정 ID 발급기. 0은 "ID 없음"(구버전 프로토콜 폴백)으로 예약.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// 붙여넣기 직렬화 — 두 요청이 스냅샷·복원(스왑 경로)이나 타이핑 주입을
/// 교차해 출력을 섞지 않게 한다 (IPC 커넥션 스레드가 여러 개다).
static PASTE_LOCK: Mutex<()> = Mutex::new(());
/// 감시 스레드 중복 기동 방지.
static WATCHER_STARTED: AtomicBool = AtomicBool::new(false);

/// 현재 링 버퍼 상한 (기본 50). spawn_watcher에서 설정.
static CAP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(50);

/// 새 클립보드 텍스트를 히스토리 맨 앞에 넣는다 (중복은 앞으로 이동, 상한 유지).
/// 수집 경로와 테스트에서 공용으로 쓴다.
fn record(text: String) {
    if text.is_empty() || text.len() > MAX_ITEM_BYTES {
        return;
    }
    let cap = CAP.load(Ordering::Relaxed).max(1);
    if let Ok(mut h) = HISTORY.lock() {
        // 같은 내용은 중복 생성 없이 맨 앞으로 이동 (dedupe) — ID는 유지한다.
        let entry = if let Some(pos) = h.iter().position(|e| e.text == text) {
            h.remove(pos).expect("position으로 찾은 항목")
        } else {
            ClipEntry {
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                text,
            }
        };
        h.push_front(entry);
        while h.len() > cap {
            h.pop_back();
        }
    }
}

/// 안정 ID로 항목 조회. 밀려났거나 모르는 ID면 None.
fn entry_by_id(id: u64) -> Option<ClipEntry> {
    HISTORY.lock().ok()?.iter().find(|e| e.id == id).cloned()
}

/// n번째 최근 항목(1-기반) 조회. 없으면 None.
fn entry_by_slot(n: usize) -> Option<ClipEntry> {
    if n == 0 {
        return None;
    }
    HISTORY.lock().ok()?.get(n - 1).cloned()
}

/// n번째 최근 항목 텍스트 (테스트용).
#[cfg(test)]
fn slot(n: usize) -> Option<String> {
    entry_by_slot(n).map(|e| e.text)
}

/// n번째 최근 항목의 안정 ID (테스트용).
#[cfg(test)]
fn slot_id(n: usize) -> Option<u64> {
    entry_by_slot(n).map(|e| e.id)
}

/// 현재 히스토리 길이 (진단/테스트용).
pub fn len() -> usize {
    HISTORY.lock().map(|h| h.len()).unwrap_or(0)
}

// ── 감시 상태 기계 (순수 로직 — 레이스 회귀를 실기 없이 테스트) ─────────────

/// 감시 스레드의 지속 상태. 전이는 [`watch_gate`]/[`watch_transition`]만 한다.
struct WatchState {
    /// 마지막으로 **소비한** 세대 번호. Busy(일시 잠김)는 소비하지 않아
    /// 다음 틱에 같은 변화를 재시도한다 (복사 유실 방지).
    last_generation: isize,
    /// 마지막으로 기록한 텍스트 (세대 미지원 플랫폼의 diff 폴백 + dedupe).
    last_text: String,
}

/// 클립보드를 **열기 전** 게이트: 스왑 주입 중이거나 세대가 그대로면 열지
/// 않는다. 세대 0은 미지원 폴백 — 항상 열어 diff로 판정한다.
fn watch_gate(suppressed: bool, generation: isize, last_generation: isize) -> bool {
    !suppressed && (generation == 0 || generation != last_generation)
}

/// 게이트 통과 후 클립보드에서 관측한 결과.
enum WatchRead {
    /// 비밀번호 관리자 등이 수집 제외로 표시 — 확정 비수집.
    Concealed,
    /// 텍스트 읽기 성공.
    Text(String),
    /// 다른 앱이 클립보드를 잠깐 쥐고 있는 일시 오류 (arboard Occupied).
    Busy,
    /// 텍스트가 없는 클립보드(이미지 등) 등 확정 실패.
    NoText,
}

/// 관측 결과 하나로 상태를 전이하고, 기록할 텍스트를 돌려준다 (순수).
/// Busy만 세대를 소비하지 않는다 — 나머지는 확정 판정이라 소비해, 같은
/// 내용에 매 폴링 클립보드를 다시 열지 않는다.
fn watch_transition(state: &mut WatchState, generation: isize, read: WatchRead) -> Option<String> {
    match read {
        WatchRead::Busy => None,
        WatchRead::Concealed | WatchRead::NoText => {
            state.last_generation = generation;
            None
        }
        WatchRead::Text(text) => {
            state.last_generation = generation;
            if !text.is_empty() && text != state.last_text {
                state.last_text = text.clone();
                Some(text)
            } else {
                None
            }
        }
    }
}

/// 클립보드 감시 스레드 시작 — 변화가 있을 때만 읽어 수집한다.
/// `enabled`가 false면 아무것도 하지 않는다 (opt-in). 중복 호출은 무시.
pub fn spawn_watcher(enabled: bool, history_size: usize) {
    CAP.store(history_size.clamp(1, 1000), Ordering::Relaxed);
    if !enabled {
        tracing::info!("클립보드 히스토리 수집: 비활성 (clipboard.history_enabled=false)");
        return;
    }
    if WATCHER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let mut board = match arboard::Clipboard::new() {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("클립보드 감시 시작 실패: {e}");
                WATCHER_STARTED.store(false, Ordering::SeqCst);
                return;
            }
        };
        // 세대 번호(macOS changeCount / Windows GetClipboardSequenceNumber)로
        // 실제 변화만 감지한다 — get_text를 매 폴링마다 부르면 클립보드를 계속
        // 열어 다른 클립보드 관리자·복사 중인 앱과 경합하고 비용도 크다.
        // 세대 번호가 없는 플랫폼(change_count=0)은 get_text diff로 폴백한다.
        let mut state = WatchState {
            last_generation: platform::change_count(),
            last_text: String::new(),
        };
        if !platform::is_concealed() {
            if let Ok(t) = board.get_text() {
                record(t.clone()); // 시작 시점 클립보드는 "직전 항목"
                state.last_text = t;
            }
        }
        tracing::info!(
            "클립보드 히스토리 수집 시작 (상한 {})",
            CAP.load(Ordering::Relaxed)
        );

        loop {
            std::thread::sleep(POLL_INTERVAL);
            let generation = platform::change_count();
            if !watch_gate(
                SUPPRESS_CAPTURE.load(Ordering::Relaxed),
                generation,
                state.last_generation,
            ) {
                continue;
            }
            let read = if platform::is_concealed() {
                WatchRead::Concealed
            } else {
                match board.get_text() {
                    Ok(text) => WatchRead::Text(text),
                    Err(arboard::Error::ClipboardOccupied) => WatchRead::Busy,
                    Err(_) => WatchRead::NoText,
                }
            };
            if let Some(text) = watch_transition(&mut state, generation, read) {
                record(text);
            }
        }
    });
}

/// 스왑 주입(비-Windows) 동안 수집을 멈추는 플래그 — 슬롯 스왑이 새 복사로
/// 잡히지 않게. Windows 타이핑 주입은 클립보드를 안 바꾸므로 세울 일이 없다.
static SUPPRESS_CAPTURE: AtomicBool = AtomicBool::new(false);

/// 런처를 열기 직전의 전경 앱 대상 (macOS PID / Windows HWND, docs/12 흐름 B).
/// 0 = 없음. 런처가 포커스를 뺏기 전에 데몬이 스냅샷한다.
static PREV_APP_TARGET: AtomicIsize = AtomicIsize::new(0);

/// Launch 바인딩이 전경 앱 캡처를 동반해야 하는지 — 데스크톱 런처(`kmd-desktop`)
/// 실행만 해당한다. 다른 launch 바인딩(브라우저·에디터 등)이 캡처하면 런처 사용
/// 중 흐름 B의 붙여넣기 대상이 엉뚱한 앱으로 덮어써진다.
pub fn launch_captures_foreground(cmd: &str) -> bool {
    crate::keybind::launch_parts(cmd).0 == "kmd-desktop"
}

/// 런처(kmd-desktop)를 띄우기 직전에 호출 — 현재 전경 앱을 기억한다.
/// Launch 액션 경로에서 불러, 전역 핫키/레이어 어느 쪽으로 열든 캡처된다.
pub fn capture_foreground_app() {
    let target = platform::frontmost_target();
    if target != 0 {
        PREV_APP_TARGET.store(target, Ordering::Relaxed);
    }
}

/// 히스토리를 쿼리로 필터링해 돌려준다 (흐름 B 런처 검색).
/// 빈 쿼리는 전체(최신순). 대소문자 무시 부분일치 — 데몬은 후보만 넘기고
/// 정교한 랭킹은 UI(Nucleo)가 한다. 실행은 안정 ID(`ClipHit.id`)로 한다.
pub fn search(query: &str, limit: usize) -> Vec<kmd_core::ipc::ClipHit> {
    let q = query.trim().to_lowercase();
    let Ok(h) = HISTORY.lock() else {
        return Vec::new();
    };
    h.iter()
        .enumerate()
        .filter(|(_, entry)| q.is_empty() || entry.text.to_lowercase().contains(&q))
        .take(limit)
        .map(|(i, entry)| kmd_core::ipc::ClipHit {
            id: entry.id,
            slot: i + 1,
            preview: preview_line(&entry.text),
        })
        .collect()
}

/// 첫 줄 + 길이 제한 미리보기 (UI 표시용, 내용 로그엔 안 남김).
fn preview_line(text: &str) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    if first.chars().count() > 200 {
        first.chars().take(200).collect::<String>() + "…"
    } else {
        first.to_string()
    }
}

/// n번째 최근 항목을 **현재 전경 앱**에 붙여넣는다 (docs/12 흐름 A — 레이어).
/// 키 훅 경로 전용 — 실패는 로그로만 남긴다 (IPC 경로는 checked 계열 사용).
pub fn paste_slot(n: usize) {
    if let Err(e) = paste_slot_checked(n, false) {
        tracing::warn!("클립보드 슬롯 {n} 붙여넣기 실패: {e}");
    }
}

/// n번째 최근 항목(1-기반)을 붙여넣는다. `to_previous`면 런처 열기 전 앱으로
/// 포커스를 되돌린 뒤 주입한다 (docs/12 흐름 B). 빈 슬롯/실패는 Err.
pub fn paste_slot_checked(n: usize, to_previous: bool) -> Result<(), String> {
    let entry = entry_by_slot(n).ok_or_else(|| format!("클립보드 슬롯 {n}이 비어 있습니다"))?;
    paste_impl(entry, to_previous)
}

/// 안정 ID로 항목을 붙여넣는다 (런처 검색 결과 선택 경로). 검색 이후 새 복사가
/// 들어와 슬롯이 밀려도 같은 항목을 집는다. 만료/실패는 Err.
pub fn paste_item(id: u64, to_previous: bool) -> Result<(), String> {
    let entry =
        entry_by_id(id).ok_or_else(|| "선택한 클립보드 항목이 만료되었습니다".to_string())?;
    paste_impl(entry, to_previous)
}

/// 안정 ID의 항목을 붙여넣지 않고 **시스템 클립보드에 복사만** 한다
/// (런처 Cmd/Ctrl+Enter). 만료/실패는 Err.
///
/// 모든 플랫폼에서 클립보드를 실제로 교체한다 — Windows의 "붙여넣기는 클립보드
/// 무변형" 불변식과 어긋나지 않는다: 복사 전용은 사용자가 "이 항목으로 클립보드를
/// 바꿔 달라"고 명시한 동작이기 때문이다. 수집은 막지 않는다 — 감시 스레드가
/// 이 복사를 관측하면 dedupe로 같은 항목(같은 ID)이 히스토리 맨 앞으로 온다.
pub fn copy_item(id: u64) -> Result<(), String> {
    let entry =
        entry_by_id(id).ok_or_else(|| "선택한 클립보드 항목이 만료되었습니다".to_string())?;
    let _guard = PASTE_LOCK
        .lock()
        .map_err(|_| "클립보드 동작 잠금이 손상되었습니다".to_string())?;
    let mut board = arboard::Clipboard::new().map_err(|e| format!("클립보드 접근 실패: {e}"))?;
    board
        .set_text(entry.text)
        .map_err(|e| format!("클립보드 쓰기 실패: {e}"))
}

/// 복원 판단 (클립보드 스왑 경로): 우리가 넣은 임시 값이 **여전히 현재
/// 클립보드일 때만** 복원한다. 그 사이 사용자/다른 앱이 복사했다면 덮어쓰지
/// 않는다 (사용자 데이터 보호). 세대 번호가 있으면(swap≠0) 세대 비교,
/// 없으면 텍스트 일치로 판정한다.
#[cfg(not(windows))]
fn should_restore(
    swap_generation: isize,
    current_generation: isize,
    injected_text_matches: bool,
) -> bool {
    if swap_generation != 0 {
        current_generation == swap_generation
    } else {
        injected_text_matches
    }
}

/// 슬롯/ID 붙여넣기 공통 구현.
/// `to_previous`면 먼저 이전 전경 앱으로 포커스를 되돌린다.
///
/// 플랫폼 전략 (모듈 문서의 설계 불변식):
/// - Windows: 클립보드 무변형 타이핑 주입 — `platform::paste_text`.
/// - macOS/기타: 스냅샷(실패 시 **중단** — 원본을 파괴하지 않는다) → 교체 →
///   Cmd+V → 우리 값이 아직 현재일 때만 복원. 스왑 구간엔 수집을 멈춘다.
///
/// **액션 워커/커넥션 스레드에서 호출** — macOS는 RESTORE_DELAY(300ms)만큼
/// 블로킹하고, Windows는 단일 SendInput 호출이라 사실상 즉시 반환한다.
fn paste_impl(entry: ClipEntry, to_previous: bool) -> Result<(), String> {
    let _guard = PASTE_LOCK
        .lock()
        .map_err(|_| "클립보드 동작 잠금이 손상되었습니다".to_string())?;

    #[cfg(windows)]
    let target_still_foreground = platform::is_target_foreground;
    // macOS의 기존 활성화 계약은 유지하고, Windows에서만 HWND를 재검증한다.
    #[cfg(not(windows))]
    let target_still_foreground = |_target| true;

    paste_after_target_activation(
        to_previous,
        PREV_APP_TARGET.load(Ordering::Relaxed),
        platform::activate_target,
        || {
            // 앱 전환이 반영될 시간을 준다 (포커스가 도착해야 주입이 대상에 간다).
            std::thread::sleep(Duration::from_millis(120));
        },
        target_still_foreground,
        || {
            #[cfg(windows)]
            {
                platform::paste_text(&entry.text)
            }
            #[cfg(not(windows))]
            {
                paste_via_swap(&entry)
            }
        },
    )
}

/// 이전 앱 대상 붙여넣기의 안전 경계. 대상이 없거나 활성화가 실패하면 현재
/// 포커스에는 절대 주입하지 않고 즉시 Err를 반환한다.
///
/// 활성화·대기·주입을 콜백으로 받아 OS 포커스 없이도 순서와 fail-closed 계약을
/// 결정적으로 검증한다.
fn paste_after_target_activation(
    to_previous: bool,
    target: isize,
    activate: impl FnOnce(isize) -> bool,
    wait_for_focus: impl FnOnce(),
    target_still_foreground: impl FnOnce(isize) -> bool,
    paste: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if to_previous {
        if target == 0 {
            return Err(
                "런처를 열기 전 앱을 찾지 못해 붙여넣기를 중단했습니다 — 현재 포커스에는 주입하지 않았습니다"
                    .to_string(),
            );
        }
        if !activate(target) {
            return Err(
                "런처를 열기 전 앱을 활성화하지 못해 붙여넣기를 중단했습니다 — 현재 포커스에는 주입하지 않았습니다"
                    .to_string(),
            );
        }
        wait_for_focus();
        // Windows에서는 대기 중 다른 창이 포커스를 훔칠 수 있다. 실제 Unicode
        // SendInput 직전에 캡처한 HWND가 여전히 전경인지 확인해 fail-closed한다.
        if !target_still_foreground(target) {
            return Err(
                "대기 중 포커스가 다른 창으로 바뀌어 붙여넣기를 중단했습니다 — 현재 포커스에는 주입하지 않았습니다"
                    .to_string(),
            );
        }
    }
    paste()
}

/// 클립보드 스왑 경로 (비-Windows): 스냅샷 실패 = 원본을 손실 없이 복원할 수
/// 없음 → 붙여넣기 자체를 포기한다. (사용자의 클립보드가 그대로 남으므로
/// 수동 Cmd+V로 항상 복구 가능.)
#[cfg(not(windows))]
fn paste_via_swap(entry: &ClipEntry) -> Result<(), String> {
    let saved = platform::snapshot().map_err(|e| format!("클립보드 백업 실패로 중단: {e}"))?;
    let mut board = arboard::Clipboard::new().map_err(|e| format!("클립보드 접근 실패: {e}"))?;

    SUPPRESS_CAPTURE.store(true, Ordering::Relaxed);
    let result = paste_swap(&mut board, entry, saved);
    SUPPRESS_CAPTURE.store(false, Ordering::Relaxed);
    result
}

/// 스왑→주입→조건부 복원. SUPPRESS_CAPTURE 해제를 위해 paste_via_swap에서 분리.
#[cfg(not(windows))]
fn paste_swap(
    board: &mut arboard::Clipboard,
    entry: &ClipEntry,
    saved: platform::Snapshot,
) -> Result<(), String> {
    // set 실패 = 백엔드가 empty 후 set 하다 실패해 원본이 이미 부분 파괴됐을 수
    // 있다 — 스냅샷으로 즉시 되돌리고 결과를 함께 보고한다 (실패 원자성).
    if let Err(e) = board.set_text(&entry.text) {
        return Err(swap_failure_message(
            &e.to_string(),
            platform::restore(saved),
        ));
    }
    // 우리 스왑 직후의 세대 번호 — 복원 시점에 이 값이면 "그 사이 아무도 안 씀".
    let swap_generation = platform::change_count();
    // 세팅이 시스템에 반영될 짧은 시간을 준 뒤 주입한다.
    std::thread::sleep(Duration::from_millis(30));
    crate::keybind::inject_paste();

    std::thread::sleep(RESTORE_DELAY);
    let injected_text_matches =
        swap_generation == 0 && board.get_text().ok().as_deref() == Some(entry.text.as_str());
    if should_restore(
        swap_generation,
        platform::change_count(),
        injected_text_matches,
    ) {
        platform::restore(saved).map_err(|e| format!("기존 클립보드 복원 실패: {e}"))?;
    } else {
        tracing::info!("붙여넣기 후 클립보드가 이미 바뀜 — 복원 생략 (새 복사 보존)");
    }
    Ok(())
}

/// set 실패 시 원본 복원까지 시도한 결과를 하나의 에러로 합친다 — 호출자/사용자가
/// "내 클립보드 원본이 살아있는가"를 알 수 있어야 한다 (실패 원자성 보고).
#[cfg(not(windows))]
fn swap_failure_message(set_error: &str, restore_result: Result<(), String>) -> String {
    match restore_result {
        Ok(()) => format!("클립보드 세팅 실패: {set_error} (원본은 복원됨)"),
        Err(restore_error) => {
            format!("클립보드 세팅 실패: {set_error}; 원본 복원도 실패: {restore_error}")
        }
    }
}

// ── macOS NSPasteboard ──────────────────────────────────────────────────────
//
// changeCount: 클립보드가 바뀔 때마다 증가하는 정수 — 실제 변화만 싸게 감지.
// Concealed: `org.nspasteboard.ConcealedType` 마크(비번 관리자 관례)가 있으면
// 민감 항목이므로 수집하지 않는다. NSPasteboard는 스레드 제약이 없어 감시
// 스레드에서 직접 호출해도 된다.
#[cfg(target_os = "macos")]
mod platform {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    /// 텍스트 스냅샷 (기존 macOS 계약 유지 — 비텍스트 복원은 docs/12 후속).
    #[derive(Debug)]
    pub struct Snapshot {
        text: Option<String>,
    }

    /// 현재 클립보드 백업. 텍스트가 아니면 None 스냅샷 (복원 시 no-op).
    pub fn snapshot() -> Result<Snapshot, String> {
        let mut board = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        Ok(Snapshot {
            text: board.get_text().ok(),
        })
    }

    /// 백업 복원. None 스냅샷은 아무것도 하지 않는다 (기존 동작과 동일).
    /// NSPasteboard엔 잠금이 없어 잠금 후 재검증이 불가능하다 — 호출부의
    /// should_restore 사전 판정이 전부다.
    pub fn restore(snapshot: Snapshot) -> Result<(), String> {
        if let Some(text) = snapshot.text {
            let mut board = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            board.set_text(text).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// `[[NSPasteboard generalPasteboard] changeCount]`. 실패 시 0.
    pub fn change_count() -> isize {
        unsafe {
            let Some(cls) = AnyClass::get(c"NSPasteboard") else {
                return 0;
            };
            let pb: *mut AnyObject = msg_send![cls, generalPasteboard];
            if pb.is_null() {
                return 0;
            }
            msg_send![pb, changeCount]
        }
    }

    /// 현재 클립보드에 Concealed(비밀번호) 마크가 있으면 true.
    /// `[pb availableTypeFromArray:@[@"org.nspasteboard.ConcealedType"]]` != nil.
    pub fn is_concealed() -> bool {
        unsafe {
            let (Some(pb_cls), Some(str_cls), Some(arr_cls)) = (
                AnyClass::get(c"NSPasteboard"),
                AnyClass::get(c"NSString"),
                AnyClass::get(c"NSArray"),
            ) else {
                return false;
            };
            let pb: *mut AnyObject = msg_send![pb_cls, generalPasteboard];
            if pb.is_null() {
                return false;
            }
            let ty: *mut AnyObject = msg_send![str_cls, stringWithUTF8String: c"org.nspasteboard.ConcealedType".as_ptr()];
            if ty.is_null() {
                return false;
            }
            let arr: *mut AnyObject = msg_send![arr_cls, arrayWithObject: ty];
            if arr.is_null() {
                return false;
            }
            let found: *mut AnyObject = msg_send![pb, availableTypeFromArray: arr];
            !found.is_null()
        }
    }

    /// 현재 전경 앱의 PID. `[[NSWorkspace sharedWorkspace] frontmostApplication]`.
    /// 없으면 0. (docs/12 흐름 B — 런처가 포커스를 뺏기 전에 캡처)
    pub fn frontmost_target() -> isize {
        unsafe {
            let Some(ws_cls) = AnyClass::get(c"NSWorkspace") else {
                return 0;
            };
            let ws: *mut AnyObject = msg_send![ws_cls, sharedWorkspace];
            if ws.is_null() {
                return 0;
            }
            let app: *mut AnyObject = msg_send![ws, frontmostApplication];
            if app.is_null() {
                return 0;
            }
            let pid: i32 = msg_send![app, processIdentifier];
            pid as isize
        }
    }

    /// PID로 앱을 전면 활성화. 성공하면 true.
    /// `[[NSRunningApplication runningApplicationWithProcessIdentifier:] activateWithOptions:]`
    pub fn activate_target(target: isize) -> bool {
        // NSApplicationActivateIgnoringOtherApps
        const ACTIVATE_IGNORING_OTHER_APPS: usize = 1 << 1;
        unsafe {
            let Some(cls) = AnyClass::get(c"NSRunningApplication") else {
                return false;
            };
            let app: *mut AnyObject =
                msg_send![cls, runningApplicationWithProcessIdentifier: target as i32];
            if app.is_null() {
                return false;
            }
            let ok: bool = msg_send![app, activateWithOptions: ACTIVATE_IGNORING_OTHER_APPS];
            ok
        }
    }
}

// ── Windows ─────────────────────────────────────────────────────────────────
//
// 감시는 GetClipboardSequenceNumber(클립보드를 열지 않는 세대 번호)로 변화를
// 감지하고, 변화가 있을 때만 연다 (읽기 전용). 붙여넣기는 클립보드를 **읽지도
// 쓰지도 않는다** — SendInput KEYEVENTF_UNICODE 타이핑 주입 (설계 불변식은
// winclip.rs 모듈 문서). 스냅샷/복원 경로가 없으므로 SetClipboardData·
// HGLOBAL 수명·WM_DESTROYCLIPBOARD 관련 코드도 존재하지 않는다.
#[cfg(windows)]
mod platform {
    use crate::winclip;
    use clipboard_win::{raw, Clipboard};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_KEYBOARD, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    };

    /// GetClipboardSequenceNumber — 클립보드를 열지 않고 읽는 세대 번호.
    /// 권한 없는 세션 등 실패 시 0 (호출부가 텍스트 diff로 폴백).
    pub fn change_count() -> isize {
        raw::seq_num().map_or(0, |n| n.get() as isize)
    }

    /// 수집 제외 마커 판정. 존재 확인은 클립보드를 열지 않고 하고
    /// (IsClipboardFormatAvailable), 값 확인이 필요할 때만 연다.
    /// 열기/읽기 실패 시 **보수적으로 제외**(true) — 민감할 수 있는 항목을
    /// 불확실한 상태에서 수집하지 않는다.
    pub fn is_concealed() -> bool {
        if let Some(f) = clipboard_win::register_format(winclip::MARKER_EXCLUDE) {
            if clipboard_win::is_format_avail(f.get()) {
                return true;
            }
        }
        let value_markers: Vec<(&str, u32)> = winclip::MARKERS_OPT_OUT_ZERO
            .iter()
            .filter_map(|name| clipboard_win::register_format(name).map(|f| (*name, f.get())))
            .filter(|(_, f)| clipboard_win::is_format_avail(*f))
            .collect();
        if value_markers.is_empty() {
            return false;
        }
        let Ok(_clip) = Clipboard::new_attempts(5) else {
            return true;
        };
        for (name, format) in value_markers {
            let mut data = Vec::new();
            match raw::get_vec(format, &mut data) {
                // 판정은 winclip::format_is_sensitive 단일 정책을 따른다.
                Ok(_) if data.len() >= 4 && !winclip::format_is_sensitive(name, &data) => {}
                // 값이 0(제외) 또는 못 읽음/형식 이상 → 보수적으로 제외
                _ => return true,
            }
        }
        false
    }

    /// 현재 전경 창 HWND (docs/12 흐름 B — 런처가 포커스를 뺏기 전에 캡처).
    pub fn frontmost_target() -> isize {
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() as isize }
    }

    /// HWND로 창을 전면 활성화. 성공하면 true.
    pub fn activate_target(target: isize) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            IsIconic, IsWindow, SetForegroundWindow, ShowWindow, SW_RESTORE,
        };
        let hwnd = target as windows_sys::Win32::Foundation::HWND;
        unsafe {
            if hwnd.is_null() || IsWindow(hwnd) == 0 {
                return false;
            }
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd) != 0
        }
    }

    /// 활성화 대기 직후에도 캡처한 HWND가 전경인지 확인한다.
    pub fn is_target_foreground(target: isize) -> bool {
        let hwnd = target as windows_sys::Win32::Foundation::HWND;
        !hwnd.is_null()
            && unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() == hwnd }
    }

    /// 항목 텍스트를 KEYEVENTF_UNICODE 타이핑으로 전경 앱에 직접 주입한다.
    ///
    /// 시스템 클립보드는 읽지도 쓰지도 않는다 — 원본(이미지·파일·커스텀 포맷
    /// 전부)이 그대로 남아 부분 소실·소유권 레이스가 원천 불가능하다.
    /// 우리 훅은 LLKHF_INJECTED로 주입 이벤트를 통과시키므로 되먹임도 없다.
    /// 단일 행·길이·호환 한계는 `winclip::encode_for_injection` 문서 참고.
    ///
    /// 전체 down/up 이벤트를 **단 한 번의 SendInput 호출**로 보낸다 — SendInput은
    /// 호출 단위로 입력 스트림에 원자적으로 들어가므로, 호출을 쪼개면 생기는
    /// 물리 키 입력 interleave 창(반쪽 서러게이트·문자 사이 끼어들기)이 없다.
    /// 호출 크기는 `winclip::MAX_INJECT_UTF16_UNITS`(4096유닛 = 이벤트 8192개)로
    /// 제한되어 입력 큐를 점령하지 않는다.
    pub fn paste_text(text: &str) -> Result<(), String> {
        let units = winclip::encode_for_injection(text)?;
        if units.is_empty() {
            return Ok(());
        }
        let inputs = build_unicode_inputs(&units);
        // SendInput은 부분 실패 시에만 last-error를 세팅한다 — 이전 호출의
        // stale 값을 오류 보고에 싣지 않도록 먼저 0으로 지운다.
        unsafe { windows_sys::Win32::Foundation::SetLastError(0) };
        let sent = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
        if sent == inputs.len() as u32 {
            return Ok(());
        }
        // 부분 전송 (UIPI 차단, 다른 스레드의 입력 큐 잠금 등). 이미 들어간
        // 문자는 회수 불가능하고, 나머지를 재시도하면 어디까지 전달됐는지
        // 알 수 없어 중복 입력이 된다 — 재시도하지 않는다.
        let last_error = unsafe { windows_sys::Win32::Foundation::GetLastError() };
        let cleanup = match winclip::dangling_down_unit(&units, sent as usize) {
            Some(unit) => {
                // 홀수에서 끊기면 마지막 이벤트가 짝 없는 key-down이다. 대응
                // key-up을 즉시 시도하고, 실패하면 키 훅 워치독의 기존 pending
                // release 큐에 등록해 성공할 때까지 제한적으로 재시도한다.
                let cleaned = crate::keybind::windows::release_unicode_unit_or_track(unit);
                if cleaned {
                    "; 짝 잃은 마지막 key-down은 key-up으로 정리했습니다"
                } else {
                    "; 짝 잃은 마지막 key-down의 key-up 정리 전송도 실패해 재시도 큐에 등록했습니다"
                }
            }
            None => "",
        };
        Err(format!(
            "유니코드 주입이 이벤트 {}개 중 {sent}개에서 중단됐습니다 \
             (GetLastError={last_error}; 대상 창이 더 높은 무결성 수준이면 UIPI 차단일 수 \
             있습니다){cleanup}. 이미 입력된 문자는 회수할 수 없고 중복 입력을 막기 위해 \
             재시도하지 않습니다 — 클립보드는 변경되지 않았습니다",
            inputs.len()
        ))
    }

    /// code unit들을 down/up INPUT 쌍으로 변환. wVk=0 + KEYEVENTF_UNICODE →
    /// VK_PACKET 경유로 대상 앱에 WM_CHAR가 전달된다.
    fn build_unicode_inputs(units: &[u16]) -> Vec<INPUT> {
        let mut inputs = Vec::with_capacity(units.len() * 2);
        for &unit in units {
            for flags in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
                let mut input: INPUT = unsafe { std::mem::zeroed() };
                input.r#type = INPUT_KEYBOARD;
                input.Anonymous.ki.wScan = unit;
                input.Anonymous.ki.dwFlags = flags;
                inputs.push(input);
            }
        }
        inputs
    }

    // Windows 전용 테스트 — 크로스 체크(cargo check --tests --target
    // x86_64-pc-windows-msvc)로 컴파일이 검증되고, Windows에서 실행된다.
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn 유니코드_input은_유닛당_down_up_쌍이고_vk를_쓰지_않는다() {
            let units: Vec<u16> = "한a🦀".encode_utf16().collect();
            let inputs = build_unicode_inputs(&units);
            assert_eq!(inputs.len(), units.len() * 2);
            for (i, input) in inputs.iter().enumerate() {
                assert_eq!(input.r#type, INPUT_KEYBOARD);
                let ki = unsafe { input.Anonymous.ki };
                assert_eq!(ki.wVk, 0, "VK_PACKET 경유 — 가상 키를 지정하지 않는다");
                assert_eq!(ki.wScan, units[i / 2]);
                assert_eq!(ki.dwFlags & KEYEVENTF_UNICODE, KEYEVENTF_UNICODE);
                assert_eq!(
                    ki.dwFlags & KEYEVENTF_KEYUP != 0,
                    i % 2 == 1,
                    "짝수 = down, 홀수 = up"
                );
            }
        }
    }
}

// 기타 플랫폼: 세대 번호 없음(0 → 감시는 get_text diff, 복원 판정은 텍스트
// 비교 폴백), Concealed 판정 없음, 전경창 제어 없음. 백업은 텍스트만.
#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    #[derive(Debug)]
    pub struct Snapshot {
        text: Option<String>,
    }

    pub fn snapshot() -> Result<Snapshot, String> {
        let mut board = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        Ok(Snapshot {
            text: board.get_text().ok(),
        })
    }

    pub fn restore(snapshot: Snapshot) -> Result<(), String> {
        if let Some(text) = snapshot.text {
            let mut board = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            board.set_text(text).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn change_count() -> isize {
        0
    }
    pub fn is_concealed() -> bool {
        false
    }
    pub fn frontmost_target() -> isize {
        0
    }
    pub fn activate_target(_target: isize) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 전역 HISTORY를 공유하므로 테스트를 직렬화한다 (병렬 실행 시 상호 오염 방지).
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn reset() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        HISTORY.lock().unwrap().clear();
        CAP.store(50, Ordering::Relaxed);
        PREV_APP_TARGET.store(0, Ordering::Relaxed);
        guard
    }

    #[test]
    fn record_최신이_앞으로_그리고_중복_이동() {
        let _g = reset();
        record("a".into());
        record("b".into());
        record("c".into());
        assert_eq!(slot(1).as_deref(), Some("c")); // 최신
        assert_eq!(slot(2).as_deref(), Some("b"));
        assert_eq!(slot(3).as_deref(), Some("a"));

        // "a" 재복사 → 중복 생성 없이 안정 ID를 유지한 채 맨 앞으로
        let a_id = slot_id(3).unwrap();
        record("a".into());
        assert_eq!(slot(1).as_deref(), Some("a"));
        assert_eq!(slot_id(1), Some(a_id), "dedupe 이동에도 ID 유지");
        assert_eq!(slot(2).as_deref(), Some("c"));
        assert_eq!(len(), 3, "중복은 새 항목을 만들지 않는다");
    }

    #[test]
    fn 상한_초과시_오래된것부터_밀린다() {
        let _g = reset();
        CAP.store(3, Ordering::Relaxed);
        for s in ["1", "2", "3", "4"] {
            record(s.into());
        }
        assert_eq!(len(), 3);
        assert_eq!(slot(1).as_deref(), Some("4"));
        assert_eq!(slot(3).as_deref(), Some("2"));
        assert!(slot(4).is_none(), "1은 밀려남");
    }

    #[test]
    fn 빈_문자열과_초과크기는_수집하지_않는다() {
        let _g = reset();
        record(String::new());
        record("x".repeat(MAX_ITEM_BYTES + 1));
        assert_eq!(len(), 0);
    }

    #[test]
    fn 빈_슬롯은_none() {
        let _g = reset();
        assert!(slot(0).is_none());
        assert!(slot(1).is_none());
    }

    #[test]
    fn 안정_id는_새복사로_슬롯이_밀려도_같은_항목을_가리킨다() {
        let _g = reset();
        record("first".into());
        record("selected account number".into());
        let id = slot_id(1).unwrap();

        // 검색→선택 사이에 새 복사가 들어와 슬롯이 밀리는 상황
        record("background clipboard update".into());
        assert_eq!(slot(1).as_deref(), Some("background clipboard update"));
        assert_eq!(
            entry_by_id(id).unwrap().text,
            "selected account number",
            "슬롯이 밀려도 ID는 같은 항목"
        );
    }

    #[test]
    fn 검색_결과에_안정_id가_실린다() {
        let _g = reset();
        record("alpha".into());
        record("beta".into());
        let hits = search("", 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].slot, 1);
        assert_eq!(entry_by_id(hits[0].id).unwrap().text, "beta");
        assert_eq!(entry_by_id(hits[1].id).unwrap().text, "alpha");
    }

    #[test]
    fn 빈_슬롯과_만료된_id_붙여넣기는_에러다() {
        let _g = reset();
        // 항목 조회가 클립보드 접근보다 먼저 실패하므로 실제 클립보드를 건드리지 않는다.
        assert!(paste_slot_checked(1, false).is_err());
        assert!(paste_item(u64::MAX, false).is_err());
    }

    #[test]
    fn 이전_대상이_없으면_현재_포커스에_주입하지_않는다() {
        let mut activated = false;
        let mut pasted = false;
        let error = paste_after_target_activation(
            true,
            0,
            |_| {
                activated = true;
                true
            },
            || {},
            |_| true,
            || {
                pasted = true;
                Ok(())
            },
        )
        .unwrap_err();

        assert!(!activated, "대상이 없으면 활성화를 시도하지 않는다");
        assert!(!pasted, "현재 포커스에 텍스트가 주입되면 안 된다");
        assert!(error.contains("현재 포커스에는 주입하지 않았습니다"));
    }

    #[test]
    fn 이전_대상_활성화가_실패하면_주입을_중단한다() {
        let mut waited = false;
        let mut pasted = false;
        let error = paste_after_target_activation(
            true,
            42,
            |target| {
                assert_eq!(target, 42);
                false
            },
            || waited = true,
            |_| true,
            || {
                pasted = true;
                Ok(())
            },
        )
        .unwrap_err();

        assert!(!waited, "활성화 실패 뒤 포커스 대기를 하면 안 된다");
        assert!(!pasted, "활성화 실패 뒤 현재 포커스에 주입하면 안 된다");
        assert!(error.contains("활성화하지 못해"));
    }

    #[test]
    fn 이전_대상_활성화_성공_뒤에만_대기하고_주입한다() {
        let steps = std::cell::RefCell::new(Vec::new());
        paste_after_target_activation(
            true,
            42,
            |_| {
                steps.borrow_mut().push("activate");
                true
            },
            || steps.borrow_mut().push("wait"),
            |_| {
                steps.borrow_mut().push("verify");
                true
            },
            || {
                steps.borrow_mut().push("paste");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(*steps.borrow(), ["activate", "wait", "verify", "paste"]);
    }

    #[test]
    fn 활성화_대기_중_포커스가_바뀌면_주입하지_않는다() {
        let mut verified_target = 0;
        let mut pasted = false;
        let error = paste_after_target_activation(
            true,
            42,
            |_| true,
            || {},
            |target| {
                verified_target = target;
                false
            },
            || {
                pasted = true;
                Ok(())
            },
        )
        .unwrap_err();

        assert_eq!(verified_target, 42);
        assert!(
            !pasted,
            "포커스가 바뀐 현재 창에는 Unicode를 주입하면 안 된다"
        );
        assert!(error.contains("포커스가 다른 창으로 바뀌어"));
    }

    #[test]
    fn 만료된_id_복사도_에러다() {
        let _g = reset();
        // copy_item도 항목 조회가 먼저다 — 실제 클립보드를 건드리지 않는다.
        assert!(copy_item(u64::MAX).is_err());
    }

    #[test]
    fn 전경_캡처는_데스크톱_런처_실행에만_동반된다() {
        // 다른 launch 바인딩이 흐름 B 붙여넣기 대상을 덮어쓰는 회귀 방지.
        assert!(launch_captures_foreground("kmd-desktop"));
        assert!(!launch_captures_foreground("firefox"));
        assert!(
            !launch_captures_foreground("kmd-desktop.exe"),
            "정확히 일치해야 함"
        );
        assert!(!launch_captures_foreground(""));
    }

    #[cfg(not(windows))]
    #[test]
    fn set_실패_보고는_원본_생존_여부를_담는다() {
        // 실패 원자성: set 실패 시 즉시 복원을 시도하고, 사용자가 "원본이
        // 살아있는가"를 에러 메시지에서 알 수 있어야 한다.
        let restored = swap_failure_message("잠김", Ok(()));
        assert!(restored.contains("원본은 복원됨"), "{restored}");
        let lost = swap_failure_message("잠김", Err("열기 실패".into()));
        assert!(lost.contains("원본 복원도 실패"), "{lost}");
    }

    #[cfg(not(windows))]
    #[test]
    fn 복원은_우리가_넣은_값이_현재일_때만_한다() {
        // 세대 번호 지원: 세대가 그대로면 복원, 바뀌었으면(외부 복사) 생략
        assert!(should_restore(7, 7, false));
        assert!(!should_restore(7, 8, true), "외부 복사 발생 → 복원 생략");
        // 세대 번호 미지원(0): 텍스트 일치로 판정
        assert!(should_restore(0, 0, true));
        assert!(!should_restore(0, 0, false));
    }

    // ── 감시 상태 기계 (레이스 시나리오를 순수 로직으로 재현) ───────────────

    #[test]
    fn 감시_게이트는_세대가_그대로면_클립보드를_열지_않는다() {
        assert!(!watch_gate(false, 5, 5), "변화 없음 → 열지 않음");
        assert!(watch_gate(false, 6, 5), "세대 변화 → 연다");
        assert!(watch_gate(false, 0, 0), "세대 미지원(0) → 항상 열어 diff");
    }

    #[test]
    fn 감시_게이트는_스왑_주입_중이면_열지_않는다() {
        // 스왑 경로가 클립보드를 바꾸는 동안엔 세대가 변해도 수집하지 않는다
        // (우리가 넣은 임시 값이 "새 복사"로 잡히는 것 방지).
        assert!(!watch_gate(true, 6, 5));
        assert!(!watch_gate(true, 0, 0));
    }

    #[test]
    fn 감시_busy는_세대를_소비하지_않아_다음_틱에_재시도한다() {
        // 레이스: 외부 복사(세대 6) 직후 복사 주체가 클립보드를 아직 쥐고 있어
        // 첫 시도는 Busy — 세대를 소비하면 이 복사를 영영 잃는다.
        let mut state = WatchState {
            last_generation: 5,
            last_text: String::new(),
        };
        assert_eq!(watch_transition(&mut state, 6, WatchRead::Busy), None);
        assert_eq!(state.last_generation, 5, "Busy는 세대 미소비");
        // 다음 틱: 세대는 여전히 6 → 게이트 통과 → 이번엔 읽기 성공 → 기록.
        assert!(watch_gate(false, 6, state.last_generation));
        assert_eq!(
            watch_transition(&mut state, 6, WatchRead::Text("복사본".into())).as_deref(),
            Some("복사본")
        );
        assert_eq!(state.last_generation, 6);
        // 그 다음 틱: 변화 없음 → 게이트가 닫힌다 (클립보드 재열기 없음).
        assert!(!watch_gate(false, 6, state.last_generation));
    }

    #[test]
    fn 감시_민감_항목과_비텍스트는_세대를_소비하고_기록하지_않는다() {
        let mut state = WatchState {
            last_generation: 5,
            last_text: String::new(),
        };
        assert_eq!(watch_transition(&mut state, 6, WatchRead::Concealed), None);
        assert_eq!(state.last_generation, 6, "확정 비수집 → 세대 소비");
        assert_eq!(watch_transition(&mut state, 7, WatchRead::NoText), None);
        assert_eq!(state.last_generation, 7, "이미지 등 비텍스트 → 세대 소비");
        assert!(
            !watch_gate(false, 7, state.last_generation),
            "같은 내용에 매 폴링 클립보드를 다시 열지 않는다"
        );
    }

    #[test]
    fn 감시_세대_미지원_플랫폼은_텍스트_diff로_중복을_거른다() {
        let mut state = WatchState {
            last_generation: 0,
            last_text: String::new(),
        };
        assert_eq!(
            watch_transition(&mut state, 0, WatchRead::Text("a".into())).as_deref(),
            Some("a")
        );
        // 게이트는 늘 통과하지만(세대 0) 같은 텍스트는 기록하지 않는다.
        assert!(watch_gate(false, 0, state.last_generation));
        assert_eq!(
            watch_transition(&mut state, 0, WatchRead::Text("a".into())),
            None
        );
        assert_eq!(
            watch_transition(&mut state, 0, WatchRead::Text("b".into())).as_deref(),
            Some("b")
        );
    }

    #[test]
    fn 감시_빈_텍스트는_기록하지_않는다() {
        let mut state = WatchState {
            last_generation: 1,
            last_text: "이전".into(),
        };
        assert_eq!(
            watch_transition(&mut state, 2, WatchRead::Text(String::new())),
            None
        );
        assert_eq!(state.last_generation, 2);
    }
}
