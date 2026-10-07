//! Windows 키 바인딩 백엔드 — WH_KEYBOARD_LL 글로벌 키보드 훅
//!
//! 저수준 키보드 훅으로 키 이벤트를 가로채고,
//! 바인딩 테이블에 따라 키를 리매핑하거나 억제한다.

use super::engine::{EngineState, KeyDecision};
use super::mouse::{MouseSink, MouseWorker};
use super::{
    resolve_launch_cmd, BindAction, KeybindConfig, KeyboardBackend, MacroStep, MouseBind, VKey,
};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use windows_sys::Win32::Foundation::{GetLastError, SetLastError, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

// ── 글로벌 상태 (콜백에서 접근) ──────────────────────────────────────────────
//
// 바인딩 판정 로직은 전부 keybind::engine(플랫폼 독립, 단위 테스트 가능)에
// 있다. 이 파일은 OS 훅 설치/이벤트 변환/액션 큐잉만 담당한다.
//
// 합성 키 SendInput은 물리 PassThrough보다 늦어지는 역전을 막기 위해 콜백의
// 짧은 batch fast path에서 실행하고, Launch/클립보드는 slow worker로 분리한다.
// LL 훅 콜백이 LowLevelHooksTimeout(기본 수백 ms)을 초과하면 OS가 훅을
// 조용히 제거하므로 콜백에서는 sleep·파일·IPC·프로세스 작업을 하지 않는다.
// SendInput으로 주입된 이벤트는 LLKHF_INJECTED 플래그로 걸러지므로
// 재진입 문제가 없다.

static HOOK_STATE: OnceLock<Arc<Mutex<EngineState>>> = OnceLock::new();

/// 메시지 루프 스레드 ID — stop() 시 WM_QUIT를 보내 GetMessageW를 깨운다
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);

// ── 훅 생존 감시 (워치독) ───────────────────────────────────────────────────
//
// Windows는 LL 훅을 **조용히** 제거한다. 콜백이 LowLevelHooksTimeout(기본
// 300ms)을 넘기거나, Modern Standby(S0 저전력 대기) 복귀 과정에서 훅 스레드가
// 스로틀/서스펜드되면 OS가 훅을 체인에서 떼어낸다. 이때 통지는 없다 —
// HHOOK 핸들은 그대로 유효해 보이고, GetMessageW는 계속 블로킹하며,
// 프로세스는 멀쩡히 살아 있다. 결과적으로 "데몬은 실행 중인데 키맵만 죽는"
// 상태가 되고, 재시작 전까지 영구히 복구되지 않았다.
//
// 대응: 훅 콜백이 하트비트를 남기고, 워치독이 "사용자는 입력 중인데 훅은
// 조용한" 상태를 감지하면 무해한 키(VK_NONAME)를 주입해 생존을 확인한 뒤,
// 죽었으면 훅 스레드에 재설치를 요청한다.

/// 훅 콜백이 마지막으로 호출된 시각 (GetTickCount). 콜백 최상단에서 갱신하므로
/// 주입 이벤트로도 갱신된다 — 워치독의 생존 프로브가 성립하는 근거.
static HOOK_LAST_SEEN: AtomicU32 = AtomicU32::new(0);

/// 훅 재설치 누적 횟수 (진단용 — `kmd daemon status`로 노출)
static HOOK_REINSTALLS: AtomicU32 = AtomicU32::new(0);

/// 현재 설치된 훅 세대. 워치독 프로브와 재설치 요청 사이의 ABA 오판을 막는다.
static HOOK_GENERATION: AtomicU32 = AtomicU32::new(0);

/// SendInput 실패 누적 횟수 (키 값은 기록하지 않는 진단용 카운터)
static INJECTION_FAILURES: AtomicU32 = AtomicU32::new(0);
static LAST_INJECTION_EXPECTED: AtomicU32 = AtomicU32::new(0);
static LAST_INJECTION_SENT: AtomicU32 = AtomicU32::new(0);
static LAST_INJECTION_ERROR: AtomicU32 = AtomicU32::new(0);

/// 훅 콜백 최대 지연과 큐 깊이 진단. 사용자 입력 내용은 포함하지 않는다.
static HOOK_MAX_LATENCY_MS: AtomicU32 = AtomicU32::new(0);
static HOOK_SLOW_CALLBACKS: AtomicU32 = AtomicU32::new(0);
static HOOK_PANICS: AtomicU32 = AtomicU32::new(0);
static FAST_QUEUE_DEPTH: AtomicU32 = AtomicU32::new(0);
static SLOW_QUEUE_DEPTH: AtomicU32 = AtomicU32::new(0);
static FAST_QUEUE_MAX_DEPTH: AtomicU32 = AtomicU32::new(0);
static SLOW_QUEUE_MAX_DEPTH: AtomicU32 = AtomicU32::new(0);
static QUEUE_DROPS: AtomicU32 = AtomicU32::new(0);

/// 재설치 실패 뒤에는 훅 프로브를 주입하지 않고 제한된 backoff로 직접 재시도한다.
static HOOK_REINSTALL_PENDING: AtomicBool = AtomicBool::new(false);

/// 합성 key-up/마우스 button-up 실패는 성공할 때까지 추적한다. 콜백에서는
/// 최초 시도와 기록만 하고, 재시도는 워치독/fast worker에서 제한된 묶음으로 한다.
static PENDING_SYNTHETIC_RELEASES: OnceLock<Mutex<PendingReleases<SyntheticRelease>>> =
    OnceLock::new();
static PENDING_MOUSE_RELEASES: OnceLock<Mutex<PendingReleases<MouseBind>>> = OnceLock::new();

/// 훅 스레드에 재설치를 요청하는 커스텀 메시지.
/// LL 훅은 설치한 스레드의 메시지 큐로 디스패치되므로 재설치도 그 스레드에서
/// 해야 한다 (다른 스레드에서 SetWindowsHookExW를 부르면 훅이 그쪽에 붙는다).
const WM_KMD_REINSTALL_HOOK: u32 = WM_APP + 0x10;

/// 워치독 점검 주기
const WATCHDOG_INTERVAL_MS: u64 = 2000;

/// 시스템 입력보다 훅 하트비트가 이만큼 뒤처지면 "의심" 상태
const HOOK_STALE_MS: i32 = 1500;

/// 시스템 전체 입력이 이보다 오래 없으면 판단을 보류한다.
/// 사용자가 자리를 비운 상태에서 프로브를 쏘면 유휴 타이머가 리셋돼
/// 화면 꺼짐·절전이 영영 걸리지 않는다 — 이 가드가 그걸 막는다.
const IDLE_SKIP_MS: i32 = 5000;

/// 프로브 최소 간격 — 마우스만 쓰는 구간에서 매 주기 주입하지 않도록 제한
const PROBE_MIN_INTERVAL_MS: i32 = 10_000;

/// 프로브 주입 후 훅 콜백을 기다리는 시간
const PROBE_WAIT_MS: u64 = 120;

/// 재설치 실패 재시도 backoff. 프로브 입력 없이 메시지만 전송한다.
const REINSTALL_RETRY_INITIAL_MS: u32 = 2_000;
const REINSTALL_RETRY_MAX_MS: u32 = 60_000;

/// 한 주기에서 처리하는 release 재시도 수와 fast worker 점검 주기.
const RELEASE_RETRY_BATCH: usize = 8;
const STOP_RELEASE_RETRY_PASSES: usize = 3;
const MOUSE_RELEASE_RETRY_MS: u64 = 25;

/// 훅 콜백 지연 경고 기준. SendInput fast path가 비정상적으로 오래 걸리는지 감시한다.
const HOOK_LATENCY_WARN_MS: u32 = 20;

/// 무제한 mpsc 큐가 밀릴 때 최초 경고를 남기는 깊이.
const QUEUE_WARN_DEPTH: u32 = 32;

/// 훅 재설치 누적 횟수 (진단용)
pub fn hook_reinstall_count() -> u32 {
    HOOK_REINSTALLS.load(Ordering::Relaxed)
}

/// 마지막 훅 이벤트 이후 경과 ms. 훅 미설치면 None.
pub fn hook_idle_ms() -> Option<u32> {
    let last = HOOK_LAST_SEEN.load(Ordering::Relaxed);
    if last == 0 {
        return None;
    }
    Some(unsafe { GetTickCount() }.wrapping_sub(last))
}

/// 시스템 전체 마지막 입력 시각 (키보드+마우스, GetTickCount 기준)
fn last_system_input_tick() -> Option<u32> {
    unsafe {
        let mut lii: LASTINPUTINFO = std::mem::zeroed();
        lii.cbSize = std::mem::size_of::<LASTINPUTINFO>() as u32;
        if GetLastInputInfo(&mut lii) == 0 {
            return None;
        }
        Some(lii.dwTime)
    }
}

/// fast worker 작업 — 상태형 마우스만 소유한다.
///
/// 키 합성은 물리 이벤트보다 늦게 도착하지 않도록 훅 콜백의 짧은 SendInput
/// fast path에서 즉시 처리한다. Launch/클립보드는 별도 slow worker로 보낸다.
enum FastJob {
    /// 마우스 바인딩 시작 — 이동/휠은 모션 워커, 버튼은 즉시 down 주입
    Engage(MouseBind),
    /// 마우스 바인딩 정지 — 이동/휠 해제, 버튼 up 주입
    Release(MouseBind),
    /// 활성 마우스 바인딩 전체 정지 (stuck-mouse 방지)
    StopAll,
    /// 일회성 마우스 액션. Engage/Release와 같은 FIFO에서 실행해 순서를 보장한다.
    Pulse(MouseBind),
}

static FAST_TX: Mutex<Option<mpsc::Sender<FastJob>>> = Mutex::new(None);
static SLOW_TX: Mutex<Option<mpsc::Sender<BindAction>>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingRelease<T> {
    item: T,
    generation: u64,
}

/// 재시도해야 할 키보드 해제. 일반 리맵 키는 VK 기반이고, 클립보드 타이핑은
/// KEYEVENTF_UNICODE의 UTF-16 code unit 기반이라 입력 형식을 함께 보존한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyntheticRelease {
    VirtualKey(u16),
    UnicodeUnit(u16),
}

