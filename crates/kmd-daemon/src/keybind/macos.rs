//! macOS 키 바인딩 백엔드 — CGEventTap 글로벌 키보드 훅
//!
//! Core Graphics CGEventTap으로 키 이벤트를 가로채고,
//! 바인딩 테이블에 따라 키를 리매핑하거나 억제한다.
//!
//! 필수: 시스템 설정 > 개인 정보 보호 및 보안 > 손쉬운 사용 권한

use super::engine::{EngineState, KeyDecision};
use super::mouse::{MouseSink, MouseWorker};
use super::{
    resolve_launch_cmd, BindAction, KeybindConfig, KeyboardBackend, MacroStep, MouseBind, VKey,
};
use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Instant;

// ── Core Graphics / Core Foundation FFI ──────────────────────────────────────

type CGEventRef = *mut c_void;
type CGEventSourceRef = *mut c_void;
type CFMachPortRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFStringRef = *const c_void;
type CGEventTapProxy = *mut c_void;

const CG_EVENT_KEY_DOWN: u32 = 10;
const CG_EVENT_KEY_UP: u32 = 11;
const CG_EVENT_FLAGS_CHANGED: u32 = 12;
const CG_EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
/// 입력 폭주 시 OS가 탭을 꺼버릴 때 오는 타입. 타임아웃과 마찬가지로
/// 재활성화하지 않으면 재시작 전까지 훅이 조용히 죽는다.
const CG_EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

const CG_SESSION_EVENT_TAP: u32 = 1;
const CG_HEAD_INSERT_EVENT_TAP: u32 = 0;
const CG_EVENT_TAP_OPTION_DEFAULT: u32 = 0;

const CG_EVENT_FLAG_MASK_SHIFT: u64 = 0x0002_0000;
const CG_EVENT_FLAG_MASK_CONTROL: u64 = 0x0004_0000;
const CG_EVENT_FLAG_MASK_ALTERNATE: u64 = 0x0008_0000;
const CG_EVENT_FLAG_MASK_COMMAND: u64 = 0x0010_0000;

const CG_KEYBOARD_EVENT_KEYCODE: u32 = 9;
pub(crate) const CG_EVENT_SOURCE_USER_DATA: u32 = 42;
const CG_EVENT_SOURCE_STATE_PRIVATE: i32 = -1;

/// 자체 생성 이벤트 식별용 매직 넘버
pub(crate) const MAGIC_USER_DATA: i64 = 0x6B6D6400; // "kmd\0"

type CGEventTapCallBack = unsafe extern "C" fn(
    proxy: CGEventTapProxy,
    type_: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef;

#[link(name = "CoreGraphics", kind = "framework")]
#[allow(dead_code)]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: CGEventTapCallBack,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventSourceCreate(state_id: i32) -> CGEventSourceRef;
    fn CGEventCreateKeyboardEvent(
        source: CGEventSourceRef,
        virtual_key: u16,
        key_down: bool,
    ) -> CGEventRef;
    fn CGEventPost(tap: u32, event: CGEventRef);
    fn CGEventSetFlags(event: CGEventRef, flags: u64);
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);
    fn CGEventSetType(event: CGEventRef, type_: u32);
    /// 실제 HID 수정자 플래그 조회 (state_id=1: kCGEventSourceStateHIDSystemState)
    fn CGEventSourceFlagsState(state_id: i32) -> u64;
    fn CGEventCreate(source: CGEventSourceRef) -> CGEventRef;
    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    fn CGEventCreateMouseEvent(
        source: CGEventSourceRef,
        mouse_type: u32,
        position: CGPoint,
        button: u32,
    ) -> CGEventRef;
    /// C 가변 인자 함수 — wheel_count=1이면 wheel1만 읽는다
    fn CGEventCreateScrollWheelEvent(
        source: CGEventSourceRef,
        units: u32,
        wheel_count: u32,
        wheel1: i32,
        ...
    ) -> CGEventRef;
}

/// Core Graphics 좌표 (좌상단 원점, 포인트 단위)
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

// ── 마우스 이벤트 타입/버튼 상수 ─────────────────────────────────────────────

const CG_EVENT_LEFT_MOUSE_DOWN: u32 = 1;
const CG_EVENT_LEFT_MOUSE_UP: u32 = 2;
const CG_EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
const CG_EVENT_RIGHT_MOUSE_UP: u32 = 4;
const CG_EVENT_MOUSE_MOVED: u32 = 5;
const CG_EVENT_LEFT_MOUSE_DRAGGED: u32 = 6;
const CG_EVENT_RIGHT_MOUSE_DRAGGED: u32 = 7;
const CG_EVENT_OTHER_MOUSE_DOWN: u32 = 25;
const CG_EVENT_OTHER_MOUSE_UP: u32 = 26;
const CG_EVENT_OTHER_MOUSE_DRAGGED: u32 = 27;

const CG_MOUSE_BUTTON_LEFT: u32 = 0;
const CG_MOUSE_BUTTON_RIGHT: u32 = 1;
const CG_MOUSE_BUTTON_CENTER: u32 = 2;

/// 스크롤 단위: 라인 (kCGScrollEventUnitLine)
const CG_SCROLL_EVENT_UNIT_LINE: u32 = 1;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        num_values: i64,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> *mut c_void;
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        c_str: *const u8,
        encoding: u32,
    ) -> *const c_void;
    static kCFBooleanTrue: *const c_void;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: CFMachPortRef,
        order: i64,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRun();
    fn CFRunLoopStop(rl: CFRunLoopRef);
    fn CFRelease(cf: *mut c_void);
    static kCFRunLoopCommonModes: CFStringRef;
    fn CFArrayGetCount(arr: *const c_void) -> i64;
    fn CFArrayGetValueAtIndex(arr: *const c_void, idx: i64) -> *const c_void;
    fn CFStringCompare(s1: *const c_void, s2: *const c_void, opts: u64) -> i32;
}

// ── Carbon TIS (Text Input Source) API — 한/영 입력 소스 전환 ────────────────

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn TISCopyCurrentKeyboardInputSource() -> *mut c_void;
    fn TISGetInputSourceProperty(source: *mut c_void, key: *const c_void) -> *const c_void;
    static kTISPropertyInputSourceLanguages: *const c_void;
}

// ── VKey → macOS CGKeyCode 변환 ──────────────────────────────────────────────

pub(crate) fn vkey_to_cg(key: VKey) -> u16 {
    match key {
        VKey::A => 0x00,
        VKey::S => 0x01,
        VKey::D => 0x02,
        VKey::F => 0x03,
        VKey::H => 0x04,
        VKey::G => 0x05,
        VKey::Z => 0x06,
        VKey::X => 0x07,
        VKey::C => 0x08,
        VKey::V => 0x09,
        VKey::B => 0x0B,
        VKey::Q => 0x0C,
        VKey::W => 0x0D,
        VKey::E => 0x0E,
        VKey::R => 0x0F,
        VKey::Y => 0x10,
        VKey::T => 0x11,
        VKey::O => 0x1F,
        VKey::U => 0x20,
        VKey::I => 0x22,
        VKey::P => 0x23,
        VKey::L => 0x25,
        VKey::J => 0x26,
        VKey::K => 0x28,
        VKey::N => 0x2D,
        VKey::M => 0x2E,
        VKey::Num0 => 0x1D,
        VKey::Num1 => 0x12,
        VKey::Num2 => 0x13,
        VKey::Num3 => 0x14,
        VKey::Num4 => 0x15,
        VKey::Num5 => 0x17,
        VKey::Num6 => 0x16,
        VKey::Num7 => 0x1A,
        VKey::Num8 => 0x1C,
        VKey::Num9 => 0x19,
        VKey::F1 => 0x7A,
        VKey::F2 => 0x78,
        VKey::F3 => 0x63,
        VKey::F4 => 0x76,
        VKey::F5 => 0x60,
        VKey::F6 => 0x61,
        VKey::F7 => 0x62,
        VKey::F8 => 0x64,
        VKey::F9 => 0x65,
        VKey::F10 => 0x6D,
        VKey::F11 => 0x67,
        VKey::F12 => 0x6F,
        VKey::F19 => 0x50,
        VKey::Escape => 0x35,
        VKey::Tab => 0x30,
        VKey::CapsLock => 0x39,
        VKey::Space => 0x31,
        VKey::Enter => 0x24,
        VKey::Backspace => 0x33,
        VKey::Delete => 0x75,
        VKey::Left => 0x7B,
        VKey::Right => 0x7C,
        VKey::Up => 0x7E,
        VKey::Down => 0x7D,
        VKey::Home => 0x73,
        VKey::End => 0x77,
        VKey::PageUp => 0x74,
        VKey::PageDown => 0x79,
        VKey::Insert => 0x72,
        VKey::PrintScreen => 0x69,
        VKey::ScrollLock => 0x6B,
        VKey::Pause => 0x71,
        VKey::LShift => 0x38,
        VKey::RShift => 0x3C,
        VKey::LCtrl => 0x3B,
        VKey::RCtrl => 0x3E,
        VKey::LAlt => 0x3A,
        VKey::RAlt => 0x3D,
        VKey::LWin => 0x37,
        VKey::RWin => 0x36,
        VKey::Semicolon => 0x29,
        VKey::Quote => 0x27,
        VKey::Comma => 0x2B,
        VKey::Period => 0x2F,
        VKey::Slash => 0x2C,
        VKey::Backslash => 0x2A,
        VKey::LBracket => 0x21,
        VKey::RBracket => 0x1E,
        VKey::Minus => 0x1B,
        VKey::Equal => 0x18,
        VKey::Grave => 0x32,
        VKey::Hangul => 0x68,
        VKey::Hanja => 0x68,
    }
}