#[derive(Debug)]
struct PendingReleases<T> {
    entries: Vec<PendingRelease<T>>,
    next_generation: u64,
}

impl<T> Default for PendingReleases<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            next_generation: 0,
        }
    }
}

impl<T: Copy + Eq> PendingReleases<T> {
    fn record(&mut self, item: T) {
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.item == item) {
            entry.generation = self.next_generation;
        } else {
            self.entries.push(PendingRelease {
                item,
                generation: self.next_generation,
            });
        }
    }

    fn snapshot(&self, limit: usize) -> Vec<PendingRelease<T>> {
        self.entries.iter().take(limit).copied().collect()
    }

    fn complete(&mut self, completed: PendingRelease<T>) -> bool {
        let Some(pos) = self.entries.iter().position(|entry| *entry == completed) else {
            return false;
        };
        self.entries.remove(pos);
        true
    }

    fn remove(&mut self, item: T) {
        self.entries.retain(|entry| entry.item != item);
    }

    fn items(&self) -> Vec<T> {
        self.entries.iter().map(|entry| entry.item).collect()
    }
}

fn lock_pending<T>(
    pending: &Mutex<PendingReleases<T>>,
) -> std::sync::MutexGuard<'_, PendingReleases<T>> {
    pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn retry_pending_releases<T: Copy + Eq>(
    pending: &Mutex<PendingReleases<T>>,
    limit: usize,
    mut release: impl FnMut(T) -> bool,
) -> Vec<T> {
    let snapshot = lock_pending(pending).snapshot(limit);
    let mut completed_items = Vec::new();
    for entry in snapshot {
        if release(entry.item) && lock_pending(pending).complete(entry) {
            completed_items.push(entry.item);
        }
    }
    completed_items
}

fn pending_synthetic_releases() -> &'static Mutex<PendingReleases<SyntheticRelease>> {
    PENDING_SYNTHETIC_RELEASES.get_or_init(|| Mutex::new(PendingReleases::default()))
}

fn pending_mouse_releases() -> &'static Mutex<PendingReleases<MouseBind>> {
    PENDING_MOUSE_RELEASES.get_or_init(|| Mutex::new(PendingReleases::default()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionLane {
    Inline,
    MouseWorker,
    Slow,
}

fn action_lane(action: &BindAction) -> ActionLane {
    match action {
        BindAction::Launch(_) | BindAction::ClipPaste(_) => ActionLane::Slow,
        BindAction::Mouse(_) => ActionLane::MouseWorker,
        _ => ActionLane::Inline,
    }
}

fn note_queue_depth(max_depth: &AtomicU32, depth: u32) {
    max_depth.fetch_max(depth, Ordering::Relaxed);
}

/// 상태형 마우스 작업을 fast worker에 넣는다 (훅 콜백에서 호출 — 블로킹 없음).
fn queue_fast(job: FastJob) {
    let sender = FAST_TX.lock().ok().and_then(|g| g.clone());
    match sender {
        Some(tx) => {
            let depth = FAST_QUEUE_DEPTH.fetch_add(1, Ordering::Relaxed) + 1;
            note_queue_depth(&FAST_QUEUE_MAX_DEPTH, depth);
            if tx.send(job).is_err() {
                FAST_QUEUE_DEPTH.fetch_sub(1, Ordering::Relaxed);
                QUEUE_DROPS.fetch_add(1, Ordering::Relaxed);
            }
        }
        None => {
            QUEUE_DROPS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// 파일/IPC/프로세스 작업을 slow worker에 넣는다.
fn queue_slow(action: BindAction) {
    let sender = SLOW_TX.lock().ok().and_then(|g| g.clone());
    match sender {
        Some(tx) => {
            let depth = SLOW_QUEUE_DEPTH.fetch_add(1, Ordering::Relaxed) + 1;
            note_queue_depth(&SLOW_QUEUE_MAX_DEPTH, depth);
            if tx.send(action).is_err() {
                SLOW_QUEUE_DEPTH.fetch_sub(1, Ordering::Relaxed);
                QUEUE_DROPS.fetch_add(1, Ordering::Relaxed);
            }
        }
        None => {
            QUEUE_DROPS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// ── VKey → Windows VK 코드 변환 ─────────────────────────────────────────────

fn vkey_to_vk(key: VKey) -> u16 {
    match key {
        VKey::A => 0x41,
        VKey::B => 0x42,
        VKey::C => 0x43,
        VKey::D => 0x44,
        VKey::E => 0x45,
        VKey::F => 0x46,
        VKey::G => 0x47,
        VKey::H => 0x48,
        VKey::I => 0x49,
        VKey::J => 0x4A,
        VKey::K => 0x4B,
        VKey::L => 0x4C,
        VKey::M => 0x4D,
        VKey::N => 0x4E,
        VKey::O => 0x4F,
        VKey::P => 0x50,
        VKey::Q => 0x51,
        VKey::R => 0x52,
        VKey::S => 0x53,
        VKey::T => 0x54,
        VKey::U => 0x55,
        VKey::V => 0x56,
        VKey::W => 0x57,
        VKey::X => 0x58,
        VKey::Y => 0x59,
        VKey::Z => 0x5A,
        VKey::Num0 => 0x30,
        VKey::Num1 => 0x31,
        VKey::Num2 => 0x32,
        VKey::Num3 => 0x33,
        VKey::Num4 => 0x34,
        VKey::Num5 => 0x35,
        VKey::Num6 => 0x36,
        VKey::Num7 => 0x37,
        VKey::Num8 => 0x38,
        VKey::Num9 => 0x39,
        VKey::F1 => VK_F1,
        VKey::F2 => VK_F2,
        VKey::F3 => VK_F3,
        VKey::F4 => VK_F4,
        VKey::F5 => VK_F5,
        VKey::F6 => VK_F6,
        VKey::F7 => VK_F7,
        VKey::F8 => VK_F8,
        VKey::F9 => VK_F9,
        VKey::F10 => VK_F10,
        VKey::F11 => VK_F11,
        VKey::F12 => VK_F12,
        // F19 — 트리거 전용(macOS CapsLock→F19 재맵용). Windows는 CapsLock을
        // 직접 트리거로 쓰므로 F19가 실제로 오진 않지만 exhaustive match를 위해 둔다.
        VKey::F19 => VK_F19,
        VKey::Escape => VK_ESCAPE,
        VKey::Tab => VK_TAB,
        VKey::CapsLock => VK_CAPITAL,
        VKey::Space => VK_SPACE,
        VKey::Enter => VK_RETURN,
        VKey::Backspace => VK_BACK,
        VKey::Delete => VK_DELETE,
        VKey::Left => VK_LEFT,
        VKey::Right => VK_RIGHT,
        VKey::Up => VK_UP,
        VKey::Down => VK_DOWN,
        VKey::Home => VK_HOME,
        VKey::End => VK_END,
        VKey::PageUp => VK_PRIOR,
        VKey::PageDown => VK_NEXT,
        VKey::Insert => VK_INSERT,
        VKey::PrintScreen => VK_SNAPSHOT,
        VKey::ScrollLock => VK_SCROLL,
        VKey::Pause => VK_PAUSE,
        VKey::LShift => VK_LSHIFT,
        VKey::RShift => VK_RSHIFT,
        VKey::LCtrl => VK_LCONTROL,
        VKey::RCtrl => VK_RCONTROL,
        VKey::LAlt => VK_LMENU,
        VKey::RAlt => VK_RMENU,
        VKey::LWin => VK_LWIN,
        VKey::RWin => VK_RWIN,
        VKey::Semicolon => VK_OEM_1,
        VKey::Quote => VK_OEM_7,
        VKey::Comma => VK_OEM_COMMA,
        VKey::Period => VK_OEM_PERIOD,
        VKey::Slash => VK_OEM_2,
        VKey::Backslash => VK_OEM_5,
        VKey::LBracket => VK_OEM_4,
        VKey::RBracket => VK_OEM_6,
        VKey::Minus => VK_OEM_MINUS,
        VKey::Equal => VK_OEM_PLUS,
        VKey::Grave => VK_OEM_3,
        VKey::Hangul => 0x15,
        VKey::Hanja => 0x19,
    }
}

/// Windows VK 코드 → VKey 역변환.
///
/// 정방향 [`vkey_to_vk`](컴파일러가 변형 누락을 잡는 exhaustive match)에서
/// 자동 생성한다 — 거울상 match 두 벌을 손으로 유지하다 어긋나는 것 방지.
/// 정합성은 왕복 테스트(`vk_왕복_변환_일치`)가 보장한다.
fn vk_to_vkey(vk: u16) -> Option<VKey> {
    static REVERSE: OnceLock<std::collections::HashMap<u16, VKey>> = OnceLock::new();
    REVERSE
        .get_or_init(|| VKey::ALL.iter().map(|&k| (vkey_to_vk(k), k)).collect())
        .get(&vk)
        .copied()
}

// ── SendInput 헬퍼 ──────────────────────────────────────────────────────────

/// LLKHF_INJECTED 플래그 — SendInput으로 주입된 이벤트 식별
const LLKHF_INJECTED: u32 = 0x00000010;

/// 확장 키 여부 판정 — 이 키들은 KEYEVENTF_EXTENDEDKEY 플래그 필수
fn is_extended_vk(vk: u16) -> bool {
    matches!(
        vk,
        VK_UP
            | VK_DOWN
            | VK_LEFT
            | VK_RIGHT
            | VK_HOME
            | VK_END
            | VK_PRIOR
            | VK_NEXT
            | VK_INSERT
            | VK_DELETE
            | VK_RCONTROL
            | VK_RMENU
            | VK_LWIN
            | VK_RWIN
            | VK_SNAPSHOT
    )
}

#[derive(Debug)]
struct SendInputFailure {
    sent: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyboardEvent {
    Down(u16),
    Up(u16),
}

fn keyboard_input(event: KeyboardEvent) -> INPUT {
    let (vk, key_up) = match event {
        KeyboardEvent::Down(vk) => (vk, false),
        KeyboardEvent::Up(vk) => (vk, true),
    };
    unsafe {
        let mut input: INPUT = std::mem::zeroed();
        input.r#type = INPUT_KEYBOARD;
        input.Anonymous.ki.wVk = vk;
        input.Anonymous.ki.wScan = MapVirtualKeyW(vk as u32, 0) as u16;
        input.Anonymous.ki.dwFlags = if key_up { KEYEVENTF_KEYUP } else { 0 }
            | if is_extended_vk(vk) {
                KEYEVENTF_EXTENDEDKEY
            } else {
                0
            };
        input
    }
}

fn unicode_key_up_input(unit: u16) -> INPUT {
    unsafe {
        let mut input: INPUT = std::mem::zeroed();
        input.r#type = INPUT_KEYBOARD;
        input.Anonymous.ki.wScan = unit;
        input.Anonymous.ki.dwFlags = KEYEVENTF_UNICODE | KEYEVENTF_KEYUP;
        input
    }
}

/// SendInput 반환 개수를 항상 검증한다. operation은 고정된 동작 분류만 받고
/// VK/사용자 입력 내용은 진단에 남기지 않는다.
fn submit_inputs(inputs: &[INPUT], _operation: &'static str) -> Result<(), SendInputFailure> {
    if inputs.is_empty() {
        return Ok(());
    }
    let expected = inputs.len() as u32;
    let sent = unsafe {
        SetLastError(0);
        SendInput(
            expected,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent == expected {
        return Ok(());
    }

    let os_error = unsafe { GetLastError() };
    INJECTION_FAILURES.fetch_add(1, Ordering::Relaxed);
    LAST_INJECTION_EXPECTED.store(expected, Ordering::Relaxed);
    LAST_INJECTION_SENT.store(sent, Ordering::Relaxed);
    LAST_INJECTION_ERROR.store(os_error, Ordering::Relaxed);
    Err(SendInputFailure { sent })
}

/// SendInput이 prefix 일부만 처리한 경우 실제로 내려간 키만 역순 해제한다.
fn cleanup_events_for_accepted_prefix(
    events: &[KeyboardEvent],
    accepted: usize,
) -> Vec<KeyboardEvent> {
    let mut held = Vec::new();
    for event in events.iter().take(accepted) {
        match *event {
            KeyboardEvent::Down(vk) => {
                if !held.contains(&vk) {
                    held.push(vk);
                }
            }
            KeyboardEvent::Up(vk) => {
                if let Some(pos) = held.iter().rposition(|&held_vk| held_vk == vk) {
                    held.remove(pos);
                }
            }
        }
    }
    held.into_iter().rev().map(KeyboardEvent::Up).collect()
}

fn unaccepted_cleanup_releases(
    cleanup: &[KeyboardEvent],
    accepted: usize,
) -> Vec<SyntheticRelease> {
    cleanup
        .iter()
        .skip(accepted)
        .filter_map(|event| match *event {
            KeyboardEvent::Up(vk) => Some(SyntheticRelease::VirtualKey(vk)),
            KeyboardEvent::Down(_) => None,
        })
        .collect()
}

fn send_keyboard_events(
    events: &[KeyboardEvent],
    operation: &'static str,
) -> Result<(), SendInputFailure> {
    let inputs: Vec<INPUT> = events.iter().copied().map(keyboard_input).collect();
    match submit_inputs(&inputs, operation) {
        Ok(()) => Ok(()),
        Err(failure) => {
            let cleanup = cleanup_events_for_accepted_prefix(events, failure.sent as usize);
            if !cleanup.is_empty() {
                let cleanup_inputs: Vec<INPUT> =
                    cleanup.iter().copied().map(keyboard_input).collect();
                // 훅 콜백에서는 정리 batch를 한 번만 시도한다. UIPI처럼 주입이
                // 막힌 상황에서 키마다 재시도하면 LowLevelHooksTimeout을 넘길 수 있다.
                if let Err(cleanup_failure) =
                    submit_inputs(&cleanup_inputs, "부분 실패 수정자 정리")
                {
                    // 정리 batch마저 부분 실패하면 실제 전송되지 않은 up만 기존
                    // 워치독 재시도 큐로 넘긴다. 즉시 호출의 prefix는 이미
                    // 해제됐으므로 다시 기록하지 않는다.
                    let unsent =
                        unaccepted_cleanup_releases(&cleanup, cleanup_failure.sent as usize);
                    let mut pending = lock_pending(pending_synthetic_releases());
                    for release in unsent {
                        pending.record(release);
                    }
                }
            }
            Err(failure)
        }
    }
}

fn send_synthetic_release(release: SyntheticRelease) -> Result<(), SendInputFailure> {
    let input = match release {
        SyntheticRelease::VirtualKey(vk) => keyboard_input(KeyboardEvent::Up(vk)),
        SyntheticRelease::UnicodeUnit(unit) => unicode_key_up_input(unit),
    };
    submit_inputs(&[input], "키 up")
}

fn release_synthetic_or_track(release: SyntheticRelease) -> bool {
    if send_synthetic_release(release).is_ok() {
        lock_pending(pending_synthetic_releases()).remove(release);
        true
    } else {
        lock_pending(pending_synthetic_releases()).record(release);
        false
    }
}

/// 콜백에서는 key-up을 한 번만 시도하고 실패를 기록한다. 기록된 항목은
/// 워치독이 성공할 때까지 보존하며 한 주기당 제한된 수만 재시도한다.
fn release_key_up_or_track(vk: u16) -> bool {
    release_synthetic_or_track(SyntheticRelease::VirtualKey(vk))
}

/// 클립보드 유니코드 주입이 down에서 끊겼을 때 대응 up을 즉시 시도하고,
/// 실패하면 일반 합성 키와 같은 워치독 재시도 큐에 등록한다.
pub(crate) fn release_unicode_unit_or_track(unit: u16) -> bool {
    release_synthetic_or_track(SyntheticRelease::UnicodeUnit(unit))
}

fn retry_pending_synthetic_releases() {
    retry_pending_releases(
        pending_synthetic_releases(),
        RELEASE_RETRY_BATCH,
        |release| send_synthetic_release(release).is_ok(),
    );
}

fn key_press_events(vk: u16) -> [KeyboardEvent; 2] {
    [KeyboardEvent::Down(vk), KeyboardEvent::Up(vk)]
}

fn send_key_press(vk: u16) -> Result<(), SendInputFailure> {
    send_keyboard_events(&key_press_events(vk), "키 press")
}

/// 코드 진입 주입: 트리거 down → 키 down을 **한 번의 SendInput 호출**로 보낸다.
/// 두 이벤트 사이에 다른 입력이 끼어들 수 없어 Alt+Tab 같은 조합의
/// 순서(트리거가 먼저)가 보장된다. 트리거 up은 ChordRelease에서 별도 주입.
fn send_chord_engage(trigger_vk: u16, key_vk: u16) -> Result<(), SendInputFailure> {
    send_keyboard_events(
        &[KeyboardEvent::Down(trigger_vk), KeyboardEvent::Down(key_vk)],
        "chord 진입",
    )
}

// ── 마우스 주입 헬퍼 ────────────────────────────────────────────────────────

/// 휠 1노치 델타 (WinUser.h WHEEL_DELTA)
const WHEEL_DELTA_UNIT: i32 = 120;

fn mouse_input(dx: i32, dy: i32, mouse_data: i32, flags: u32) -> INPUT {
    unsafe {
        let mut input: INPUT = std::mem::zeroed();
        input.r#type = INPUT_MOUSE;
        input.Anonymous.mi.dx = dx;
        input.Anonymous.mi.dy = dy;
        // DWORD 필드지만 휠 델타는 부호 있는 값 — 2의 보수 캐스팅
        input.Anonymous.mi.mouseData = mouse_data as u32;
        input.Anonymous.mi.dwFlags = flags;
        input
    }
}

fn send_mouse_input(
    dx: i32,
    dy: i32,
    mouse_data: i32,
    flags: u32,
    operation: &'static str,
) -> Result<(), SendInputFailure> {
    submit_inputs(&[mouse_input(dx, dy, mouse_data, flags)], operation)
}

fn send_mouse_move(dx: i32, dy: i32) -> Result<(), SendInputFailure> {
    send_mouse_input(dx, dy, 0, MOUSEEVENTF_MOVE, "마우스 이동")
}

fn send_mouse_wheel(notches: i32) -> Result<(), SendInputFailure> {
    send_mouse_input(
        0,
        0,
        notches * WHEEL_DELTA_UNIT,
        MOUSEEVENTF_WHEEL,
        "마우스 휠",
    )
}

fn mouse_button_flags(bind: MouseBind, down: bool) -> Option<u32> {
    let flags = match (bind, down) {
        (MouseBind::BtnLeft, true) => MOUSEEVENTF_LEFTDOWN,
        (MouseBind::BtnLeft, false) => MOUSEEVENTF_LEFTUP,
        (MouseBind::BtnRight, true) => MOUSEEVENTF_RIGHTDOWN,
        (MouseBind::BtnRight, false) => MOUSEEVENTF_RIGHTUP,
        (MouseBind::BtnMiddle, true) => MOUSEEVENTF_MIDDLEDOWN,
        (MouseBind::BtnMiddle, false) => MOUSEEVENTF_MIDDLEUP,
        _ => return None,
    };
    Some(flags)
}

fn send_mouse_button(bind: MouseBind, down: bool) -> Result<(), SendInputFailure> {
    let Some(flags) = mouse_button_flags(bind, down) else {
        return Ok(());
    };
    send_mouse_input(0, 0, 0, flags, "마우스 버튼")
}

fn send_mouse_button_click(bind: MouseBind) -> Result<(), SendInputFailure> {
    let (Some(down), Some(up)) = (
        mouse_button_flags(bind, true),
        mouse_button_flags(bind, false),
    ) else {
        return Ok(());
    };
    let inputs = [mouse_input(0, 0, 0, down), mouse_input(0, 0, 0, up)];
    match submit_inputs(&inputs, "마우스 클릭") {
        Ok(()) => Ok(()),
        Err(failure) => {
            if let Some(accepted_down) =
                mouse_release_for_accepted_click_prefix(bind, failure.sent as usize)
            {
                // down만 수락됐으면 up을 한 번 즉시 시도한다. 이 정리 호출도
                // 실패하면 전역 pending 큐에 남겨 fast worker가 계속 재시도한다.
                let _ = release_mouse_button_or_track(accepted_down);
            }
            Err(failure)
        }
    }
}

fn mouse_release_for_accepted_click_prefix(bind: MouseBind, accepted: usize) -> Option<MouseBind> {
    (accepted == 1).then_some(bind)
}

fn is_mouse_button(bind: MouseBind) -> bool {
    matches!(
        bind,
        MouseBind::BtnLeft | MouseBind::BtnRight | MouseBind::BtnMiddle
    )
}

fn release_mouse_button_or_track(bind: MouseBind) -> bool {
    let released = send_mouse_button(bind, false).is_ok();
    track_mouse_release_result(pending_mouse_releases(), bind, released)
}

fn track_mouse_release_result(
    pending: &Mutex<PendingReleases<MouseBind>>,
    bind: MouseBind,
    released: bool,
) -> bool {
    if released {
        lock_pending(pending).remove(bind);
        true
    } else {
        lock_pending(pending).record(bind);
        false
    }
}

fn retry_pending_mouse_buttons(pressed_buttons: &mut Vec<MouseBind>) {
    let completed = retry_pending_releases(pending_mouse_releases(), RELEASE_RETRY_BATCH, |bind| {
        send_mouse_button(bind, false).is_ok()
    });
    pressed_buttons.retain(|bind| !completed.contains(bind));
}

/// SendInput 기반 [`MouseSink`] — 모션 워커가 틱마다 호출
struct WinMouseSink;

impl MouseSink for WinMouseSink {
    fn move_rel(&mut self, dx: i32, dy: i32) {
        let _ = send_mouse_move(dx, dy);
    }
    fn wheel(&mut self, notches: i32) {
        let _ = send_mouse_wheel(notches);
    }
}

/// [P3 스파이크] 현재 전경 앱에 Ctrl+V 주입 — 클립보드 붙여넣기 확장의 핵심 능력.
pub(super) fn inject_paste() {
    const VK_CONTROL: u16 = 0x11;
    const VK_V: u16 = 0x56;
    let _ = send_combo(&[VK_CONTROL], VK_V);
}

fn combo_events(modifier_vks: &[u16], key_vk: u16) -> Vec<KeyboardEvent> {
    let mut events = Vec::with_capacity(modifier_vks.len() * 2 + 2);
    for &m in modifier_vks {
        events.push(KeyboardEvent::Down(m));
    }
    events.extend(key_press_events(key_vk));
    for &m in modifier_vks.iter().rev() {
        events.push(KeyboardEvent::Up(m));
    }
    events
}

fn send_combo(modifier_vks: &[u16], key_vk: u16) -> Result<(), SendInputFailure> {
    send_keyboard_events(&combo_events(modifier_vks, key_vk), "키 조합")
}

// ── 바인딩 액션 실행 ────────────────────────────────────────────────────────

/// 합성 입력 fast path. 훅 콜백에서 호출되므로 파일/IPC/프로세스 작업 금지.
fn execute_inline_action(action: &BindAction) -> Result<(), SendInputFailure> {
    match action {
        BindAction::SendKey(key) => send_key_press(vkey_to_vk(*key)),
        BindAction::SendCombo { modifiers, key } => {
            let mod_vks: Vec<u16> = modifiers.iter().map(|m| vkey_to_vk(*m)).collect();
            send_combo(&mod_vks, vkey_to_vk(*key))
        }
        BindAction::Macro(steps) => {
            // 설정 파서가 이벤트 수를 이 상한 이하로 보장한다. 콜백 안에서
            // Vec가 반복 재할당되지 않도록 한 번만 고정 크기로 예약한다.
            let mut events = Vec::with_capacity(super::MAX_MACRO_INPUT_EVENTS);
            for step in steps {
                match step {
                    MacroStep::KeyPress(k) => {
                        events.push(KeyboardEvent::Down(vkey_to_vk(*k)));
                    }
                    MacroStep::KeyRelease(k) => {
                        events.push(KeyboardEvent::Up(vkey_to_vk(*k)));
                    }
                    MacroStep::Combo { modifiers, key } => {
                        let mod_vks: Vec<u16> = modifiers.iter().map(|m| vkey_to_vk(*m)).collect();
                        events.extend(combo_events(&mod_vks, vkey_to_vk(*key)));
                    }
                }
            }
            send_keyboard_events(&events, "키 매크로")
        }
        BindAction::Mouse(_) | BindAction::Launch(_) | BindAction::ClipPaste(_) => {
            unreachable!("worker 액션이 합성 입력 inline path에 들어옴")
        }
    }
}

/// 콤보/리맵/더블탭에서 발생한 일회성 마우스 액션.
/// 상태형 Engage/Release와 같은 worker FIFO에서 실행한다.
fn execute_mouse_pulse(mb: MouseBind) -> Result<(), SendInputFailure> {
    match mb {
        MouseBind::BtnLeft | MouseBind::BtnRight | MouseBind::BtnMiddle => {
            send_mouse_button_click(mb)
        }
        MouseBind::WheelUp => send_mouse_wheel(1),
        MouseBind::WheelDown => send_mouse_wheel(-1),
        MouseBind::MoveUp => send_mouse_move(0, -25),
        MouseBind::MoveDown => send_mouse_move(0, 25),
        MouseBind::MoveLeft => send_mouse_move(-25, 0),
        MouseBind::MoveRight => send_mouse_move(25, 0),
        MouseBind::Slow => Ok(()),
    }
}

/// 느린 액션 전용 worker. 훅 콜백과 key-up/chord release 경로를 막지 않는다.
fn execute_slow_action(action: BindAction) {
    match action {
        BindAction::Launch(cmd) => {
            // 런처가 포커스를 뺏기 전에 전경 HWND 캡처 (docs/12 흐름 B).
            // 다른 launch 바인딩이 런처 사용 중 붙여넣기 대상을 덮어쓰지 않도록
            // 데스크톱 런처를 여는 액션에서만 캡처한다.
            if crate::clipboard::launch_captures_foreground(&cmd) {
                crate::clipboard::capture_foreground_app();
            }
            let resolved = resolve_launch_cmd(&cmd);
            tracing::info!("프로그램 실행: {resolved}");
            std::thread::spawn(move || {
                use kmd_core::process::HideConsole;
                if let Err(e) = std::process::Command::new(&resolved).hide_console().spawn() {
                    tracing::error!("프로그램 실행 실패: {resolved} — {e}");
                }
            });
        }
        BindAction::ClipPaste(n) => crate::clipboard::paste_slot(n),
        _ => tracing::error!("합성 입력 액션이 느린 worker로 잘못 전달됨"),
    }
}

fn recover_engine_after_injection_failure() {
    let mut stop_mouse = false;
    if let Some(state) = HOOK_STATE.get() {
        if let Ok(mut guard) = state.lock() {
            stop_mouse = guard.recover_from_injection_failure();
        }
    }
    if stop_mouse {
        // 복구가 레이어 상태를 지우면 해당 물리 key-up은 더 이상 MouseRelease로
        // 변환되지 않는다. 같은 FIFO에 StopAll을 넣어 모션과 버튼을 함께 끊는다.
        queue_fast(FastJob::StopAll);
    }
}

// ── 키보드 훅 콜백 ──────────────────────────────────────────────────────────

unsafe extern "system" fn keyboard_hook_proc(
    code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    // 생존 하트비트 — 콜백이 호출됐다는 사실 자체가 훅이 체인에 살아 있다는
    // 증거다. 주입(INJECTED) 이벤트와 code<0도 포함해 최상단에서 기록해야
    // 워치독의 프로브가 성립한다. (catch_unwind 밖 — 원자 저장은 패닉 불가)
    let started = GetTickCount();
    HOOK_LAST_SEEN.store(nonzero_tick(started), Ordering::Relaxed);

    // `extern "system"` 경계를 넘는 패닉은 정의되지 않은 동작이고, 현대 Rust는
    // 이를 감지하면 프로세스를 abort시킨다 — 이 콜백이 유일한 키 이벤트 처리
    // 지점이라 그 순간 키보드 전체가 잠긴다. 어떤 패닉도 여기서 격리하고,
    // 다음 훅으로 이벤트를 넘겨(remap 없이) OS 기본 동작은 살린다.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        keyboard_hook_proc_inner(code, w_param, l_param)
    }))
    .unwrap_or_else(|_| {
        HOOK_PANICS.fetch_add(1, Ordering::Relaxed);
        CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param)
    });

    let elapsed = GetTickCount().wrapping_sub(started);
    HOOK_MAX_LATENCY_MS.fetch_max(elapsed, Ordering::Relaxed);
    if elapsed >= HOOK_LATENCY_WARN_MS {
        HOOK_SLOW_CALLBACKS.fetch_add(1, Ordering::Relaxed);
    }
    result
}

unsafe fn keyboard_hook_proc_inner(code: i32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param);
    }

    let kb = &*(l_param as *const KBDLLHOOKSTRUCT);

    if kb.flags & LLKHF_INJECTED != 0 {
        return CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param);
    }

    let is_down = w_param == WM_KEYDOWN as usize || w_param == WM_SYSKEYDOWN as usize;
    let is_up = w_param == WM_KEYUP as usize || w_param == WM_SYSKEYUP as usize;

    if !is_down && !is_up {
        return CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param);
    }

    let vk = kb.vkCode as u16;
    let Some(vkey) = vk_to_vkey(vk) else {
        return CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param);
    };

    let state = match HOOK_STATE.get() {
        Some(s) => s,
        None => return CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param),
    };

    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param),
    };

    // 바인딩 판정은 순수 엔진에 위임한다. 합성 키 입력은 이 콜백 안에서 한 번의
    // SendInput batch로 즉시 넣어, 이후 물리 PassThrough 입력과 순서가 뒤집히지
    // 않게 한다. 파일/IPC/프로세스 작업은 slow worker로만 큐잉한다.
    // layer_trigger는 무시한다: Windows SendInput은 물리 modifier가 눌린
    // 상태에서도 합성 이벤트에 간섭하지 않는다 (macOS 전용 컨텍스트).
    match guard.process_key(vkey, is_down, kb.time) {
        KeyDecision::PassThrough => {
            drop(guard);
            CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param)
        }
        KeyDecision::Suppress => 1,
        KeyDecision::Execute { action, .. } => {
            drop(guard);
            match action_lane(&action) {
                ActionLane::Inline => {
                    if execute_inline_action(&action).is_err() {
                        // 물리 down은 계속 억제해 remap keyup과 불일치하지 않게 하고,
                        // 이미 전진한 combo/layer 상태만 초기화한다.
                        recover_engine_after_injection_failure();
                    }
                }
                ActionLane::MouseWorker => {
                    let BindAction::Mouse(mb) = action else {
                        unreachable!("마우스 lane에는 마우스 액션만 들어와야 함")
                    };
                    queue_fast(FastJob::Pulse(mb));
                }
                ActionLane::Slow => queue_slow(action),
            }
            1
        }
        // 코드 진입: 물리 이벤트를 억제하고 트리거+키를 묶어 주입.
        // 이후 이 홀드의 키들은 엔진이 PassThrough — OS에는 주입된
        // 트리거가 눌려 있으므로 Alt+Tab 등 조합이 그대로 동작한다.
        KeyDecision::EngageChord { trigger, key } => {
            drop(guard);
            if send_chord_engage(vkey_to_vk(trigger), vkey_to_vk(key)).is_err() {
                recover_engine_after_injection_failure();
                // chord가 실제로 생기지 않았으므로 현재 물리 키는 fail-open한다.
                CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param)
            } else {
                1
            }
        }
        // 코드 해제: 물리 트리거 up을 억제하고 주입 트리거 up으로 대체.
        // 지연 Launch는 해제를 즉시 주입한 뒤 slow worker에서 실행한다.
        KeyDecision::ReleaseChord {
            trigger,
            deferred_action,
        } => {
            drop(guard);
            let release_failed = !release_key_up_or_track(vkey_to_vk(trigger));
            if release_failed {
                // 합성 up 실패 시 물리 trigger up을 전달해 이미 내려간 수정자를
                // OS가 해제할 기회를 보존한다. tap-hold처럼 물리 키와 합성
                // modifier가 달라도 워치독이 기록된 합성 up을 계속 재시도한다.
                // 해제 전 지연 액션은 modifier 간섭을 되살리므로 실행하지 않는다.
                recover_engine_after_injection_failure();
                CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param)
            } else {
                if let Some(action) = deferred_action {
                    match action_lane(&action) {
                        ActionLane::Inline => {
                            if execute_inline_action(&action).is_err() {
                                recover_engine_after_injection_failure();
                            }
                        }
                        ActionLane::MouseWorker => {
                            let BindAction::Mouse(mb) = action else {
                                unreachable!("마우스 lane에는 마우스 액션만 들어와야 함")
                            };
                            queue_fast(FastJob::Pulse(mb));
                        }
                        ActionLane::Slow => queue_slow(action),
                    }
                }
                1
            }
        }
        // 마우스 바인딩 — 물리 키 이벤트를 억제하고 워커에 위임
        KeyDecision::MouseEngage(mb) => {
            drop(guard);
            queue_fast(FastJob::Engage(mb));
            1
        }
        KeyDecision::MouseRelease(mb) => {
            drop(guard);
            queue_fast(FastJob::Release(mb));
            1
        }
        KeyDecision::MouseStopAll => {
            drop(guard);
            queue_fast(FastJob::StopAll);
            1
        }
    }
}