/// macOS CGKeyCode → VKey 역변환.
///
/// 주의: Windows(vk_to_vkey)와 달리 정방향에서 **자동 생성하지 않는다**.
/// macOS 키코드는 전단사가 아니기 때문이다:
/// - Hangul/Hanja가 둘 다 0x68(JIS kVK_JIS_Kana 대응)로 겹친다 — 역방향에서
///   임의로 한쪽을 고르면 안 되므로 의도적으로 제외
/// - Insert(0x72=Help)/PrintScreen/ScrollLock/Pause 등은 주입(정방향)용으로만
///   두고, 물리 입력 가로채기(역방향)에서는 의도적으로 제외
///
/// 즉 이 match의 "누락"은 실수가 아니라 정책이다. 키를 추가할 때는
/// 양방향 모두 필요한지 판단해서 각각 넣는다.
fn cg_to_vkey(keycode: u16) -> Option<VKey> {
    match keycode {
        0x00 => Some(VKey::A),
        0x01 => Some(VKey::S),
        0x02 => Some(VKey::D),
        0x03 => Some(VKey::F),
        0x04 => Some(VKey::H),
        0x05 => Some(VKey::G),
        0x06 => Some(VKey::Z),
        0x07 => Some(VKey::X),
        0x08 => Some(VKey::C),
        0x09 => Some(VKey::V),
        0x0B => Some(VKey::B),
        0x0C => Some(VKey::Q),
        0x0D => Some(VKey::W),
        0x0E => Some(VKey::E),
        0x0F => Some(VKey::R),
        0x10 => Some(VKey::Y),
        0x11 => Some(VKey::T),
        0x1F => Some(VKey::O),
        0x20 => Some(VKey::U),
        0x22 => Some(VKey::I),
        0x23 => Some(VKey::P),
        0x25 => Some(VKey::L),
        0x26 => Some(VKey::J),
        0x28 => Some(VKey::K),
        0x2D => Some(VKey::N),
        0x2E => Some(VKey::M),
        0x1D => Some(VKey::Num0),
        0x12 => Some(VKey::Num1),
        0x13 => Some(VKey::Num2),
        0x14 => Some(VKey::Num3),
        0x15 => Some(VKey::Num4),
        0x17 => Some(VKey::Num5),
        0x16 => Some(VKey::Num6),
        0x1A => Some(VKey::Num7),
        0x1C => Some(VKey::Num8),
        0x19 => Some(VKey::Num9),
        0x7A => Some(VKey::F1),
        0x78 => Some(VKey::F2),
        0x63 => Some(VKey::F3),
        0x76 => Some(VKey::F4),
        0x60 => Some(VKey::F5),
        0x61 => Some(VKey::F6),
        0x62 => Some(VKey::F7),
        0x64 => Some(VKey::F8),
        0x65 => Some(VKey::F9),
        0x6D => Some(VKey::F10),
        0x67 => Some(VKey::F11),
        0x6F => Some(VKey::F12),
        0x50 => Some(VKey::F19),
        0x35 => Some(VKey::Escape),
        0x30 => Some(VKey::Tab),
        0x39 => Some(VKey::CapsLock),
        0x31 => Some(VKey::Space),
        0x24 => Some(VKey::Enter),
        0x33 => Some(VKey::Backspace),
        0x75 => Some(VKey::Delete),
        0x7B => Some(VKey::Left),
        0x7C => Some(VKey::Right),
        0x7E => Some(VKey::Up),
        0x7D => Some(VKey::Down),
        0x73 => Some(VKey::Home),
        0x77 => Some(VKey::End),
        0x74 => Some(VKey::PageUp),
        0x79 => Some(VKey::PageDown),
        0x38 => Some(VKey::LShift),
        0x3C => Some(VKey::RShift),
        0x3B => Some(VKey::LCtrl),
        0x3E => Some(VKey::RCtrl),
        0x3A => Some(VKey::LAlt),
        0x3D => Some(VKey::RAlt),
        0x37 => Some(VKey::LWin),
        0x36 => Some(VKey::RWin),
        0x29 => Some(VKey::Semicolon),
        0x27 => Some(VKey::Quote),
        0x2B => Some(VKey::Comma),
        0x2F => Some(VKey::Period),
        0x2C => Some(VKey::Slash),
        0x2A => Some(VKey::Backslash),
        0x21 => Some(VKey::LBracket),
        0x1E => Some(VKey::RBracket),
        0x1B => Some(VKey::Minus),
        0x18 => Some(VKey::Equal),
        0x32 => Some(VKey::Grave),
        _ => None,
    }
}

// ── 키 시뮬레이션 ────────────────────────────────────────────────────────────

fn modifiers_to_flags(mods: &[VKey]) -> u64 {
    let mut flags = 0u64;
    for m in mods {
        match m {
            VKey::LShift | VKey::RShift => flags |= CG_EVENT_FLAG_MASK_SHIFT,
            VKey::LCtrl | VKey::RCtrl => flags |= CG_EVENT_FLAG_MASK_CONTROL,
            VKey::LAlt | VKey::RAlt => flags |= CG_EVENT_FLAG_MASK_ALTERNATE,
            VKey::LWin | VKey::RWin => flags |= CG_EVENT_FLAG_MASK_COMMAND,
            _ => {}
        }
    }
    flags
}

fn send_key_event(keycode: u16, key_down: bool, flags: u64) {
    unsafe {
        let source = CGEventSourceCreate(CG_EVENT_SOURCE_STATE_PRIVATE);
        if source.is_null() {
            return;
        }
        let event = CGEventCreateKeyboardEvent(source, keycode, key_down);
        if event.is_null() {
            CFRelease(source);
            return;
        }
        CGEventSetFlags(event, flags);
        CGEventSetIntegerValueField(event, CG_EVENT_SOURCE_USER_DATA, MAGIC_USER_DATA);
        CGEventPost(CG_SESSION_EVENT_TAP, event);
        CFRelease(event);
        CFRelease(source);
    }
}

fn send_key_press(keycode: u16) {
    send_key_event(keycode, true, 0);
    send_key_event(keycode, false, 0);
}

/// [P3 스파이크] 현재 전경 앱에 Cmd+V 주입 — 클립보드 붙여넣기 확장의 핵심 능력.
/// 주입 이벤트는 MAGIC_USER_DATA가 붙어 자체 탭이 재처리하지 않는다.
pub(super) fn inject_paste() {
    send_combo(&[VKey::LWin], VKey::V); // macOS: LWin=Command → Cmd+V
}

fn send_combo(modifier_vkeys: &[VKey], key: VKey) {
    let flags = modifiers_to_flags(modifier_vkeys);
    let kc = vkey_to_cg(key);
    // modifier down → key down → key up → modifier up (Windows 방식과 동일)
    for m in modifier_vkeys {
        send_key_event(vkey_to_cg(*m), true, modifiers_to_flags(&[*m]));
    }
    send_key_event(kc, true, flags);
    send_key_event(kc, false, flags);
    for m in modifier_vkeys.iter().rev() {
        send_key_event(vkey_to_cg(*m), false, 0);
    }
}

/// 코드(chord) 진입 주입 (docs/08 P3): 트리거 down → 키 down 순서로 주입한다.
/// 트리거 up은 ReleaseChord에서 별도 주입 — 그 사이 OS 수정자 상태에 트리거가
/// 유지되므로 이후 통과되는 물리 키들이 트리거 조합(Option+키 등)으로 인식된다.
/// 키의 물리 up은 엔진이 PassThrough — OS가 현재 수정자 상태로 플래그를 채운다.
fn send_chord_engage(trigger: VKey, key: VKey) {
    let tflags = modifiers_to_flags(&[trigger]);
    send_key_event(vkey_to_cg(trigger), true, tflags);
    send_key_event(vkey_to_cg(key), true, tflags);
}

// ── 입력 소스 전환 (한/영 토글) ───────────────────────────────────────────────

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

/// macOS 한/영 입력 소스 토글.
///
/// TIS API(TISSelectInputSource)로 CJK 입력기를 선택하면 선택(메뉴바)만 바뀌고
/// 포커스된 앱의 입력 컨텍스트가 따라오지 않는 경우가 있다. 로그상 선택은
/// 매번 attempt=0에 확인되지만 실제 입력은 영문으로 남았다 — 선택 상태
/// 검증으로는 이 반쪽 전환을 감지할 수 없다.
///
/// CapsLock 같은 OS 네이티브 전환은 항상 완전 동기화되므로, 시스템 단축키
/// "이전 입력 소스 선택"(Ctrl+Space, symbolic hotkey 60)을 합성 이벤트로
/// 주입해 동일한 네이티브 경로(HIToolbox)를 태운다. 입력 소스가 2개(영문/한글)
/// 구성에서 "이전 소스" = 토글. IME의 미확정 조합 커밋도 정상 입력 경로로
/// 흘러 글자 순서 뒤섞임을 방지한다.
///
/// 검증 실패 시 TIS 재선택 폴백은 두지 않는다 — 백그라운드 스레드의 TIS 읽기가
/// 낡은 값을 돌려줘 성공한 전환을 "미반영"으로 오판하는 일이 있고, 그때 도는
/// TISSelectInputSource가 위의 반쪽 전환을 일으켜 잠시 후 영문으로 되돌리는
/// 증상을 만든다 (전환 후 입력이 없을 때만 재현되던 그 버그). 검증은 로그 전용.
///
/// # 직렬화·디바운스 (2026-08-10 키보드 먹통 사고)
///
/// Ctrl+Space는 macOS가 "홀드하면 입력 소스 피커 표시"로도 해석하는 단축키다.
/// 토글이 겹치면(연타 → 스레드 2개) 두 주입의 Ctrl↓/↑가 인터리브되어 OS가
/// "Ctrl 연속 홀드 + Space 2회"로 보고 피커(TextInputMenuAgent)를 띄우며,
/// 이 피커가 key focus를 훔친 채 wedge되면 시스템 전역 키보드가 먹통이 된다
/// (마우스만 생존, 재부팅 전까지 복구 불가 — 실제 발생 사고).
///
/// 방어 2중: ① in-flight 가드 — 이전 주입이 끝나기 전의 재요청은 버린다.
/// ② 디바운스 — 직전 주입 완료 후 `TOGGLE_DEBOUNCE_MS` 내 재요청도 버린다
/// (연타는 "전환이 안 된 것 같아 다시 누름"이므로 한 번만 수행하는 게 의도에
/// 부합하고, OS의 홀드 판정 윈도우와도 겹치지 않게 된다).
fn toggle_input_source() {
    const TOGGLE_DEBOUNCE_MS: u64 = 300;
    /// 이전 토글 스레드가 주입을 마쳤는지 (검증 로깅은 완료로 간주)
    static TOGGLE_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
    /// 마지막 주입 완료 시각 (monotonic ms, 0 = 아직 없음)
    static LAST_INJECT_DONE_MS: AtomicU64 = AtomicU64::new(0);

    fn monotonic_ms() -> u64 {
        static EPOCH: OnceLock<Instant> = OnceLock::new();
        EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64
    }

    if TOGGLE_IN_FLIGHT.swap(true, Ordering::AcqRel) {
        tracing::info!("입력 소스 전환 무시: 이전 토글 주입 진행 중");
        return;
    }
    let last_done = LAST_INJECT_DONE_MS.load(Ordering::Acquire);
    if last_done != 0 && monotonic_ms().saturating_sub(last_done) < TOGGLE_DEBOUNCE_MS {
        TOGGLE_IN_FLIGHT.store(false, Ordering::Release);
        tracing::info!("입력 소스 전환 무시: 디바운스({TOGGLE_DEBOUNCE_MS}ms) 내 재요청");
        return;
    }

    let to_korean = unsafe {
        let current = TISCopyCurrentKeyboardInputSource();
        if current.is_null() {
            tracing::warn!("TISCopyCurrentKeyboardInputSource 실패");
            TOGGLE_IN_FLIGHT.store(false, Ordering::Release);
            return;
        }
        let is_korean = tis_first_language_matches(current, b"ko\0");
        CFRelease(current);
        !is_korean
    };
    let target: &'static [u8] = if to_korean { b"ko\0" } else { b"en\0" };
    let target_name = if to_korean { "ko" } else { "en" };

    std::thread::spawn(move || unsafe {
        // 물리 Shift가 눌린 채로 Ctrl+Space를 주입하면 OS가 Ctrl+Shift+Space
        // (다른 단축키)로 해석해 전환이 안 된다. 예전에는 Shift가 물리적으로
        // 떨어질 때까지 최대 500ms 대기했으나, 그 탓에 "Shift를 떼야만 전환"되고
        // 오래 쥐면 타임아웃 후 실패하는 부작용이 있었다. 이제는 대기 대신 주입
        // 직전 flagsChanged로 Shift 플래그만 지운다 — Space를 누르는 즉시 전환된다.
        // (execute_layer_action의 트리거 modifier 해제 기법과 동일. 합성 이벤트는
        // MAGIC_USER_DATA가 붙어 자체 탭이 재처리하지 않으므로 엔진의
        // modifiers_held는 물리 Shift up에서 정상 정리된다.)
        let ambient = CGEventSourceFlagsState(1);
        if ambient & CG_EVENT_FLAG_MASK_SHIFT != 0 {
            let cleared = ambient & !CG_EVENT_FLAG_MASK_SHIFT;
            let source = CGEventSourceCreate(CG_EVENT_SOURCE_STATE_PRIVATE);
            if !source.is_null() {
                let ev = CGEventCreateKeyboardEvent(source, vkey_to_cg(VKey::LShift), false);
                if !ev.is_null() {
                    CGEventSetType(ev, CG_EVENT_FLAGS_CHANGED);
                    CGEventSetFlags(ev, cleared);
                    CGEventSetIntegerValueField(ev, CG_EVENT_SOURCE_USER_DATA, MAGIC_USER_DATA);
                    CGEventPost(CG_SESSION_EVENT_TAP, ev);
                    CFRelease(ev);
                }
                CFRelease(source);
            }
            std::thread::sleep(std::time::Duration::from_millis(3));
        }

        tracing::info!("입력 소스 전환: Ctrl+Space 주입 → {target_name}");
        send_combo(&[VKey::LCtrl], VKey::Space);

        // 주입(4이벤트)이 끝난 뒤에만 다음 토글을 허용한다 — 검증 로깅은
        // 읽기 전용이라 다음 토글과 겹쳐도 무해하므로 여기서 가드를 푼다.
        LAST_INJECT_DONE_MS.store(monotonic_ms().max(1), Ordering::Release);
        TOGGLE_IN_FLIGHT.store(false, Ordering::Release);

        // 주입 반영 확인 — 로그 전용. 이 스레드의 TIS 읽기는 낡은 값일 수 있어
        // 미확인이어도 개입하지 않는다 (실제 전환은 네이티브 경로가 이미 처리).
        for attempt in 0..4 {
            std::thread::sleep(std::time::Duration::from_millis(60));
            let cur = TISCopyCurrentKeyboardInputSource();
            if cur.is_null() {
                continue;
            }
            let applied = tis_first_language_matches(cur, target);
            CFRelease(cur);
            if applied {
                tracing::info!("입력 소스 전환 확인: {target_name} (attempt={attempt})");
                return;
            }
        }
        tracing::info!("입력 소스 전환 미확인(TIS 읽기 지연 가능): {target_name}");
    });
}