// ── 훅 재설치 / 워치독 ──────────────────────────────────────────────────────

/// 죽은 훅을 떼고 다시 설치한다 (훅 스레드에서만 호출).
///
/// 훅이 사라진 동안의 keyup을 놓쳤을 수 있으므로 엔진의 일시 상태도 함께
/// 리셋한다 — 그러지 않으면 "LAlt를 누른 채 훅이 죽었다가 되살아난" 경우
/// 레이어가 계속 활성으로 남아 모든 키가 화살표로 나간다.
unsafe fn reinstall_hook(hook: &mut HHOOK) -> Result<(), String> {
    // 코드 모드로 주입해 둔 트리거 해제(stuck-Alt 방지) + 마우스 정지. 무엇을 할지는
    // 엔진이 정한다 — macOS 탭 비활성화와 같은 규칙(engine::on_hook_lost).
    // 마우스 정지는 예전엔 빠져 있어, 이동 키를 누른 채 훅이 재설치되면 포인터가
    // 계속 움직였다(초기화로 레이어가 꺼져 key-up이 MouseRelease가 되지 못함).
    let cleanup = HOOK_STATE
        .get()
        .and_then(|state| state.lock().ok().map(|mut guard| guard.on_hook_lost()));
    if let Some(cleanup) = cleanup {
        if let Some(trigger) = cleanup.release_trigger {
            release_key_up_or_track(vkey_to_vk(trigger));
        }
        if cleanup.stop_mouse {
            queue_fast(FastJob::StopAll);
        }
    }

    if UnhookWindowsHookEx(*hook) == 0 {
        let error = GetLastError();
        // 훅이 OS에서 이미 조용히 제거된 경우에도 실패할 수 있으므로 새 설치는
        // 계속 시도하되, 결과를 숨기지 않는다.
        tracing::warn!("기존 키보드 훅 해제 실패: OS 오류={error} — 재설치 계속");
    }
    let fresh = SetWindowsHookExW(
        WH_KEYBOARD_LL,
        Some(keyboard_hook_proc),
        std::ptr::null_mut(),
        0,
    );
    if fresh.is_null() {
        let error = GetLastError();
        let reason = format!("키보드 훅 재설치 실패 — OS 오류={error}");
        super::set_hook_error(Some(reason.clone()));
        tracing::error!("{reason}");
        let last_seen = HOOK_LAST_SEEN.load(Ordering::Relaxed);
        HOOK_LAST_SEEN.store(
            heartbeat_after_reinstall_failure(last_seen, GetTickCount()),
            Ordering::Relaxed,
        );
        HOOK_REINSTALL_PENDING.store(true, Ordering::SeqCst);
        return Err(reason);
    }
    *hook = fresh;
    super::set_hook_error(None);
    HOOK_LAST_SEEN.store(nonzero_tick(GetTickCount()), Ordering::Relaxed);
    HOOK_REINSTALL_PENDING.store(false, Ordering::SeqCst);
    HOOK_GENERATION.fetch_add(1, Ordering::SeqCst);
    let n = HOOK_REINSTALLS.fetch_add(1, Ordering::Relaxed) + 1;
    tracing::warn!(
        "키보드 훅 재설치 완료 (누적 {n}회) — OS가 훅을 조용히 제거했음 \
         (Modern Standby 복귀 / LowLevelHooksTimeout 초과 등)"
    );
    Ok(())
}

fn nonzero_tick(tick: u32) -> u32 {
    tick.max(1)
}

/// 재설치 실패를 "훅 미설치(0)"로 바꾸면 일반 프로브 판정에서 영구 제외된다.
/// 기존 하트비트를 유지하고, 비정상적으로 0이었다면 현재 tick의 non-zero 값을 쓴다.
fn heartbeat_after_reinstall_failure(last_seen: u32, now: u32) -> u32 {
    if last_seen == 0 {
        nonzero_tick(now)
    } else {
        last_seen
    }
}

fn reinstall_retry_due(now: u32, last_attempt: Option<u32>, delay_ms: u32) -> bool {
    match last_attempt {
        None => true,
        Some(previous) => now.wrapping_sub(previous) >= delay_ms,
    }
}

fn next_reinstall_retry_delay(current_ms: u32) -> u32 {
    current_ms.saturating_mul(2).min(REINSTALL_RETRY_MAX_MS)
}

#[derive(Default)]
struct DiagnosticsSnapshot {
    injection_failures: u32,
    slow_callbacks: u32,
    hook_panics: u32,
    fast_queue_max: u32,
    slow_queue_max: u32,
    queue_drops: u32,
}

/// 훅 콜백에서는 원자 카운터만 갱신하고, 실제 로그 출력은 워치독에서 수행한다.
fn report_hook_diagnostics(previous: &mut DiagnosticsSnapshot) {
    let injection_failures = INJECTION_FAILURES.load(Ordering::Relaxed);
    if injection_failures != previous.injection_failures {
        tracing::error!(
            "SendInput 실패 누적={injection_failures}, 최근 요청={}, 처리={}, OS 오류={}",
            LAST_INJECTION_EXPECTED.load(Ordering::Relaxed),
            LAST_INJECTION_SENT.load(Ordering::Relaxed),
            LAST_INJECTION_ERROR.load(Ordering::Relaxed)
        );
        previous.injection_failures = injection_failures;
    }

    let slow_callbacks = HOOK_SLOW_CALLBACKS.load(Ordering::Relaxed);
    if slow_callbacks != previous.slow_callbacks {
        tracing::warn!(
            "키보드 훅 지연 누적={slow_callbacks}, 최대={}ms",
            HOOK_MAX_LATENCY_MS.load(Ordering::Relaxed)
        );
        previous.slow_callbacks = slow_callbacks;
    }

    let hook_panics = HOOK_PANICS.load(Ordering::Relaxed);
    if hook_panics != previous.hook_panics {
        tracing::error!("키보드 훅 콜백 패닉 누적={hook_panics} — 이벤트는 통과 처리됨");
        previous.hook_panics = hook_panics;
    }

    let fast_queue_max = FAST_QUEUE_MAX_DEPTH.load(Ordering::Relaxed);
    if fast_queue_max >= QUEUE_WARN_DEPTH && fast_queue_max != previous.fast_queue_max {
        tracing::warn!("입력 fast 큐 최대 적체={fast_queue_max}");
    }
    previous.fast_queue_max = fast_queue_max;

    let slow_queue_max = SLOW_QUEUE_MAX_DEPTH.load(Ordering::Relaxed);
    if slow_queue_max >= QUEUE_WARN_DEPTH && slow_queue_max != previous.slow_queue_max {
        tracing::warn!("느린 액션 큐 최대 적체={slow_queue_max}");
    }
    previous.slow_queue_max = slow_queue_max;

    let queue_drops = QUEUE_DROPS.load(Ordering::Relaxed);
    if queue_drops != previous.queue_drops {
        tracing::warn!("입력 작업 큐 전달 실패 누적={queue_drops}");
        previous.queue_drops = queue_drops;
    }
}