unsafe fn tis_first_language_matches(source: *mut c_void, lang: &[u8]) -> bool {
    let langs = TISGetInputSourceProperty(source, kTISPropertyInputSourceLanguages);
    if langs.is_null() {
        return false;
    }
    if CFArrayGetCount(langs) < 1 {
        return false;
    }
    let first = CFArrayGetValueAtIndex(langs, 0);
    let target =
        CFStringCreateWithCString(std::ptr::null(), lang.as_ptr(), K_CF_STRING_ENCODING_UTF8);
    let eq = CFStringCompare(first, target, 0) == 0;
    CFRelease(target as *mut c_void);
    eq
}

// ── 마우스 주입 ──────────────────────────────────────────────────────────────

/// 현재 눌린 마우스 버튼 비트마스크 (bit0=좌, bit1=우, bit2=중).
/// 이동 이벤트 타입 결정(드래그 vs 이동)과 종료 시 stuck-button 해제에 쓴다.
static BUTTONS_HELD: AtomicU8 = AtomicU8::new(0);

fn mouse_button_bit(mb: MouseBind) -> u8 {
    match mb {
        MouseBind::BtnLeft => 1,
        MouseBind::BtnRight => 2,
        MouseBind::BtnMiddle => 4,
        _ => 0,
    }
}

fn is_mouse_button(mb: MouseBind) -> bool {
    mouse_button_bit(mb) != 0
}

/// 현재 포인터 위치 — 매 틱 fresh 조회.
/// WindowServer가 화면 밖 좌표를 핀 고정하면 다음 조회에서 보정된 값이
/// 돌아오므로 별도 클램프 없이도 자가 보정된다.
fn current_mouse_pos() -> CGPoint {
    unsafe {
        let ev = CGEventCreate(std::ptr::null_mut());
        if ev.is_null() {
            return CGPoint::default();
        }
        let pos = CGEventGetLocation(ev);
        CFRelease(ev);
        pos
    }
}

fn post_mouse_event(event_type: u32, pos: CGPoint, button: u32) {
    unsafe {
        let ev = CGEventCreateMouseEvent(std::ptr::null_mut(), event_type, pos, button);
        if ev.is_null() {
            return;
        }
        CGEventSetIntegerValueField(ev, CG_EVENT_SOURCE_USER_DATA, MAGIC_USER_DATA);
        CGEventPost(CG_SESSION_EVENT_TAP, ev);
        CFRelease(ev);
    }
}

fn send_mouse_button(mb: MouseBind, down: bool) {
    let (dt, ut, button) = match mb {
        MouseBind::BtnLeft => (
            CG_EVENT_LEFT_MOUSE_DOWN,
            CG_EVENT_LEFT_MOUSE_UP,
            CG_MOUSE_BUTTON_LEFT,
        ),
        MouseBind::BtnRight => (
            CG_EVENT_RIGHT_MOUSE_DOWN,
            CG_EVENT_RIGHT_MOUSE_UP,
            CG_MOUSE_BUTTON_RIGHT,
        ),
        MouseBind::BtnMiddle => (
            CG_EVENT_OTHER_MOUSE_DOWN,
            CG_EVENT_OTHER_MOUSE_UP,
            CG_MOUSE_BUTTON_CENTER,
        ),
        _ => return,
    };
    let bit = mouse_button_bit(mb);
    if down {
        BUTTONS_HELD.fetch_or(bit, Ordering::Relaxed);
    } else {
        BUTTONS_HELD.fetch_and(!bit, Ordering::Relaxed);
    }
    post_mouse_event(if down { dt } else { ut }, current_mouse_pos(), button);
}

/// 상대 이동 — 버튼 홀드 중이면 드래그 이벤트로 보낸다
/// (앱들은 mouseMoved가 아닌 *MouseDragged만 드래그로 인식)
fn send_mouse_move_rel(dx: i32, dy: i32) {
    let cur = current_mouse_pos();
    let pos = CGPoint {
        x: cur.x + dx as f64,
        y: cur.y + dy as f64,
    };
    let held = BUTTONS_HELD.load(Ordering::Relaxed);
    let (event_type, button) = if held & 1 != 0 {
        (CG_EVENT_LEFT_MOUSE_DRAGGED, CG_MOUSE_BUTTON_LEFT)
    } else if held & 2 != 0 {
        (CG_EVENT_RIGHT_MOUSE_DRAGGED, CG_MOUSE_BUTTON_RIGHT)
    } else if held & 4 != 0 {
        (CG_EVENT_OTHER_MOUSE_DRAGGED, CG_MOUSE_BUTTON_CENTER)
    } else {
        (CG_EVENT_MOUSE_MOVED, CG_MOUSE_BUTTON_LEFT)
    };
    post_mouse_event(event_type, pos, button);
}

fn send_mouse_wheel(notches: i32) {
    unsafe {
        let ev = CGEventCreateScrollWheelEvent(
            std::ptr::null_mut(),
            CG_SCROLL_EVENT_UNIT_LINE,
            1,
            notches,
        );
        if ev.is_null() {
            return;
        }
        CGEventSetIntegerValueField(ev, CG_EVENT_SOURCE_USER_DATA, MAGIC_USER_DATA);
        CGEventPost(CG_SESSION_EVENT_TAP, ev);
        CFRelease(ev);
    }
}

/// CGEvent 기반 [`MouseSink`] — 모션 워커가 틱마다 호출
struct MacMouseSink;

impl MouseSink for MacMouseSink {
    fn move_rel(&mut self, dx: i32, dy: i32) {
        send_mouse_move_rel(dx, dy);
    }
    fn wheel(&mut self, notches: i32) {
        send_mouse_wheel(notches);
    }
}

// ── 액션 실행 ────────────────────────────────────────────────────────────────