/// 훅 생존 워치독.
///
/// "사용자는 입력 중인데(GetLastInputInfo가 최신) 우리 훅만 조용하다"를
/// 의심 신호로 삼고, 무해한 키(VK_NONAME = 0xFC, 예약된 no-op)를 주입해
/// 콜백이 도는지 확인한다. 콜백 최상단에서 주입 이벤트로도 하트비트를
/// 남기므로 이 프로브는 훅 생존 여부를 확정적으로 알려준다.
///
/// 프로브는 **시스템에 최근 입력이 있을 때만** 쏜다. 유휴 상태에서 주입하면
/// 유휴 타이머가 리셋돼 화면 꺼짐·절전이 걸리지 않기 때문이다.
fn spawn_hook_watchdog(running: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut last_probe: Option<u32> = None;
        let mut last_reinstall_attempt: Option<u32> = None;
        let mut reinstall_retry_delay = REINSTALL_RETRY_INITIAL_MS;
        let mut diagnostics = DiagnosticsSnapshot::default();
        while running.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(WATCHDOG_INTERVAL_MS));
            if !running.load(Ordering::Relaxed) {
                break;
            }

            let now = unsafe { GetTickCount() };
            report_hook_diagnostics(&mut diagnostics);
            retry_pending_synthetic_releases();

            // 이미 재설치 실패를 확인한 상태에서는 SendInput 프로브를 다시 쏘지
            // 않는다. 유휴 타이머를 계속 갱신하지 않고 메시지만 backoff 재전송한다.
            if HOOK_REINSTALL_PENDING.load(Ordering::SeqCst) {
                if reinstall_retry_due(now, last_reinstall_attempt, reinstall_retry_delay) {
                    let tid = HOOK_THREAD_ID.load(Ordering::SeqCst);
                    if tid != 0 {
                        let posted = unsafe {
                            PostThreadMessageW(
                                tid,
                                WM_KMD_REINSTALL_HOOK,
                                HOOK_GENERATION.load(Ordering::SeqCst) as usize,
                                HOOK_LAST_SEEN.load(Ordering::Relaxed) as isize,
                            )
                        };
                        if posted == 0 {
                            let error = unsafe { GetLastError() };
                            tracing::error!(
                                "키보드 훅 재설치 재시도 메시지 전송 실패: OS 오류={error}"
                            );
                        }
                    }
                    last_reinstall_attempt = Some(now);
                    reinstall_retry_delay = next_reinstall_retry_delay(reinstall_retry_delay);
                }
                continue;
            }
            last_reinstall_attempt = None;
            reinstall_retry_delay = REINSTALL_RETRY_INITIAL_MS;

            let last_hook = HOOK_LAST_SEEN.load(Ordering::Relaxed);
            let Some(last_input) = last_system_input_tick() else {
                continue;
            };
            if !should_probe_hook(now, last_hook, last_input, last_probe) {
                continue;
            }
            last_probe = Some(now);

            // ── 생존 프로브 ──
            // 콜백은 주입 이벤트로도 하트비트를 남기므로, 이 주입 뒤에도
            // HOOK_LAST_SEEN이 그대로면 훅이 체인에서 빠진 것이 확정된다.
            let before = HOOK_LAST_SEEN.load(Ordering::Relaxed);
            let generation = HOOK_GENERATION.load(Ordering::SeqCst);
            if send_key_press(VK_NONAME).is_err() {
                // 프로브 자체가 실패하면 훅 사망의 증거가 아니므로 재설치 금지.
                continue;
            }
            std::thread::sleep(std::time::Duration::from_millis(PROBE_WAIT_MS));
            if !probe_evidence_unchanged(
                generation,
                before,
                HOOK_GENERATION.load(Ordering::SeqCst),
                HOOK_LAST_SEEN.load(Ordering::Relaxed),
            ) {
                continue; // 살아 있다
            }

            // Post 직전 한 번 더 세대/하트비트를 확인한다. 메시지 처리 시 훅
            // 스레드가 같은 증거를 다시 확인해 큐 지연 중의 정상 회복도 거른다.
            let tid = HOOK_THREAD_ID.load(Ordering::SeqCst);
            if tid == 0
                || !running.load(Ordering::Relaxed)
                || !probe_evidence_unchanged(
                    generation,
                    before,
                    HOOK_GENERATION.load(Ordering::SeqCst),
                    HOOK_LAST_SEEN.load(Ordering::Relaxed),
                )
            {
                continue;
            }
            tracing::warn!("키보드 훅 무응답 감지 — 재설치 요청");
            let posted = unsafe {
                PostThreadMessageW(
                    tid,
                    WM_KMD_REINSTALL_HOOK,
                    generation as usize,
                    before as isize,
                )
            };
            if posted == 0 {
                let error = unsafe { GetLastError() };
                tracing::error!("키보드 훅 재설치 메시지 전송 실패: OS 오류={error}");
            }
        }
    })
}

/// 프로브 시작 뒤 훅 세대나 콜백 하트비트가 바뀌지 않았을 때만 재설치한다.
fn probe_evidence_unchanged(
    probe_generation: u32,
    probe_heartbeat: u32,
    current_generation: u32,
    current_heartbeat: u32,
) -> bool {
    probe_generation == current_generation && probe_heartbeat == current_heartbeat
}

/// 워치독 판정 (순수 함수 — 시각 산술만 담당해 단위 테스트 가능).
///
/// `last_hook == 0`은 훅 미설치를 뜻한다. 모든 시각은 GetTickCount 기반
/// u32로 49.7일마다 랩어라운드하므로, 차이는 `wrapping_sub` 후 `i32`로
/// 해석해 부호를 살린다 (훅이 시스템 입력보다 **앞선** 경우도 음수로 나온다).
fn should_probe_hook(now: u32, last_hook: u32, last_input: u32, last_probe: Option<u32>) -> bool {
    if last_hook == 0 {
        return false; // 훅 미설치 — 판단 대상 아님
    }
    // 훅이 시스템 입력보다 뒤처진 정도
    if (last_input.wrapping_sub(last_hook) as i32) < HOOK_STALE_MS {
        return false; // 정상 — 훅이 최근 입력을 보고 있다
    }
    // 사용자가 자리를 비웠으면 판단 보류. 유휴 상태에서 프로브를 주입하면
    // 시스템 유휴 타이머가 리셋돼 화면 꺼짐·절전이 걸리지 않는다.
    if (now.wrapping_sub(last_input) as i32) > IDLE_SKIP_MS {
        return false;
    }
    // 프로브 레이트 제한 — 마우스만 쓰는 구간에서 매 주기 주입하지 않도록
    !matches!(
        last_probe,
        Some(prev) if (now.wrapping_sub(prev) as i32) < PROBE_MIN_INTERVAL_MS
    )
}

// ── Backend 구현 ────────────────────────────────────────────────────────────

pub struct WindowsKeyboardBackend {
    running: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    fast_worker: Option<std::thread::JoinHandle<()>>,
    slow_worker: Option<std::thread::JoinHandle<()>>,
    watchdog: Option<std::thread::JoinHandle<()>>,
}

impl WindowsKeyboardBackend {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            thread: None,
            fast_worker: None,
            slow_worker: None,
            watchdog: None,
        }
    }
}

impl KeyboardBackend for WindowsKeyboardBackend {
    fn start(&mut self, config: KeybindConfig) -> Result<(), String> {
        if self.running.load(Ordering::Relaxed) {
            return Err("키 바인딩이 이미 실행 중입니다.".into());
        }

        // 글로벌 상태 설정 (콜백에서 접근).
        // OnceLock은 최초 1회만 set 가능하므로, 재시작 시에는 기존 Arc 내부의
        // 상태를 새 config로 교체한다 — 그렇지 않으면 stop/start 후에도
        // 이전 설정이 계속 적용된다.
        let new_state = EngineState::new(config);
        if let Some(existing) = HOOK_STATE.get() {
            match existing.lock() {
                Ok(mut guard) => *guard = new_state,
                Err(_) => return Err("훅 상태 잠금 실패".into()),
            }
        } else {
            let _ = HOOK_STATE.set(Arc::new(Mutex::new(new_state)));
        }

        // 상태형 마우스 fast worker와 파일/IPC/프로세스 slow worker를 분리한다.
        // slow 작업이 멎어도 key-up/chord release는 훅 fast path에서 계속 처리된다.
        let (fast_tx, fast_rx) = mpsc::channel::<FastJob>();
        let (slow_tx, slow_rx) = mpsc::channel::<BindAction>();
        {
            let mut fast_guard = FAST_TX
                .lock()
                .map_err(|_| "입력 fast 큐 잠금 실패".to_string())?;
            let mut slow_guard = SLOW_TX
                .lock()
                .map_err(|_| "느린 액션 큐 잠금 실패".to_string())?;
            *fast_guard = Some(fast_tx);
            *slow_guard = Some(slow_tx);
        }
        FAST_QUEUE_DEPTH.store(0, Ordering::Relaxed);
        SLOW_QUEUE_DEPTH.store(0, Ordering::Relaxed);
        FAST_QUEUE_MAX_DEPTH.store(0, Ordering::Relaxed);
        SLOW_QUEUE_MAX_DEPTH.store(0, Ordering::Relaxed);
        self.fast_worker = Some(std::thread::spawn(move || {
            // 모션 워커(이동/휠 틱 스레드)와 버튼 상태는 fast worker가 소유한다.
            let motion = MouseWorker::start(WinMouseSink);
            let mut pressed_buttons = lock_pending(pending_mouse_releases()).items();

            loop {
                match fast_rx.recv_timeout(std::time::Duration::from_millis(MOUSE_RELEASE_RETRY_MS))
                {
                    Ok(job) => {
                        FAST_QUEUE_DEPTH.fetch_sub(1, Ordering::Relaxed);
                        match job {
                            FastJob::Engage(mb) => {
                                if is_mouse_button(mb) {
                                    lock_pending(pending_mouse_releases()).remove(mb);
                                    if !pressed_buttons.contains(&mb)
                                        && send_mouse_button(mb, true).is_ok()
                                    {
                                        pressed_buttons.push(mb);
                                    }
                                } else {
                                    motion.engage(mb);
                                }
                            }
                            FastJob::Release(mb) => {
                                if is_mouse_button(mb) {
                                    if pressed_buttons.contains(&mb)
                                        && release_mouse_button_or_track(mb)
                                    {
                                        pressed_buttons.retain(|&button| button != mb);
                                    }
                                } else {
                                    motion.release(mb);
                                }
                            }
                            FastJob::StopAll => {
                                motion.stop_all();
                                for &mb in &pressed_buttons {
                                    lock_pending(pending_mouse_releases()).record(mb);
                                }
                            }
                            FastJob::Pulse(mb) => {
                                let _ = execute_mouse_pulse(mb);
                            }
                        }
                        retry_pending_mouse_buttons(&mut pressed_buttons);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        retry_pending_mouse_buttons(&mut pressed_buttons);
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            // 종료 정리 — 실패 항목은 전역 추적에 남겨 다음 start에서도 재시도한다.
            for &mb in &pressed_buttons {
                lock_pending(pending_mouse_releases()).record(mb);
            }
            for _ in 0..STOP_RELEASE_RETRY_PASSES {
                retry_pending_mouse_buttons(&mut pressed_buttons);
            }
            tracing::debug!("입력 fast worker 종료");
        }));
        self.slow_worker = Some(std::thread::spawn(move || {
            for action in slow_rx {
                SLOW_QUEUE_DEPTH.fetch_sub(1, Ordering::Relaxed);
                execute_slow_action(action);
            }
            tracing::debug!("느린 액션 worker 종료");
        }));

        let running = self.running.clone();
        running.store(true, Ordering::Relaxed);
        HOOK_THREAD_ID.store(0, Ordering::SeqCst);
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);

        let thread = std::thread::spawn(move || {
            unsafe {
                let mut hook = SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(keyboard_hook_proc),
                    std::ptr::null_mut(),
                    0,
                );

                if hook.is_null() {
                    let reason = format!(
                        "키보드 훅 설치 실패 — SetWindowsHookExW 실패: OS 오류={}",
                        GetLastError()
                    );
                    super::set_hook_error(Some(reason.clone()));
                    tracing::error!("SetWindowsHookExW 실패");
                    running.store(false, Ordering::Relaxed);
                    let _ = ready_tx.send(Err(reason));
                    return;
                }

                super::set_hook_error(None);
                tracing::info!("키보드 훅 설치 완료");
                HOOK_LAST_SEEN.store(nonzero_tick(GetTickCount()), Ordering::Relaxed);
                HOOK_REINSTALL_PENDING.store(false, Ordering::SeqCst);
                HOOK_GENERATION.fetch_add(1, Ordering::SeqCst);

                // PostThreadMessageW가 안정적으로 동작하도록 메시지 큐를 먼저 만든다.
                // stop()이 WM_QUIT를 보낼 수 있도록 그 뒤에만 스레드 ID를 공개한다.
                let mut msg: MSG = std::mem::zeroed();
                PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
                HOOK_THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);
                let _ = ready_tx.send(Ok(()));

                // 메시지 루프 (훅이 동작하려면 필수).
                // GetMessageW 블로킹 대기 — 기존 PeekMessageW + 1ms sleep은
                // 상시 데몬이 초당 ~1000회 깨어나는 busy-wait였다.
                // WM_QUIT(ret==0) 또는 에러(ret==-1)에서 종료.
                loop {
                    let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
                    if ret == 0 {
                        break;
                    }
                    if ret == -1 {
                        tracing::error!("키보드 훅 메시지 루프 실패: OS 오류={}", GetLastError());
                        break;
                    }
                    // 워치독이 훅 사망을 확인하면 이 메시지로 재설치를 요청한다.
                    // (LL 훅은 설치한 스레드에 묶이므로 반드시 여기서 처리)
                    if msg.message == WM_KMD_REINSTALL_HOOK {
                        let probe_generation = msg.wParam as u32;
                        let probe_heartbeat = msg.lParam as u32;
                        if running.load(Ordering::Relaxed)
                            && (HOOK_REINSTALL_PENDING.load(Ordering::SeqCst)
                                || probe_evidence_unchanged(
                                    probe_generation,
                                    probe_heartbeat,
                                    HOOK_GENERATION.load(Ordering::SeqCst),
                                    HOOK_LAST_SEEN.load(Ordering::Relaxed),
                                ))
                        {
                            let _ = reinstall_hook(&mut hook);
                        } else {
                            tracing::debug!("정상 회복된 훅의 오래된 재설치 요청 무시");
                        }
                        continue;
                    }
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                HOOK_THREAD_ID.store(0, Ordering::SeqCst);
                running.store(false, Ordering::Relaxed);
                if UnhookWindowsHookEx(hook) == 0 {
                    tracing::warn!("키보드 훅 해제 실패: OS 오류={}", GetLastError());
                }
                HOOK_LAST_SEEN.store(0, Ordering::Relaxed);
                HOOK_REINSTALL_PENDING.store(false, Ordering::SeqCst);
                tracing::info!("키보드 훅 해제 완료");
            }
        });

        self.thread = Some(thread);
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => {
                let _ = self.stop();
                return Err(reason);
            }
            Err(_) => {
                let reason = "키보드 훅 준비 신호 수신 실패".to_string();
                super::set_hook_error(Some(reason.clone()));
                let _ = self.stop();
                return Err(reason);
            }
        }
        self.watchdog = Some(spawn_hook_watchdog(self.running.clone()));
        Ok(())
    }