fn execute_action(action: &BindAction) {
    match action {
        BindAction::SendKey(key) => {
            if matches!(key, VKey::Hangul | VKey::Hanja) {
                toggle_input_source();
            } else {
                send_key_press(vkey_to_cg(*key));
            }
        }
        BindAction::SendCombo { modifiers, key } => {
            send_combo(modifiers, *key);
        }
        BindAction::Macro(steps) => {
            for step in steps {
                match step {
                    MacroStep::KeyPress(k) => send_key_event(vkey_to_cg(*k), true, 0),
                    MacroStep::KeyRelease(k) => send_key_event(vkey_to_cg(*k), false, 0),
                    MacroStep::Combo { modifiers, key } => {
                        send_combo(modifiers, *key);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        BindAction::Launch(cmd) => {
            // 런처가 포커스를 뺏기 전에 현재 전경 앱을 기억한다 (docs/12 흐름 B —
            // 나중에 클립보드 항목을 그 앱에 붙여넣기 위함). 다른 launch 바인딩이
            // 런처 사용 중 붙여넣기 대상을 덮어쓰지 않도록 데스크톱 런처를 여는
            // 액션에서만 캡처한다.
            if crate::clipboard::launch_captures_foreground(cmd) {
                crate::clipboard::capture_foreground_app();
            }
            let resolved = resolve_launch_cmd(cmd);
            tracing::info!("프로그램 실행: {resolved}");
            std::thread::spawn(move || {
                // macOS에서 Alt/Space 단축키 직후 즉시 실행하면 modifier 해제/IME 초기 조합
                // 타이밍과 겹쳐 첫 한글 조합이 깨질 수 있다. 짧은 지연 후 실행한다.
                std::thread::sleep(std::time::Duration::from_millis(45));
                if let Err(e) = std::process::Command::new(&resolved).spawn() {
                    tracing::error!("프로그램 실행 실패: {resolved} — {e}");
                }
            });
        }
        // 상태형 마우스 바인딩이 콤보/리맵/더블탭 등 일회성 경로로 온 경우 —
        // 단발 동작으로 처리 (클릭 1회, 휠 1노치, 짧은 이동)
        BindAction::Mouse(mb) => match mb {
            MouseBind::BtnLeft | MouseBind::BtnRight | MouseBind::BtnMiddle => {
                send_mouse_button(*mb, true);
                send_mouse_button(*mb, false);
            }
            MouseBind::WheelUp => send_mouse_wheel(1),
            MouseBind::WheelDown => send_mouse_wheel(-1),
            MouseBind::MoveUp => send_mouse_move_rel(0, -25),
            MouseBind::MoveDown => send_mouse_move_rel(0, 25),
            MouseBind::MoveLeft => send_mouse_move_rel(-25, 0),
            MouseBind::MoveRight => send_mouse_move_rel(25, 0),
            MouseBind::Slow => {}
        },
        BindAction::ClipPaste(n) => crate::clipboard::paste_slot(*n),
    }
}

/// 레이어 내 액션 실행 — 트리거 modifier(Alt 등)를 일시 해제한 뒤 액션을 보낸다.
/// 물리적으로 Alt를 누르고 있으면 합성 이벤트(Cmd+Left 등)에 잔여 Alt 플래그가
/// 간섭할 수 있다. 특히 한글 IME 활성 시 이 간섭이 문제가 된다.
///
/// 단순 keyUp이 아닌 flagsChanged(타입 12) 이벤트를 직접 생성하여
/// OS modifier 상태를 확실히 클리어한다.
fn execute_layer_action(action: &BindAction, trigger: VKey) {
    // Launch 액션은 합성 키 조합을 보내지 않으므로, trigger modifier(Alt) 강제 해제가
    // 오히려 런처 최초 IME 조합 컨텍스트를 깨뜨릴 수 있다.
    if matches!(action, BindAction::Launch(_)) {
        execute_action(action);
        return;
    }

    // 트리거(Alt) 플래그만 지우고 함께 눌린 다른 물리 수정자(Shift 등)는
    // 보존한다 — flags=0으로 전부 지우면 Shift 홀드 중 상태가 끊긴다.
    const ALL_MOD_MASK: u64 = CG_EVENT_FLAG_MASK_SHIFT
        | CG_EVENT_FLAG_MASK_CONTROL
        | CG_EVENT_FLAG_MASK_ALTERNATE
        | CG_EVENT_FLAG_MASK_COMMAND;
    let trigger_mask = modifiers_to_flags(&[trigger]);
    let kept_flags = unsafe { CGEventSourceFlagsState(1) } & ALL_MOD_MASK & !trigger_mask;

    unsafe {
        let source = CGEventSourceCreate(CG_EVENT_SOURCE_STATE_PRIVATE);
        if !source.is_null() {
            let trigger_kc = vkey_to_cg(trigger);
            let ev = CGEventCreateKeyboardEvent(source, trigger_kc, false);
            if !ev.is_null() {
                CGEventSetType(ev, CG_EVENT_FLAGS_CHANGED);
                CGEventSetFlags(ev, kept_flags);
                CGEventSetIntegerValueField(ev, CG_EVENT_SOURCE_USER_DATA, MAGIC_USER_DATA);
                CGEventPost(CG_SESSION_EVENT_TAP, ev);
                CFRelease(ev);
            }
            CFRelease(source);
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(3));

    // 물리 Shift가 눌려 있으면 레이어 SendKey에 합성한다 —
    // Shift+Alt+H = Shift+Left (선택 확장). Windows는 물리 Shift가 그대로
    // 통과해 자연 조합되므로 macOS만 명시 병합이 필요하다.
    let shift_flag = kept_flags & CG_EVENT_FLAG_MASK_SHIFT;
    if shift_flag != 0 {
        if let BindAction::SendKey(key) = action {
            if !matches!(key, VKey::Hangul | VKey::Hanja) {
                let kc = vkey_to_cg(*key);
                send_key_event(kc, true, shift_flag);
                send_key_event(kc, false, shift_flag);
                return;
            }
        }
    }

    execute_action(action);
}

// ── [실험] CapsLock → F19 HID 재맵 (docs/13) ─────────────────────────────────
//
// macOS는 CapsLock을 LED-토글 flagsChanged로 전달해 홀드 감지가 불가능하다.
// hidutil로 CapsLock(0x39)을 F19(0x6E)로 HID 레벨 재맵하면 깨끗한 down/up
// 일반 키가 되어, 지연·토글 없이 홀드 트리거로 쓸 수 있다(Karabiner 방식).

/// 우리가 CapsLock 재맵을 적용했는지 — stop()에서 원복 여부 판단용.
static CAPSLOCK_REMAPPED: AtomicBool = AtomicBool::new(false);

/// CapsLock→F19 재맵이 적용됐는지 — 셀프테스트가 엔진의 실제 트리거
/// 키코드(F19)를 알아내는 데 쓴다.
pub(crate) fn capslock_remapped() -> bool {
    CAPSLOCK_REMAPPED.load(Ordering::Relaxed)
}

const HID_CAPSLOCK: u64 = 0x7_0000_0039;
const HID_F19: u64 = 0x7_0000_006E;

/// config의 레이어 트리거(및 별칭)에 CapsLock이 있으면, hidutil로 CapsLock→F19
/// 재맵을 적용하고 그 트리거들을 F19로 바꿔 반환한다. CapsLock 트리거가 없으면
/// 그대로 반환(재맵도 안 함).
fn remap_capslock_trigger_to_f19(mut config: KeybindConfig) -> KeybindConfig {
    let uses_capslock = config
        .layers
        .iter()
        .any(|l| l.trigger == VKey::CapsLock || l.trigger_aliases.contains(&VKey::CapsLock));
    if !uses_capslock {
        return config;
    }

    if apply_hidutil_remap(HID_CAPSLOCK, HID_F19) {
        CAPSLOCK_REMAPPED.store(true, Ordering::Relaxed);
        tracing::info!("CapsLock→F19 HID 재맵 적용 (CapsLock 트리거 실험)");
        for layer in &mut config.layers {
            if layer.trigger == VKey::CapsLock {
                layer.trigger = VKey::F19;
            }
            for a in &mut layer.trigger_aliases {
                if *a == VKey::CapsLock {
                    *a = VKey::F19;
                }
            }
        }
    } else {
        tracing::warn!("CapsLock→F19 재맵 실패 — CapsLock 홀드 트리거가 불안정할 수 있음");
    }
    config
}

/// hidutil로 단일 키 매핑 적용. 성공 시 true.
fn apply_hidutil_remap(src: u64, dst: u64) -> bool {
    let mapping = format!(
        r#"{{"UserKeyMapping":[{{"HIDKeyboardModifierMappingSrc":{src},"HIDKeyboardModifierMappingDst":{dst}}}]}}"#
    );
    std::process::Command::new("hidutil")
        .args(["property", "--set", &mapping])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// hidutil 매핑 제거 (원복). stop()에서 우리가 적용했을 때만 호출.
fn clear_hidutil_remap() {
    if !CAPSLOCK_REMAPPED.swap(false, Ordering::Relaxed) {
        return;
    }
    let _ = std::process::Command::new("hidutil")
        .args(["property", "--set", r#"{"UserKeyMapping":[]}"#])
        .status();
    tracing::info!("CapsLock→F19 HID 재맵 원복");
}

// ── 글로벌 상태 ──────────────────────────────────────────────────────────────
//
// 바인딩 판정 로직은 전부 keybind::engine(플랫폼 독립, 단위 테스트 가능)에
// 있다. 이 파일은 CGEventTap 설치/이벤트 변환/액션 실행만 담당한다.

static HOOK_STATE: OnceLock<Arc<Mutex<EngineState>>> = OnceLock::new();

/// CGEventTap 포인터 — 타임아웃 후 재활성화에 사용
/// 탭 타임아웃 서킷 브레이커 — 이 시간(ms) 안에 MAX_RETRIES를 넘으면 재활성화 포기
const TAP_TIMEOUT_WINDOW_MS: u32 = 10_000;
const TAP_TIMEOUT_MAX_RETRIES: u32 = 5;
static TAP_TIMEOUT_WINDOW_START: AtomicU32 = AtomicU32::new(0);
static TAP_TIMEOUT_COUNT: AtomicU32 = AtomicU32::new(0);

static EVENT_TAP_PTR: std::sync::atomic::AtomicPtr<c_void> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// 엔진에 넘길 밀리초 단조 tick (u32 wrapping — 엔진의 wrapping_sub 규약)
fn tick_ms() -> u32 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u32
}

/// 엔진 결정 실행 — 레이어 컨텍스트가 있으면 트리거 수정자 해제 경로 사용.
/// **워커 스레드에서만 호출** — 매크로 스텝 sleep 등이 포함되므로
/// 탭 콜백에서 직접 부르면 kCGEventTapDisabledByTimeout 위험이 있다.
fn run_action(action: &BindAction, layer_trigger: Option<VKey>) {
    match layer_trigger {
        Some(trigger) => execute_layer_action(action, trigger),
        None => execute_action(action),
    }
}

// ── 액션 워커 ────────────────────────────────────────────────────────────────
//
// 탭 콜백은 큐잉만 하고 즉시 반환한다 (Windows 어댑터와 동일 구조).
// 콜백이 느리면 macOS가 탭을 kCGEventTapDisabledByTimeout으로 꺼버리고,
// 재활성화까지 키 이벤트가 가로채지지 않은 채 통과한다. 매크로 스텝 sleep
// (5ms/스텝), 레이어 액션의 수정자 해제 지연(3ms) 등은 전부 워커에서 수행.

/// 워커 스레드 작업 — 일회성 액션 또는 코드(chord) 모드 주입/해제
enum WorkerJob {
    Action {
        action: BindAction,
        layer_trigger: Option<VKey>,
    },
    /// 코드 진입: 트리거 down → 키 down 주입 (탭 재진입은 MAGIC_USER_DATA로 차단)
    ChordEngage { trigger: VKey, key: VKey },
    /// 코드 해제: 트리거 up 주입
    ChordRelease { trigger: VKey },
    /// 마우스 바인딩 시작 — 이동/휠은 모션 워커, 버튼은 즉시 down 주입
    MouseEngage(MouseBind),
    /// 마우스 바인딩 정지 — 이동/휠 해제, 버튼 up 주입
    MouseRelease(MouseBind),
    /// 활성 마우스 바인딩 전체 정지 (stuck-mouse 방지)
    MouseStopAll,
}

/// 액션 워커로 보내는 채널. FIFO이므로 키 입력 순서가 보존된다.
static ACTION_TX: Mutex<Option<mpsc::Sender<WorkerJob>>> = Mutex::new(None);

/// 작업을 워커 스레드 큐에 넣는다 (탭 콜백에서 호출 — 블로킹 없음)
fn queue_job(job: WorkerJob) {
    let sender = ACTION_TX.lock().ok().and_then(|g| g.clone());
    match sender {
        Some(tx) => {
            if tx.send(job).is_err() {
                tracing::warn!("액션 워커가 종료되어 작업을 실행하지 못했습니다");
            }
        }
        None => tracing::warn!("액션 워커 미시작 — 작업 무시"),
    }
}

// ── CGEventTap 콜백 ─────────────────────────────────────────────────────────

unsafe extern "C" fn event_tap_callback(
    _proxy: CGEventTapProxy,
    type_: u32,
    event: CGEventRef,
    _user_info: *mut c_void,
) -> CGEventRef {
    // `extern "C"` 경계를 넘는 패닉은 정의되지 않은 동작이고, 현대 Rust는 이를
    // 감지하면 프로세스를 abort시킨다 — 이 콜백이 유일한 키 이벤트 처리 지점이라
    // 그 순간 키보드 전체가 잠긴다. 어떤 패닉(뮤텍스 poison·인덱스 초과 등)도
    // 여기서 격리하고, 이벤트를 그대로 통과시켜(remap 없이) OS 기본 동작은 살린다.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        event_tap_callback_inner(type_, event)
    }))
    .unwrap_or_else(|_| {
        tracing::error!("event_tap_callback 내부 패닉 — 이벤트 통과 (훅 유지)");
        event
    })
}

unsafe fn event_tap_callback_inner(type_: u32, event: CGEventRef) -> CGEventRef {
    let tap_disabled =
        type_ == CG_EVENT_TAP_DISABLED_BY_TIMEOUT || type_ == CG_EVENT_TAP_DISABLED_BY_USER_INPUT;

    // null 이벤트 방어 (OS 오류 시)
    if event.is_null() && !tap_disabled {
        return event;
    }

    // 타임아웃/입력 폭주로 비활성화된 경우 재활성화 + 일시 상태 리셋
    if tap_disabled {
        // ── 서킷 브레이커 ──────────────────────────────────────────────
        // 이 탭은 활성 탭이라 모든 키 입력이 이 콜백을 동기로 거친다. 데몬이
        // CPU 기아 상태(대형 빌드 등)면 콜백이 밀려 OS가 탭을 끄는데, 그때마다
        // 무조건 되살리면 "굶주린 탭"이 계속 입력 경로를 붙들어 시스템 전체
        // 타이핑이 멈춘 것처럼 된다 (2026-08-08 실제 사고 — 사용자가 리부팅).
        // 짧은 시간에 타임아웃이 반복되면 기아 상태로 판단하고 재활성화를
        // 포기한다: 탭이 꺼진 채면 키 입력은 OS 네이티브로 즉시 정상화되고,
        // 리맵만 꺼진다. 복구는 데몬 재시작 (status가 사유를 보여준다).
        let now = tick_ms();
        let start = TAP_TIMEOUT_WINDOW_START.load(Ordering::Relaxed);
        let count = if now.saturating_sub(start) > TAP_TIMEOUT_WINDOW_MS {
            TAP_TIMEOUT_WINDOW_START.store(now, Ordering::Relaxed);
            TAP_TIMEOUT_COUNT.store(1, Ordering::Relaxed);
            1
        } else {
            TAP_TIMEOUT_COUNT.fetch_add(1, Ordering::Relaxed) + 1
        };

        // 어느 경로든 일시 상태는 리셋 (stuck modifier 방지)
        if let Some(state) = HOOK_STATE.get() {
            if let Ok(mut guard) = state.lock() {
                guard.reset_transient_state();
            }
        }

        if count > TAP_TIMEOUT_MAX_RETRIES {
            super::set_hook_error(Some(format!(
                "키 이벤트 처리 지연으로 훅을 중단했습니다 ({TAP_TIMEOUT_WINDOW_MS}ms 내 \
                 타임아웃 {count}회 — CPU 과부하 의심). 키 입력은 OS 기본 동작으로 \
                 정상이며 리맵만 꺼진 상태입니다. 복구: kmd daemon stop 후 start"
            )));
            tracing::error!(
                "CGEventTap 타임아웃 {count}회/{TAP_TIMEOUT_WINDOW_MS}ms — 재활성화 중단 (기아 상태)"
            );
            return event;
        }

        let tap = EVENT_TAP_PTR.load(Ordering::Relaxed);
        if !tap.is_null() {
            let cause = if type_ == CG_EVENT_TAP_DISABLED_BY_TIMEOUT {
                "타임아웃"
            } else {
                "입력 폭주"
            };
            tracing::warn!("CGEventTap 비활성화 감지({cause}) — 재활성화 및 상태 리셋");
            CGEventTapEnable(tap, true);
        }
        return event;
    }

    // keyDown, keyUp, flagsChanged 만 처리
    if type_ != CG_EVENT_KEY_DOWN && type_ != CG_EVENT_KEY_UP && type_ != CG_EVENT_FLAGS_CHANGED {
        return event;
    }

    // 자체 생성 이벤트 패스스루
    if CGEventGetIntegerValueField(event, CG_EVENT_SOURCE_USER_DATA) == MAGIC_USER_DATA {
        return event;
    }

    let keycode = CGEventGetIntegerValueField(event, CG_KEYBOARD_EVENT_KEYCODE) as u16;
    let Some(vkey) = cg_to_vkey(keycode) else {
        return event;
    };

    let state = match HOOK_STATE.get() {
        Some(s) => s,
        None => {
            // keycode는 실제 타이핑 내용이므로 로그에 남기지 않는다
            tracing::warn!("HOOK_STATE 미초기화 — 이벤트 패스스루");
            return event;
        }
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return event,
    };

    let tick = tick_ms();

    // ── flagsChanged 이벤트: 수정자 키를 down/up으로 변환해 엔진에 위임 ──
    if type_ == CG_EVENT_FLAGS_CHANGED {
        // 실제 OS 플래그에서 modifier 상태 판단 (토글 방식 대신)
        let flags = CGEventGetFlags(event);
        let is_down = match vkey {
            VKey::LShift | VKey::RShift => (flags & CG_EVENT_FLAG_MASK_SHIFT) != 0,
            VKey::LCtrl | VKey::RCtrl => (flags & CG_EVENT_FLAG_MASK_CONTROL) != 0,
            VKey::LAlt | VKey::RAlt => (flags & CG_EVENT_FLAG_MASK_ALTERNATE) != 0,
            VKey::LWin | VKey::RWin => (flags & CG_EVENT_FLAG_MASK_COMMAND) != 0,
            VKey::CapsLock => (flags & 0x0001_0000) != 0,
            _ => !guard.is_modifier_held(vkey),
        };

        let decision = guard.process_key(vkey, is_down, tick);

        // 놓친 keyup으로 인한 stuck modifier 방지 — OS 플래그와 동기화
        guard.sync_modifier_flags(
            (flags & CG_EVENT_FLAG_MASK_SHIFT) != 0,
            (flags & CG_EVENT_FLAG_MASK_CONTROL) != 0,
            (flags & CG_EVENT_FLAG_MASK_ALTERNATE) != 0,
            (flags & CG_EVENT_FLAG_MASK_COMMAND) != 0,
        );

        return match decision {
            KeyDecision::PassThrough => {
                drop(guard);
                event
            }
            KeyDecision::Suppress => std::ptr::null_mut(),
            KeyDecision::Execute {
                action,
                layer_trigger,
            } => {
                drop(guard);
                queue_job(WorkerJob::Action {
                    action,
                    layer_trigger,
                });
                std::ptr::null_mut()
            }
            // 코드 진입 (flagsChanged 경로로는 실제 발생하지 않지만 대칭 구현)
            KeyDecision::EngageChord { trigger, key } => {
                drop(guard);
                // 어떤 키인지는 로그하지 않는다 — 실제 타이핑 내용 비기록 정책
                tracing::debug!("chord engage: {trigger:?}+미매핑 키");
                queue_job(WorkerJob::ChordEngage { trigger, key });
                std::ptr::null_mut()
            }
            // 코드 해제: 물리 트리거 up(flagsChanged)을 억제하고 주입 up으로 대체.
            // 지연 Launch는 해제 뒤에 실행 (FIFO 큐가 순서 보장)
            KeyDecision::ReleaseChord {
                trigger,
                deferred_action,
            } => {
                drop(guard);
                tracing::debug!("chord release: {trigger:?}");
                queue_job(WorkerJob::ChordRelease { trigger });
                if let Some(action) = deferred_action {
                    queue_job(WorkerJob::Action {
                        action,
                        layer_trigger: None,
                    });
                }
                std::ptr::null_mut()
            }
            // 마우스 바인딩 (flagsChanged 경로 — LShift 저속 모드 등 수정자 매핑)
            KeyDecision::MouseEngage(mb) => {
                drop(guard);
                queue_job(WorkerJob::MouseEngage(mb));
                std::ptr::null_mut()
            }
            KeyDecision::MouseRelease(mb) => {
                drop(guard);
                queue_job(WorkerJob::MouseRelease(mb));
                std::ptr::null_mut()
            }
            KeyDecision::MouseStopAll => {
                drop(guard);
                queue_job(WorkerJob::MouseStopAll);
                std::ptr::null_mut()
            }
        };
    }

    // ── keyDown / keyUp 이벤트 ──
    let is_down = type_ == CG_EVENT_KEY_DOWN;

    match guard.process_key(vkey, is_down, tick) {
        KeyDecision::PassThrough => {
            drop(guard);
            event
        }
        KeyDecision::Suppress => std::ptr::null_mut(),
        KeyDecision::Execute {
            action,
            layer_trigger,
        } => {
            drop(guard);
            queue_job(WorkerJob::Action {
                action,
                layer_trigger,
            });
            std::ptr::null_mut()
        }
        // 코드 진입: 물리 키 down을 억제하고 트리거 down + 키 down을 주입.
        // 이후 이 홀드의 물리 키들은 PassThrough — OS 수정자 상태에 주입된
        // 트리거가 있으므로 트리거 조합으로 인식된다.
        KeyDecision::EngageChord { trigger, key } => {
            drop(guard);
            // 어떤 키인지는 로그하지 않는다 — 실제 타이핑 내용 비기록 정책
            tracing::debug!("chord engage: {trigger:?}+미매핑 키");
            queue_job(WorkerJob::ChordEngage { trigger, key });
            std::ptr::null_mut()
        }
        // 코드 해제 (keymap 토글이 코드 모드를 끊는 경우 이 경로로도 온다)
        KeyDecision::ReleaseChord {
            trigger,
            deferred_action,
        } => {
            drop(guard);
            tracing::debug!("chord release: {trigger:?}");
            queue_job(WorkerJob::ChordRelease { trigger });
            if let Some(action) = deferred_action {
                queue_job(WorkerJob::Action {
                    action,
                    layer_trigger: None,
                });
            }
            std::ptr::null_mut()
        }
        // 마우스 바인딩 — 물리 키 이벤트를 억제하고 워커에 위임
        KeyDecision::MouseEngage(mb) => {
            drop(guard);
            queue_job(WorkerJob::MouseEngage(mb));
            std::ptr::null_mut()
        }
        KeyDecision::MouseRelease(mb) => {
            drop(guard);
            queue_job(WorkerJob::MouseRelease(mb));
            std::ptr::null_mut()
        }
        KeyDecision::MouseStopAll => {
            drop(guard);
            queue_job(WorkerJob::MouseStopAll);
            std::ptr::null_mut()
        }
    }
}

// ── Backend 구현 ─────────────────────────────────────────────────────────────

/// CFRunLoopRef 등 raw pointer를 스레드 간 안전하게 전달하기 위한 래퍼.
/// CFRunLoopStop은 다른 스레드에서 호출해도 안전하다.
struct SendPtr(*mut c_void);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

pub struct MacOSKeyboardBackend {
    running: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    action_worker: Option<std::thread::JoinHandle<()>>,
    run_loop: Arc<Mutex<Option<SendPtr>>>,
}

impl MacOSKeyboardBackend {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            thread: None,
            action_worker: None,
            run_loop: Arc::new(Mutex::new(None)),
        }
    }
}

/// 접근성(손쉬운 사용) 권한 확인. 권한이 없으면 시스템 다이얼로그를 띄운다.
fn check_accessibility_permission() -> bool {
    unsafe {
        // kAXTrustedCheckOptionPrompt = "AXTrustedCheckOptionPrompt"
        let key_str = CFStringCreateWithCString(
            std::ptr::null(),
            c"AXTrustedCheckOptionPrompt".as_ptr().cast(),
            0x0800_0100, // kCFStringEncodingUTF8
        );
        if key_str.is_null() {
            return AXIsProcessTrustedWithOptions(std::ptr::null());
        }
        let keys = [key_str];
        let values = [kCFBooleanTrue];
        let options = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks as *const u8 as *const c_void,
            &kCFTypeDictionaryValueCallBacks as *const u8 as *const c_void,
        );
        let trusted = AXIsProcessTrustedWithOptions(options);
        if !options.is_null() {
            CFRelease(options);
        }
        CFRelease(key_str as *mut c_void);
        trusted
    }
}

impl KeyboardBackend for MacOSKeyboardBackend {
    fn start(&mut self, config: KeybindConfig) -> Result<(), String> {
        if self.running.load(Ordering::Relaxed) {
            return Err("키 바인딩이 이미 실행 중입니다.".into());
        }

        if !check_accessibility_permission() {
            tracing::warn!(
                "접근성 권한이 없습니다. 시스템 설정 > 개인 정보 보호 및 보안 > \
                 손쉬운 사용에서 이 앱을 허용해주세요. (권한 요청 다이얼로그가 표시됩니다)"
            );
        }

        // [실험] CapsLock 트리거: macOS는 CapsLock을 LED-토글 flagsChanged로 줘서
        // 홀드 감지가 불가능하다(macos.rs 1027 참조). hidutil로 CapsLock→F19로
        // HID 레벨 재맵해 "깨끗한 일반 키"로 만든 뒤, 트리거를 F19로 바꿔 쓴다.
        // 재맵은 재부팅 시 초기화되고 stop()에서도 원복한다 (docs/13).
        let config = remap_capslock_trigger_to_f19(config);

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

        // 액션 워커 시작 — 탭 콜백은 큐잉만 하고 실행은 이 스레드가 담당
        let (action_tx, action_rx) = mpsc::channel::<WorkerJob>();
        match ACTION_TX.lock() {
            Ok(mut g) => *g = Some(action_tx),
            Err(_) => return Err("액션 큐 잠금 실패".into()),
        }
        self.action_worker = Some(std::thread::spawn(move || {
            // 모션 워커(이동/휠 틱 스레드)와 버튼 상태는 액션 워커가 소유한다
            let motion = MouseWorker::start(MacMouseSink);
            let mut pressed_buttons: Vec<MouseBind> = Vec::new();

            for job in action_rx {
                match job {
                    WorkerJob::Action {
                        action,
                        layer_trigger,
                    } => run_action(&action, layer_trigger),
                    WorkerJob::ChordEngage { trigger, key } => send_chord_engage(trigger, key),
                    WorkerJob::ChordRelease { trigger } => {
                        send_key_event(vkey_to_cg(trigger), false, 0)
                    }
                    WorkerJob::MouseEngage(mb) => {
                        if is_mouse_button(mb) {
                            if !pressed_buttons.contains(&mb) {
                                pressed_buttons.push(mb);
                                send_mouse_button(mb, true);
                            }
                        } else {
                            motion.engage(mb);
                        }
                    }
                    WorkerJob::MouseRelease(mb) => {
                        if is_mouse_button(mb) {
                            if let Some(pos) = pressed_buttons.iter().position(|&b| b == mb) {
                                pressed_buttons.remove(pos);
                                send_mouse_button(mb, false);
                            }
                        } else {
                            motion.release(mb);
                        }
                    }
                    WorkerJob::MouseStopAll => {
                        motion.stop_all();
                        for mb in pressed_buttons.drain(..) {
                            send_mouse_button(mb, false);
                        }
                    }
                }
            }
            // 종료 정리 — 눌린 버튼 해제 (모션은 MouseWorker Drop이 정지)
            for mb in pressed_buttons.drain(..) {
                send_mouse_button(mb, false);
            }
            tracing::debug!("액션 워커 종료");
        }));

        let running = self.running.clone();
        running.store(true, Ordering::Relaxed);
        let rl_store = self.run_loop.clone();

        let thread = std::thread::spawn(move || {
            unsafe {
                let trusted = check_accessibility_permission();
                tracing::info!("AXIsProcessTrusted = {trusted}");

                let events_of_interest: u64 = (1u64 << CG_EVENT_KEY_DOWN)
                    | (1u64 << CG_EVENT_KEY_UP)
                    | (1u64 << CG_EVENT_FLAGS_CHANGED);

                // kCGHIDEventTap(0) — Alt(Option) 키를 시스템 처리 전에 억제해야
                // 특수문자(˙∆˚¬) 생성을 방지할 수 있음
                let mut tap = CGEventTapCreate(
                    0, // kCGHIDEventTap
                    CG_HEAD_INSERT_EVENT_TAP,
                    CG_EVENT_TAP_OPTION_DEFAULT,
                    events_of_interest,
                    event_tap_callback,
                    std::ptr::null_mut(),
                );

                if tap.is_null() {
                    tracing::warn!("kCGHIDEventTap 실패, kCGSessionEventTap 재시도...");
                    tap = CGEventTapCreate(
                        CG_SESSION_EVENT_TAP,
                        CG_HEAD_INSERT_EVENT_TAP,
                        CG_EVENT_TAP_OPTION_DEFAULT,
                        events_of_interest,
                        event_tap_callback,
                        std::ptr::null_mut(),
                    );
                }

                if tap.is_null() {
                    // 여기서 죽어도 데몬 본체는 계속 산다 — status가 "실행 중"으로
                    // 보이는 함정을 막으려면 실패 사유를 반드시 남겨야 한다.
                    let exe = std::env::current_exe()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|_| "kmd-daemon".into());
                    super::set_hook_error(Some(format!(
                        "키보드 훅 설치 실패 (AXIsProcessTrusted={trusted}) — \
                         시스템 설정 > 개인 정보 보호 및 보안 > 손쉬운 사용에서 \
                         {exe} 을(를) 다시 허용하세요. \
                         재빌드하면 서명이 바뀌어 기존 허용이 무효화됩니다."
                    )));
                    tracing::error!(
                        "CGEventTapCreate 실패 — 접근성(손쉬운 사용) 권한을 확인하세요. \
                         (AXIsProcessTrusted={trusted}, pid={}, exe={exe})",
                        std::process::id()
                    );
                    running.store(false, Ordering::Relaxed);
                    return;
                }

                // 타임아웃 후 재활성화를 위해 tap 포인터 저장
                EVENT_TAP_PTR.store(tap, Ordering::Relaxed);

                let source = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
                if source.is_null() {
                    super::set_hook_error(Some(
                        "키보드 훅 설치 실패 — CFMachPortCreateRunLoopSource 실패".into(),
                    ));
                    tracing::error!("CFMachPortCreateRunLoopSource 실패");
                    CFRelease(tap);
                    running.store(false, Ordering::Relaxed);
                    return;
                }

                let rl = CFRunLoopGetCurrent();
                CFRunLoopAddSource(rl, source, kCFRunLoopCommonModes);
                CGEventTapEnable(tap, true);

                if let Ok(mut store) = rl_store.lock() {
                    *store = Some(SendPtr(rl));
                }

                super::set_hook_error(None);
                tracing::info!("CGEventTap 키보드 훅 설치 완료");

                CFRunLoopRun();

                CGEventTapEnable(tap, false);
                CFRelease(source);
                CFRelease(tap);

                tracing::info!("CGEventTap 키보드 훅 해제 완료");
            }
        });

        self.thread = Some(thread);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), String> {
        // CapsLock→F19 재맵을 적용했다면 원복 (다른 앱들이 CapsLock을 정상 사용)
        clear_hidutil_remap();

        // stuck modifier 방지: 종료 전에 held 상태인 modifier key-up 전송
        if let Some(state) = HOOK_STATE.get() {
            if let Ok(mut guard) = state.lock() {
                for vkey in guard.held_modifiers() {
                    send_key_event(vkey_to_cg(vkey), false, 0);
                }
                guard.reset_transient_state();
            }
        }

        self.running.store(false, Ordering::Relaxed);
        EVENT_TAP_PTR.store(std::ptr::null_mut(), Ordering::Relaxed);
        if let Ok(store) = self.run_loop.lock() {
            if let Some(ref rl) = *store {
                unsafe {
                    CFRunLoopStop(rl.0);
                }
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // 센더를 드롭하면 워커가 남은 큐를 소진한 뒤 recv 루프를 끝낸다
        if let Ok(mut g) = ACTION_TX.lock() {
            *g = None;
        }
        if let Some(worker) = self.action_worker.take() {
            let _ = worker.join();
        }
        Ok(())
    }
}