    fn stop(&mut self) -> Result<(), String> {
        self.running.store(false, Ordering::Relaxed);
        // GetMessageW에서 블로킹 중인 메시지 루프를 WM_QUIT로 깨운다
        let tid = HOOK_THREAD_ID.load(Ordering::SeqCst);
        if tid != 0 {
            let posted = unsafe { PostThreadMessageW(tid, WM_QUIT, 0, 0) };
            if posted == 0 {
                let error = unsafe { GetLastError() };
                tracing::error!("키보드 훅 종료 메시지 전송 실패: OS 오류={error}");
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // 워치독은 running=false를 보고 다음 주기에 빠져나온다
        if let Some(watchdog) = self.watchdog.take() {
            let _ = watchdog.join();
        }
        // 코드 모드 중 중지된 경우 주입돼 있는 트리거를 해제한다 (stuck-Alt 방지).
        // slow worker join보다 먼저 수행해 느린 클립보드 작업에 해제가 막히지 않게 한다.
        // (engine::on_hook_lost — macOS stop과 같은 규칙. 마우스는 아래 worker 정리가 멈춘다)
        if let Some(state) = HOOK_STATE.get() {
            if let Ok(mut guard) = state.lock() {
                if let Some(trigger) = guard.on_hook_lost().release_trigger {
                    tracing::info!("중지 시 코드 트리거 해제: {trigger:?}");
                    release_key_up_or_track(vkey_to_vk(trigger));
                }
            }
        }
        for _ in 0..STOP_RELEASE_RETRY_PASSES {
            retry_pending_synthetic_releases();
        }
        // 센더를 드롭하면 각 worker의 recv 루프가 끝난다.
        if let Ok(mut g) = FAST_TX.lock() {
            *g = None;
        }
        if let Ok(mut g) = SLOW_TX.lock() {
            *g = None;
        }
        if let Some(worker) = self.fast_worker.take() {
            let _ = worker.join();
        }
        if let Some(worker) = self.slow_worker.take() {
            let _ = worker.join();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 자동 생성 역변환의 정합성: 모든 VKey가 왕복 변환에서 자기 자신으로
    /// 돌아와야 한다. VK 코드가 겹치면(비단사) 여기서 즉시 실패한다.
    #[test]
    fn vk_왕복_변환_일치() {
        for &k in VKey::ALL {
            assert_eq!(vk_to_vkey(vkey_to_vk(k)), Some(k), "{k:?} 왕복 불일치");
        }
    }

    // ── 합성 입력 순서 / 실패 정리 ──

    #[test]
    fn 키_press는_down_up_단일_batch_순서() {
        assert_eq!(
            key_press_events(VK_ESCAPE),
            [KeyboardEvent::Down(VK_ESCAPE), KeyboardEvent::Up(VK_ESCAPE)]
        );
    }

    #[test]
    fn 콤보는_수정자_down부터_역순_up까지_원자_순서() {
        assert_eq!(
            combo_events(&[VK_LCONTROL, VK_LSHIFT], 0x41),
            vec![
                KeyboardEvent::Down(VK_LCONTROL),
                KeyboardEvent::Down(VK_LSHIFT),
                KeyboardEvent::Down(0x41),
                KeyboardEvent::Up(0x41),
                KeyboardEvent::Up(VK_LSHIFT),
                KeyboardEvent::Up(VK_LCONTROL),
            ]
        );
    }

    #[test]
    fn 부분_주입_실패는_실제로_내려간_키만_역순_해제() {
        let events = combo_events(&[VK_LCONTROL, VK_LSHIFT], 0x41);
        assert_eq!(
            cleanup_events_for_accepted_prefix(&events, 3),
            vec![
                KeyboardEvent::Up(0x41),
                KeyboardEvent::Up(VK_LSHIFT),
                KeyboardEvent::Up(VK_LCONTROL),
            ]
        );
        assert!(
            cleanup_events_for_accepted_prefix(&events, events.len()).is_empty(),
            "정상 완료된 batch에는 추가 해제가 없어야 한다"
        );
    }

    #[test]
    fn 즉시_정리도_부분_실패하면_전송되지_않은_up을_재시도_대상으로_남긴다() {
        let cleanup = vec![
            KeyboardEvent::Up(0x41),
            KeyboardEvent::Up(VK_LSHIFT),
            KeyboardEvent::Up(VK_LCONTROL),
        ];
        assert_eq!(
            unaccepted_cleanup_releases(&cleanup, 1),
            vec![
                SyntheticRelease::VirtualKey(VK_LSHIFT),
                SyntheticRelease::VirtualKey(VK_LCONTROL),
            ],
            "첫 up만 성공했으면 나머지 두 키는 pending release여야 한다"
        );
        assert_eq!(
            unaccepted_cleanup_releases(&cleanup, 0),
            vec![
                SyntheticRelease::VirtualKey(0x41),
                SyntheticRelease::VirtualKey(VK_LSHIFT),
                SyntheticRelease::VirtualKey(VK_LCONTROL),
            ],
            "정리 호출 전체 실패면 모든 up을 추적해야 한다"
        );
    }

    #[test]
    fn 유니코드_unit_release도_일반_키와_같은_재시도_큐를_사용한다() {
        let pending = Mutex::new(PendingReleases::default());
        let release = SyntheticRelease::UnicodeUnit('한' as u16);
        lock_pending(&pending).record(release);

        retry_pending_releases(&pending, RELEASE_RETRY_BATCH, |_| false);
        assert_eq!(lock_pending(&pending).items(), vec![release]);
        retry_pending_releases(&pending, RELEASE_RETRY_BATCH, |_| true);
        assert!(lock_pending(&pending).items().is_empty());
    }

    #[test]
    fn 합성_release는_실패하면_추적을_유지하고_성공할_때만_제거한다() {
        let pending = Mutex::new(PendingReleases::default());
        lock_pending(&pending).record(VK_LCONTROL);

        let mut attempts = 0;
        retry_pending_releases(&pending, RELEASE_RETRY_BATCH, |_| {
            attempts += 1;
            false
        });
        assert_eq!(attempts, 1);
        assert_eq!(lock_pending(&pending).items(), vec![VK_LCONTROL]);

        retry_pending_releases(&pending, RELEASE_RETRY_BATCH, |_| true);
        assert!(lock_pending(&pending).items().is_empty());
    }

    #[test]
    fn release_재시도는_묶음_상한과_재기록_세대를_지킨다() {
        let pending = Mutex::new(PendingReleases::default());
        for vk in 1..=(RELEASE_RETRY_BATCH as u16 + 2) {
            lock_pending(&pending).record(vk);
        }
        let stale = lock_pending(&pending).snapshot(1)[0];
        lock_pending(&pending).record(stale.item);
        assert!(
            !lock_pending(&pending).complete(stale),
            "재시도 중 다시 실패한 release를 오래된 성공이 지우면 안 된다"
        );

        let mut attempts = 0;
        retry_pending_releases(&pending, RELEASE_RETRY_BATCH, |_| {
            attempts += 1;
            false
        });
        assert_eq!(attempts, RELEASE_RETRY_BATCH);
    }

    #[test]
    fn 마우스_release도_성공_전에는_추적에서_제거하지_않는다() {
        let pending = Mutex::new(PendingReleases::default());
        lock_pending(&pending).record(MouseBind::BtnLeft);
        retry_pending_releases(&pending, RELEASE_RETRY_BATCH, |_| false);
        assert_eq!(lock_pending(&pending).items(), vec![MouseBind::BtnLeft]);
        retry_pending_releases(&pending, RELEASE_RETRY_BATCH, |_| true);
        assert!(lock_pending(&pending).items().is_empty());
    }

    #[test]
    fn 일회성_클릭은_down만_수락되면_up_정리를_요구한다() {
        assert_eq!(
            mouse_release_for_accepted_click_prefix(MouseBind::BtnLeft, 0),
            None
        );
        assert_eq!(
            mouse_release_for_accepted_click_prefix(MouseBind::BtnLeft, 1),
            Some(MouseBind::BtnLeft)
        );
        assert_eq!(
            mouse_release_for_accepted_click_prefix(MouseBind::BtnLeft, 2),
            None
        );
    }

    #[test]
    fn 일회성_클릭_정리_batch_실패는_pending_up으로_남는다() {
        let pending = Mutex::new(PendingReleases::default());
        let bind =
            mouse_release_for_accepted_click_prefix(MouseBind::BtnRight, 1).expect("down 수락");

        assert!(!track_mouse_release_result(&pending, bind, false));
        assert_eq!(lock_pending(&pending).items(), vec![MouseBind::BtnRight]);
        assert!(track_mouse_release_result(&pending, bind, true));
        assert!(
            lock_pending(&pending).items().is_empty(),
            "정리 up이 성공한 뒤에만 pending에서 제거"
        );
    }

    #[test]
    fn 액션을_inline_mouse_slow_lane으로_분리() {
        assert_eq!(
            action_lane(&BindAction::SendKey(VKey::Escape)),
            ActionLane::Inline
        );
        assert_eq!(
            action_lane(&BindAction::Mouse(MouseBind::BtnLeft)),
            ActionLane::MouseWorker
        );
        assert_eq!(
            action_lane(&BindAction::Launch("kmd-desktop".into())),
            ActionLane::Slow
        );
        assert_eq!(action_lane(&BindAction::ClipPaste(1)), ActionLane::Slow);
    }

    // ── 훅 워치독 판정 ──
    //
    // OS가 LL 훅을 조용히 제거하는 사고(Modern Standby 복귀 등)를 감지하는
    // 조건식. 시각 산술이 전부라 여기서 검증한다.

    #[test]
    fn 훅이_최근_입력을_보고_있으면_프로브_안함() {
        // 사용자가 방금 친 키를 훅도 봤다 → 정상
        let now = 100_000;
        assert!(!should_probe_hook(now, now - 100, now - 100, None));
    }

    #[test]
    fn 입력은_있는데_훅만_조용하면_프로브() {
        // 시스템은 0.2초 전 입력을 봤는데 훅은 10초째 조용 → 사망 의심
        let now = 100_000;
        assert!(should_probe_hook(now, now - 10_000, now - 200, None));
    }

    #[test]
    fn 프로브_뒤_세대나_하트비트가_바뀌면_재설치_취소() {
        assert!(probe_evidence_unchanged(7, 1000, 7, 1000));
        assert!(!probe_evidence_unchanged(7, 1000, 8, 1000));
        assert!(!probe_evidence_unchanged(7, 1000, 7, 1001));
    }

    #[test]
    fn 훅_미설치면_판단_대상_아님() {
        let now = 100_000;
        assert!(!should_probe_hook(now, 0, now - 200, None));
    }

    #[test]
    fn 재설치_실패는_하트비트를_0으로_만들지_않는다() {
        assert_eq!(heartbeat_after_reinstall_failure(42_000, 50_000), 42_000);
        assert_eq!(heartbeat_after_reinstall_failure(0, 50_000), 50_000);
        assert_eq!(heartbeat_after_reinstall_failure(0, 0), 1);
    }

    #[test]
    fn 재설치_재시도는_지수_backoff와_상한을_지킨다() {
        let now = 100_000;
        assert!(reinstall_retry_due(now, None, REINSTALL_RETRY_INITIAL_MS));
        assert!(!reinstall_retry_due(
            now,
            Some(now - 1_000),
            REINSTALL_RETRY_INITIAL_MS
        ));
        assert!(reinstall_retry_due(
            now,
            Some(now - REINSTALL_RETRY_INITIAL_MS),
            REINSTALL_RETRY_INITIAL_MS
        ));
        assert_eq!(next_reinstall_retry_delay(2_000), 4_000);
        assert_eq!(
            next_reinstall_retry_delay(REINSTALL_RETRY_MAX_MS),
            REINSTALL_RETRY_MAX_MS
        );
    }

    #[test]
    fn 자리비움_상태에서는_프로브_금지() {
        // 유휴 상태에서 키를 주입하면 시스템 유휴 타이머가 리셋돼
        // 화면 꺼짐·절전이 영영 안 걸린다 — 반드시 걸러야 한다.
        let now = 100_000;
        let last_input = now - (IDLE_SKIP_MS as u32) - 1_000;
        assert!(!should_probe_hook(
            now,
            last_input - 10_000,
            last_input,
            None
        ));
    }

    #[test]
    fn 프로브_레이트_제한() {
        let now = 100_000;
        let stale_hook = now - 10_000;
        let fresh_input = now - 200;
        // 직전 프로브 직후 → 억제
        assert!(!should_probe_hook(
            now,
            stale_hook,
            fresh_input,
            Some(now - 1_000)
        ));
        // 최소 간격 경과 → 허용
        assert!(should_probe_hook(
            now,
            stale_hook,
            fresh_input,
            Some(now - (PROBE_MIN_INTERVAL_MS as u32) - 1)
        ));
    }

    #[test]
    fn 틱_랩어라운드_구간에서도_정상_판정() {
        // GetTickCount는 49.7일마다 0으로 되감긴다. 그 경계에서 뺄셈이
        // 거대한 양수가 되면 "멀쩡한 훅"을 죽은 것으로 오판한다.
        let now = 500u32; // 랩어라운드 직후
        let last_hook = u32::MAX - 500; // 되감기 직전 (실제로는 1초 전)
        let last_input = u32::MAX - 400;
        assert!(
            !should_probe_hook(now, last_hook, last_input, None),
            "랩어라운드 경계에서 정상 훅을 사망으로 오판하면 안 된다"
        );

        // 같은 경계에서 진짜로 뒤처진 경우는 정상적으로 잡아낸다
        let stale_hook = u32::MAX - 20_000;
        assert!(should_probe_hook(now, stale_hook, now - 200, None));
    }
}
