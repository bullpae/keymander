//! 키 바인딩 엔진 — OS-native 키보드 훅 기반 키 리매핑
//!
//! kanata 외부 의존 없이 직접 키 바인딩을 처리한다.
//! Windows: SetWindowsHookEx(WH_KEYBOARD_LL)
//! macOS: CGEventTap (향후)
//! Linux: evdev + uinput (향후)

pub mod engine;
pub mod mouse;

#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "macos")]
pub mod selftest;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

// ── 훅 설치 상태 보고 ────────────────────────────────────────────────────────
//
// OS 훅 설치는 백그라운드 스레드에서 일어나므로 start()가 돌려주는 Ok(())는
// "훅이 살아 있다"는 뜻이 아니다. macOS에서 접근성 권한이 없으면 데몬은 멀쩡히
// 뜨고 status도 레이어 목록을 그대로 뿌리지만 키는 하나도 잡히지 않는다.
// 실제 설치 결과를 여기에 기록해 `kmd daemon status`가 조용한 실패를 드러낸다.

/// 훅 설치 실패 사유. None = 정상 동작 중.
static HOOK_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// 훅 설치 결과 기록 — 성공 시 None, 실패 시 사용자에게 보여줄 사유.
pub fn set_hook_error(reason: Option<String>) {
    if let Ok(mut g) = HOOK_ERROR.lock() {
        *g = reason;
    }
}

/// 현재 훅 설치 실패 사유 (없으면 None)
pub fn hook_error() -> Option<String> {
    HOOK_ERROR.lock().ok().and_then(|g| g.clone())
}

// ── 가상 키 코드 (플랫폼 공용) ──────────────────────────────────────────────

/// 플랫폼 독립적 가상 키 식별자
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VKey {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    /// F19 — 트리거 전용. macOS에서 CapsLock을 hidutil로 F19에 재맵해
    /// LED-토글 문제 없이 깨끗한 홀드 트리거로 쓰기 위한 것 (docs/13).
    F19,
    Escape,
    Tab,
    CapsLock,
    Space,
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    PrintScreen,
    ScrollLock,
    Pause,
    LShift,
    RShift,
    LCtrl,
    RCtrl,
    LAlt,
    RAlt,
    LWin,
    RWin,
    Semicolon,
    Quote,
    Comma,
    Period,
    Slash,
    Backslash,
    LBracket,
    RBracket,
    Minus,
    Equal,
    Grave,
    Hangul,
    Hanja,
}

/// 키 이름 문자열 → VKey 파싱 (config 파일 파싱 시 사용)
impl VKey {
    /// 모든 VKey 변형 목록 — 플랫폼별 역방향 키코드 조회를 정방향 변환에서
    /// 자동 생성하는 데 쓴다 (거울상 match 이중 유지 방지).
    /// 변형을 추가하면 이 목록에도 추가한다. 정방향 변환(match)은 컴파일러가
    /// 누락을 잡아주고, 이 목록의 누락은 각 플랫폼의 왕복 테스트가 잡는다.
    // 현재 사용처(windows.rs의 역방향 맵·왕복 테스트)가 cfg(windows) 뒤에 있어
    // macOS 빌드에서만 dead_code로 보인다 — 삭제하면 Windows가 깨진다.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub const ALL: &'static [VKey] = &[
        VKey::A,
        VKey::B,
        VKey::C,
        VKey::D,
        VKey::E,
        VKey::F,
        VKey::G,
        VKey::H,
        VKey::I,
        VKey::J,
        VKey::K,
        VKey::L,
        VKey::M,
        VKey::N,
        VKey::O,
        VKey::P,
        VKey::Q,
        VKey::R,
        VKey::S,
        VKey::T,
        VKey::U,
        VKey::V,
        VKey::W,
        VKey::X,
        VKey::Y,
        VKey::Z,
        VKey::Num0,
        VKey::Num1,
        VKey::Num2,
        VKey::Num3,
        VKey::Num4,
        VKey::Num5,
        VKey::Num6,
        VKey::Num7,
        VKey::Num8,
        VKey::Num9,
        VKey::F1,
        VKey::F2,
        VKey::F3,
        VKey::F4,
        VKey::F5,
        VKey::F6,
        VKey::F7,
        VKey::F8,
        VKey::F9,
        VKey::F10,
        VKey::F11,
        VKey::F12,
        VKey::F19,
        VKey::Escape,
        VKey::Tab,
        VKey::CapsLock,
        VKey::Space,
        VKey::Enter,
        VKey::Backspace,
        VKey::Delete,
        VKey::Left,
        VKey::Right,
        VKey::Up,
        VKey::Down,
        VKey::Home,
        VKey::End,
        VKey::PageUp,
        VKey::PageDown,
        VKey::Insert,
        VKey::PrintScreen,
        VKey::ScrollLock,
        VKey::Pause,
        VKey::LShift,
        VKey::RShift,
        VKey::LCtrl,
        VKey::RCtrl,
        VKey::LAlt,
        VKey::RAlt,
        VKey::LWin,
        VKey::RWin,
        VKey::Semicolon,
        VKey::Quote,
        VKey::Comma,
        VKey::Period,
        VKey::Slash,
        VKey::Backslash,
        VKey::LBracket,
        VKey::RBracket,
        VKey::Minus,
        VKey::Equal,
        VKey::Grave,
        VKey::Hangul,
        VKey::Hanja,
    ];

    pub fn from_name(name: &str) -> Option<Self> {
        let n = name.trim().to_ascii_lowercase();
        match n.as_str() {
            "a" => Some(Self::A),
            "b" => Some(Self::B),
            "c" => Some(Self::C),
            "d" => Some(Self::D),
            "e" => Some(Self::E),
            "f" => Some(Self::F),
            "g" => Some(Self::G),
            "h" => Some(Self::H),
            "i" => Some(Self::I),
            "j" => Some(Self::J),
            "k" => Some(Self::K),
            "l" => Some(Self::L),
            "m" => Some(Self::M),
            "n" => Some(Self::N),
            "o" => Some(Self::O),
            "p" => Some(Self::P),
            "q" => Some(Self::Q),
            "r" => Some(Self::R),
            "s" => Some(Self::S),
            "t" => Some(Self::T),
            "u" => Some(Self::U),
            "v" => Some(Self::V),
            "w" => Some(Self::W),
            "x" => Some(Self::X),
            "y" => Some(Self::Y),
            "z" => Some(Self::Z),
            "0" => Some(Self::Num0),
            "1" => Some(Self::Num1),
            "2" => Some(Self::Num2),
            "3" => Some(Self::Num3),
            "4" => Some(Self::Num4),
            "5" => Some(Self::Num5),
            "6" => Some(Self::Num6),
            "7" => Some(Self::Num7),
            "8" => Some(Self::Num8),
            "9" => Some(Self::Num9),
            "f1" => Some(Self::F1),
            "f2" => Some(Self::F2),
            "f3" => Some(Self::F3),
            "f4" => Some(Self::F4),
            "f5" => Some(Self::F5),
            "f6" => Some(Self::F6),
            "f7" => Some(Self::F7),
            "f8" => Some(Self::F8),
            "f9" => Some(Self::F9),
            "f10" => Some(Self::F10),
            "f11" => Some(Self::F11),
            "f12" => Some(Self::F12),
            "f19" => Some(Self::F19),
            "esc" | "escape" => Some(Self::Escape),
            "tab" => Some(Self::Tab),
            "caps" | "capslock" => Some(Self::CapsLock),
            "space" | "spc" => Some(Self::Space),
            "enter" | "return" | "ret" => Some(Self::Enter),
            "backspace" | "bspc" | "bs" => Some(Self::Backspace),
            "delete" | "del" => Some(Self::Delete),
            "left" => Some(Self::Left),
            "right" | "rght" => Some(Self::Right),
            "up" => Some(Self::Up),
            "down" => Some(Self::Down),
            "home" => Some(Self::Home),
            "end" => Some(Self::End),
            "pageup" | "pgup" => Some(Self::PageUp),
            "pagedown" | "pgdn" => Some(Self::PageDown),
            "insert" | "ins" => Some(Self::Insert),
            "printscreen" | "prtsc" => Some(Self::PrintScreen),
            "scrolllock" => Some(Self::ScrollLock),
            "pause" => Some(Self::Pause),
            "lshift" => Some(Self::LShift),
            "rshift" => Some(Self::RShift),
            "lctrl" | "lcontrol" => Some(Self::LCtrl),
            "rctrl" | "rcontrol" => Some(Self::RCtrl),
            "lalt" => Some(Self::LAlt),
            "ralt" => Some(Self::RAlt),
            "lwin" | "lsuper" | "lmeta" | "lcmd" | "cmd" => Some(Self::LWin),
            "rwin" | "rsuper" | "rmeta" | "rcmd" => Some(Self::RWin),
            ";" | "semicolon" => Some(Self::Semicolon),
            "'" | "quote" => Some(Self::Quote),
            "," | "comma" => Some(Self::Comma),
            "." | "period" | "dot" => Some(Self::Period),
            "/" | "slash" => Some(Self::Slash),
            "\\" | "backslash" => Some(Self::Backslash),
            "[" | "lbracket" => Some(Self::LBracket),
            "]" | "rbracket" => Some(Self::RBracket),
            "-" | "minus" => Some(Self::Minus),
            "=" | "equal" => Some(Self::Equal),
            "`" | "grave" => Some(Self::Grave),
            "hangul" | "han" | "kor" => Some(Self::Hangul),
            "hanja" => Some(Self::Hanja),
            _ => None,
        }
    }
}

// ── Launch 커맨드 경로 해석 ──────────────────────────────────────────────────

/// `launch:name` 액션에서 실행 경로를 결정한다.
/// 실행 파일과 같은 디렉토리에서 먼저 찾고, 없으면 원래 이름을 반환한다.
/// 경로 순회(../) 공격을 방지하기 위해 파일 이름만 허용.
pub fn resolve_launch_cmd(name: &str) -> String {
    let safe_name = std::path::Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(name);

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            #[cfg(windows)]
            let bin = format!("{safe_name}.exe");
            #[cfg(not(windows))]
            let bin = safe_name.to_string();

            let sibling = dir.join(&bin);
            if sibling.exists() {
                return sibling.to_string_lossy().into_owned();
            }
        }
    }
    safe_name.to_string()
}

// ── 붙여넣기 주입 (확장 능력 스파이크, docs/11) ──────────────────────────────
//
// [P3 프로토타입] 클립보드 확장이 현재 분리 구조에서 가능한지 실증하기 위한 것.
// 데스크톱이 클립보드를 세팅한 뒤 이 함수를 IPC로 호출하면, 데몬이 현재 전경
// 앱에 붙여넣기 단축키(macOS Cmd+V / Windows·Linux Ctrl+V)를 주입한다.
// 데몬이 클립보드 자체를 만질 필요가 없어 가볍게 유지된다.
pub fn inject_paste() {
    #[cfg(target_os = "macos")]
    macos::inject_paste();
    #[cfg(windows)]
    windows::inject_paste();
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        tracing::info!("inject_paste: 이 플랫폼은 아직 미지원");
    }
}

// ── 수정자 키 헬퍼 (플랫폼 공용) ─────────────────────────────────────────────

pub fn is_modifier_key(vkey: &VKey) -> bool {
    matches!(
        vkey,
        VKey::LShift
            | VKey::RShift
            | VKey::LCtrl
            | VKey::RCtrl
            | VKey::LAlt
            | VKey::RAlt
            | VKey::LWin
            | VKey::RWin
    )
}

pub fn modifier_satisfied(m: &Modifier, held: &HashSet<VKey>) -> bool {
    match m {
        Modifier::Shift => held.contains(&VKey::LShift) || held.contains(&VKey::RShift),
        Modifier::Ctrl => held.contains(&VKey::LCtrl) || held.contains(&VKey::RCtrl),
        Modifier::Alt => held.contains(&VKey::LAlt) || held.contains(&VKey::RAlt),
        Modifier::Win => held.contains(&VKey::LWin) || held.contains(&VKey::RWin),
    }
}

// ── 바인딩 액션 ─────────────────────────────────────────────────────────────

/// 키 바인딩이 트리거되었을 때 수행할 동작
///
/// Windows 저수준 훅 콜백에서 매크로를 단일 `SendInput` 배치로 실행하므로,
/// 사용자 설정이 콜백 시간을 무제한 늘리지 못하게 입력 이벤트 수를 제한한다.
const MAX_MACRO_INPUT_EVENTS: usize = 64;

#[derive(Debug, Clone)]
pub enum BindAction {
    /// 다른 키 하나를 전송
    SendKey(VKey),
    /// 수정자+키 조합 전송 (예: Ctrl+C)
    SendCombo { modifiers: Vec<VKey>, key: VKey },
    /// 매크로: 여러 키 순차 전송
    Macro(Vec<MacroStep>),
    /// 외부 프로그램 실행
    Launch(String),
    /// 마우스 조작 — 키 down/up이 시작/정지에 대응하는 상태형 액션.
    /// 엔진이 [`engine::KeyDecision::MouseEngage`]/`MouseRelease`로 변환한다.
    Mouse(MouseBind),
    /// 클립보드 히스토리 n번째(1-기반) 항목을 전경 앱에 붙여넣기 (docs/12 흐름 A).
    ClipPaste(usize),
}

/// 마우스 바인딩 종류
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseBind {
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    BtnLeft,
    BtnRight,
    BtnMiddle,
    WheelUp,
    WheelDown,
    /// 홀드 중 이동 속도를 낮추는 정밀 모드
    Slow,
}

/// 매크로 한 스텝
#[derive(Debug, Clone)]
pub enum MacroStep {
    KeyPress(VKey),
    KeyRelease(VKey),
    Combo { modifiers: Vec<VKey>, key: VKey },
}

// ── 수정자 / 콤보 / 더블탭 ──────────────────────────────────────────────────

/// 수정자 키 종류 (좌/우 구분 없이 매칭)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Modifier {
    Shift,
    Ctrl,
    Alt,
    Win,
}

/// 수정자+키 조합 트리거 (예: Shift+Space)
#[derive(Debug, Clone)]
pub struct ComboTrigger {
    pub modifiers: Vec<Modifier>,
    pub key: VKey,
}

/// 더블탭 바인딩: 같은 키를 빠르게 두 번 탭하면 액션 실행
#[derive(Debug, Clone)]
pub struct DoubleTapBinding {
    pub key: VKey,
    pub action: BindAction,
    pub timeout_ms: u32,
}

// ── 레이어 ──────────────────────────────────────────────────────────────────

/// 레이어 활성 중 매핑되지 않은 키의 처리 방식 (VIA의 KC_TRNS/KC_NO 대응)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnmappedBehavior {
    /// 맨키만 통과 — 트리거는 억제된 상태이므로 OS 조합(Alt+Tab 등)은 죽는다 (현행)
    #[default]
    Plain,
    /// 트리거 조합을 OS로 투과 — 코드(chord) 모드로 진입해 Alt+Tab 등이 그대로 동작.
    /// 트리거가 수정자 키일 때만 성립하며, 아니면 Plain으로 폴백한다.
    Passthrough,
    /// 미매핑 키 억제 (VIA의 KC_NO)
    Block,
}

impl UnmappedBehavior {
    /// TOML 문자열 파싱 — 미지 값은 None (호출부에서 경고 후 기본값 사용)
    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "plain" => Some(Self::Plain),
            "passthrough" => Some(Self::Passthrough),
            "block" => Some(Self::Block),
            _ => None,
        }
    }
}

/// 키 레이어: 특정 키를 홀드하면 활성화되는 리매핑 세트
#[derive(Debug, Clone)]
pub struct Layer {
    pub name: String,
    /// 이 레이어를 활성화하는 트리거 키
    pub trigger: VKey,
    /// 트리거로 함께 인정하는 별칭 키.
    /// Windows 한국어 배열은 물리 오른쪽 Alt가 VK_HANGUL로 들어오므로
    /// trigger=RAlt인 레이어는 Hangul도 트리거로 잡는다.
    pub trigger_aliases: Vec<VKey>,
    /// 트리거 키를 짧게 탭했을 때 보낼 키 (tap-hold)
    pub tap_action: Option<VKey>,
    /// tap-hold 판정 시간 (밀리초)
    pub tap_hold_ms: u32,
    /// 매핑되지 않은 키의 처리 방식
    pub unmapped: UnmappedBehavior,
    /// 레이어 활성 시 키 매핑 (즉시 실행)
    pub mappings: HashMap<VKey, BindAction>,
    /// 레이어 내 더블탭 매핑: 첫 탭 → single_action, 두 번째 탭(timeout 이내) → double_action
    pub double_tap_mappings: HashMap<VKey, LayerDoubleTap>,
}

impl Layer {
    /// 이 키가 레이어 트리거(별칭 포함)인지
    pub fn matches_trigger(&self, vkey: VKey) -> bool {
        self.trigger == vkey || self.trigger_aliases.contains(&vkey)
    }
}

/// tap-hold(모드탭) 바인딩: 짧게 탭 = tap 키, 홀드 중 다른 키 = hold 수정자 조합.
/// 예: CapsLock — tap=CapsLock, hold=LCtrl (HHKB 스타일)
#[derive(Debug, Clone)]
pub struct TapHoldBinding {
    /// 물리 키
    pub key: VKey,
    /// 짧게 탭했을 때 보낼 키 (None이면 탭 무동작)
    pub tap: Option<VKey>,
    /// 홀드 중 다른 키와 조합할 수정자 키
    pub hold: VKey,
    /// tap 판정 시간 (밀리초)
    pub timeout_ms: u32,
}

/// 레이어 내 더블탭 설정
#[derive(Debug, Clone)]
pub struct LayerDoubleTap {
    pub single_action: BindAction,
    pub double_action: BindAction,
    pub timeout_ms: u32,
}

// ── 전체 키 바인딩 설정 ─────────────────────────────────────────────────────

/// 키 바인딩 엔진 설정
#[derive(Debug, Clone)]
pub struct KeybindConfig {
    /// 항상 활성화되는 단순 리매핑
    pub remaps: HashMap<VKey, BindAction>,
    /// 레이어 목록
    pub layers: Vec<Layer>,
    /// 수정자+키 콤보 바인딩 (예: Shift+Space → 한영 전환)
    pub combos: Vec<(ComboTrigger, BindAction)>,
    /// 더블탭 바인딩 (예: RShift 두 번 → 한영 전환)
    pub double_taps: Vec<DoubleTapBinding>,
    /// tap-hold(모드탭) 바인딩 (예: CapsLock — 탭=CapsLock, 홀드=Ctrl)
    pub tap_holds: Vec<TapHoldBinding>,
    /// Runtime toggle hotkey. When disabled, this and Launch combos still run.
    pub toggle_keymap: Option<ComboTrigger>,
    /// 손버릇 통계를 센다 (`[launcher.keymap] stats`, docs/19) — 기본 꺼짐
    pub stats: bool,
}

impl KeybindConfig {
    pub fn empty() -> Self {
        Self {
            remaps: HashMap::new(),
            layers: Vec::new(),
            combos: Vec::new(),
            double_taps: Vec::new(),
            tap_holds: Vec::new(),
            toggle_keymap: None,
            stats: false,
        }
    }
}

// ── TOML 설정 → KeybindConfig 변환 ───────────────────────────────────────────
//
// 프리셋 기본값(vim-nav/minimal)과 사용자 config 병합은 kmd-core의
// keymap::effective_keymap이 단일 소스로 수행한다. 이 모듈은 병합이 끝난
// TOML 설정을 런타임 타입으로 변환만 한다.

impl KeybindConfig {
    /// kmd-core의 TOML 설정을 런타임 KeybindConfig로 변환
    pub fn from_config(cfg: &kmd_core::config::KeymapConfig) -> Option<Self> {
        let has_custom = !cfg.remaps.is_empty()
            || !cfg.layers.is_empty()
            || !cfg.combos.is_empty()
            || !cfg.double_taps.is_empty()
            || !cfg.tap_holds.is_empty();
        if !has_custom {
            return None;
        }

        let mut remaps = HashMap::new();
        for (key_str, action_str) in &cfg.remaps {
            if let (Some(key), Some(action)) = (VKey::from_name(key_str), parse_action(action_str))
            {
                remaps.insert(key, action);
            } else {
                tracing::warn!("리맵 파싱 실패: {key_str} = {action_str}");
            }
        }

        // config 맵은 BTreeMap — 이름순 순회라 트리거가 겹쳐도 이기는 레이어가 고정된다
        // (예전 HashMap은 실행마다 순서가 달라 승자가 바뀌었다. 엔진은 첫 일치를 고른다).
        let mut layers: Vec<Layer> = Vec::new();
        for (name, layer_cfg) in &cfg.layers {
            if layer_cfg.trigger.is_empty() {
                tracing::warn!("레이어 '{name}': trigger가 비어있어 건너뜀");
                continue;
            }
            let Some(trigger) = VKey::from_name(&layer_cfg.trigger) else {
                tracing::warn!(
                    "레이어 '{name}': 알 수 없는 trigger '{}'",
                    layer_cfg.trigger
                );
                continue;
            };

            let tap_action = layer_cfg.tap_action.as_deref().and_then(VKey::from_name);

            let mut mappings = HashMap::new();
            for (k, v) in &layer_cfg.mappings {
                if let (Some(key), Some(action)) = (VKey::from_name(k), parse_action(v)) {
                    mappings.insert(key, action);
                } else {
                    tracing::warn!("레이어 '{name}' 매핑 파싱 실패: {k} = {v}");
                }
            }

            let mut double_tap_mappings = HashMap::new();
            for (k, dt) in &layer_cfg.double_taps {
                let Some(key) = VKey::from_name(k) else {
                    tracing::warn!("레이어 '{name}' 더블탭 키 파싱 실패: {k}");
                    continue;
                };
                let (Some(single), Some(double)) =
                    (parse_action(&dt.single), parse_action(&dt.double))
                else {
                    tracing::warn!(
                        "레이어 '{name}' 더블탭 액션 파싱 실패: {k} (single={}, double={})",
                        dt.single,
                        dt.double
                    );
                    continue;
                };
                double_tap_mappings.insert(
                    key,
                    LayerDoubleTap {
                        single_action: single,
                        double_action: double,
                        timeout_ms: dt.timeout_ms.unwrap_or(300),
                    },
                );
            }

            let unmapped = match layer_cfg.unmapped.as_deref() {
                None => UnmappedBehavior::default(),
                Some(s) => UnmappedBehavior::from_name(s).unwrap_or_else(|| {
                    tracing::warn!("레이어 '{name}' unmapped 값 파싱 실패: {s} — plain 사용");
                    UnmappedBehavior::default()
                }),
            };

            // Windows 한국어 배열: 물리 오른쪽 Alt = VK_HANGUL — 함께 트리거로 인정
            let trigger_aliases = if trigger == VKey::RAlt {
                vec![VKey::Hangul]
            } else {
                Vec::new()
            };

            if let Some(prev) = layers.iter().find(|l| l.trigger == trigger) {
                tracing::warn!(
                    "레이어 '{name}': trigger '{}'가 레이어 '{}'와 겹침 — '{}'가 우선",
                    layer_cfg.trigger,
                    prev.name,
                    prev.name
                );
            }
            layers.push(Layer {
                name: name.clone(),
                trigger,
                trigger_aliases,
                tap_action,
                tap_hold_ms: layer_cfg.tap_hold_ms.unwrap_or(200),
                unmapped,
                mappings,
                double_tap_mappings,
            });
        }

        let mut combos = Vec::new();
        for combo_cfg in &cfg.combos {
            if let (Some(trigger), Some(action)) = (
                parse_combo_trigger(&combo_cfg.trigger),
                parse_action(&combo_cfg.action),
            ) {
                combos.push((trigger, action));
            } else {
                tracing::warn!(
                    "콤보 파싱 실패: trigger={}, action={}",
                    combo_cfg.trigger,
                    combo_cfg.action
                );
            }
        }

        let mut double_taps = Vec::new();
        for dt_cfg in &cfg.double_taps {
            if let (Some(key), Some(action)) =
                (VKey::from_name(&dt_cfg.key), parse_action(&dt_cfg.action))
            {
                double_taps.push(DoubleTapBinding {
                    key,
                    action,
                    timeout_ms: dt_cfg.timeout_ms.unwrap_or(300),
                });
            } else {
                tracing::warn!(
                    "더블탭 파싱 실패: key={}, action={}",
                    dt_cfg.key,
                    dt_cfg.action
                );
            }
        }

        // 키 이름순 (BTreeMap) — 별칭이 같은 키로 풀리면 첫 항목이 이긴다
        let mut tap_holds: Vec<TapHoldBinding> = Vec::new();
        for (key_str, th_cfg) in &cfg.tap_holds {
            let Some(key) = VKey::from_name(key_str) else {
                tracing::warn!("tap-hold 키 파싱 실패: {key_str}");
                continue;
            };
            let Some(hold) = VKey::from_name(&th_cfg.hold) else {
                tracing::warn!("tap-hold '{key_str}' hold 파싱 실패: {}", th_cfg.hold);
                continue;
            };
            if !is_modifier_key(&hold) {
                tracing::warn!(
                    "tap-hold '{key_str}' hold는 수정자 키여야 함: {}",
                    th_cfg.hold
                );
                continue;
            }
            let tap = match th_cfg.tap.as_deref() {
                None => None,
                Some(t) => match VKey::from_name(t) {
                    Some(v) => Some(v),
                    None => {
                        tracing::warn!("tap-hold '{key_str}' tap 파싱 실패: {t}");
                        continue;
                    }
                },
            };
            if tap_holds.iter().any(|t| t.key == key) {
                tracing::warn!("tap-hold '{key_str}': 같은 키가 이미 정의됨 — 무시");
                continue;
            }
            tap_holds.push(TapHoldBinding {
                key,
                tap,
                hold,
                timeout_ms: th_cfg.timeout_ms.unwrap_or(200),
            });
        }

        tracing::info!(
            "커스텀 키 설정 로드: 리맵 {}개, 레이어 {}개, 콤보 {}개, 더블탭 {}개, 모드탭 {}개",
            remaps.len(),
            layers.len(),
            combos.len(),
            double_taps.len(),
            tap_holds.len()
        );

        Some(Self {
            remaps,
            layers,
            combos,
            double_taps,
            tap_holds,
            toggle_keymap: None,
            stats: cfg.stats,
        })
    }
}

/// 액션 문자열 파서.
///
/// 지원 형식:
/// - `"Home"` → SendKey(Home)
/// - `"Ctrl+Left"` → SendCombo { mods: [LCtrl], key: Left }
/// - `"Ctrl+Shift+End"` → SendCombo { mods: [LCtrl, LShift], key: End }
/// - `"launch:kmd-desktop"` → Launch("kmd-desktop")
/// - `"macro:Home;Shift+End;Ctrl+C"` → Macro([...])
/// - `"mouse:up"` / `"mouse:click"` / `"mouse:wheel-up"` / `"mouse:slow"` → Mouse(...)
fn parse_action(s: &str) -> Option<BindAction> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    if let Some(cmd) = s.strip_prefix("launch:") {
        return Some(BindAction::Launch(cmd.trim().to_string()));
    }

    if let Some(macro_str) = s.strip_prefix("macro:") {
        return parse_macro(macro_str);
    }

    if let Some(mouse_str) = s.strip_prefix("mouse:") {
        return parse_mouse_bind(mouse_str).map(BindAction::Mouse);
    }

    // 클립보드 히스토리 n번째 붙여넣기 (예: "clip:2")
    if let Some(n_str) = s.strip_prefix("clip:") {
        return n_str
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|&n| n >= 1)
            .map(BindAction::ClipPaste);
    }

    // 수정자+키 콤보 (예: "Ctrl+Left", "Ctrl+Shift+End")
    if s.contains('+') {
        if let Some((mod_strs, key)) = parse_combo_vkeys(s) {
            let modifiers: Vec<VKey> = mod_strs
                .iter()
                .filter_map(|m| parse_modifier_vkey(m.trim()))
                .collect();
            if !modifiers.is_empty() {
                return Some(BindAction::SendCombo { modifiers, key });
            }
        }
    }

    VKey::from_name(s).map(BindAction::SendKey)
}

/// 마우스 바인딩 문자열 파서 (`mouse:` 접두어 이후 부분)
fn parse_mouse_bind(s: &str) -> Option<MouseBind> {
    match s.trim().to_ascii_lowercase().as_str() {
        "up" => Some(MouseBind::MoveUp),
        "down" => Some(MouseBind::MoveDown),
        "left" => Some(MouseBind::MoveLeft),
        "right" => Some(MouseBind::MoveRight),
        "click" | "lclick" | "btn1" => Some(MouseBind::BtnLeft),
        "rclick" | "btn2" => Some(MouseBind::BtnRight),
        "mclick" | "btn3" => Some(MouseBind::BtnMiddle),
        "wheel-up" | "wheelup" => Some(MouseBind::WheelUp),
        "wheel-down" | "wheeldown" => Some(MouseBind::WheelDown),
        "slow" => Some(MouseBind::Slow),
        _ => None,
    }
}

/// 수정자 이름 → VKey (액션 실행용, L/R 구분)
fn parse_modifier_vkey(name: &str) -> Option<VKey> {
    match name.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(VKey::LCtrl),
        "shift" => Some(VKey::LShift),
        "alt" => Some(VKey::LAlt),
        "win" | "super" | "meta" | "cmd" | "command" => Some(VKey::LWin),
        _ => VKey::from_name(name),
    }
}

/// 수정자 이름 → Modifier (콤보 트리거용, L/R 무관)
fn parse_modifier(name: &str) -> Option<Modifier> {
    match name.to_ascii_lowercase().as_str() {
        "shift" | "lshift" | "rshift" => Some(Modifier::Shift),
        "ctrl" | "control" | "lctrl" | "rctrl" => Some(Modifier::Ctrl),
        "alt" | "lalt" | "ralt" => Some(Modifier::Alt),
        "win" | "super" | "meta" | "cmd" | "command" | "lwin" | "rwin" | "lcmd" | "rcmd" => {
            Some(Modifier::Win)
        }
        _ => None,
    }
}

/// 콤보 트리거 문자열 파서 (예: "Shift+Space", "Alt+Space" → ComboTrigger)
pub fn parse_combo_trigger(s: &str) -> Option<ComboTrigger> {
    let (mod_strs, key) = parse_combo_vkeys(s)?;
    let modifiers: Vec<Modifier> = mod_strs
        .iter()
        .filter_map(|m| parse_modifier(m.trim()))
        .collect();
    if modifiers.is_empty() {
        return None;
    }
    Some(ComboTrigger { modifiers, key })
}

/// 매크로 문자열 파서: "Home;Shift+End;Ctrl+C" → Macro steps
fn parse_macro(s: &str) -> Option<BindAction> {
    let mut steps = Vec::new();
    let mut input_event_count = 0usize;
    for part in s.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if part.contains('+') {
            // parse_combo_vkeys의 Vec 할당 전에도 거대한 사용자 콤보를 거부한다.
            let component_count = part.split('+').count();
            if component_count
                .checked_mul(2)
                .is_none_or(|count| count > MAX_MACRO_INPUT_EVENTS)
            {
                return None;
            }
            let (mod_strs, key) = parse_combo_vkeys(part)?;
            let modifiers: Vec<VKey> = mod_strs
                .iter()
                .filter_map(|m| parse_modifier_vkey(m.trim()))
                .collect();
            input_event_count = input_event_count.checked_add(modifiers.len() * 2 + 2)?;
            steps.push(MacroStep::Combo { modifiers, key });
        } else {
            let key = VKey::from_name(part)?;
            input_event_count = input_event_count.checked_add(2)?;
            steps.push(MacroStep::KeyPress(key));
            steps.push(MacroStep::KeyRelease(key));
        }
        if input_event_count > MAX_MACRO_INPUT_EVENTS {
            return None;
        }
    }
    if steps.is_empty() {
        None
    } else {
        Some(BindAction::Macro(steps))
    }
}

fn parse_combo_vkeys(s: &str) -> Option<(Vec<&str>, VKey)> {
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    if parts.len() < 2 {
        return None;
    }
    let (key_str, mod_strs) = parts.split_last()?;
    if key_str.is_empty() {
        return None;
    }
    if mod_strs.iter().any(|m| m.is_empty()) {
        return None;
    }
    let key = VKey::from_name(key_str)?;
    Some((mod_strs.to_vec(), key))
}

// ── Backend trait ────────────────────────────────────────────────────────────

/// 키보드 훅 백엔드 인터페이스 (플랫폼별 구현)
pub trait KeyboardBackend: Send {
    fn start(&mut self, config: KeybindConfig) -> Result<(), String>;
    fn stop(&mut self) -> Result<(), String>;
}

/// 쌓인 손버릇 통계를 꺼낸다 (docs/19). 훅이 없는 플랫폼(Linux 는 kanata)은 빈 목록.
pub fn take_key_stats() -> Vec<(String, u32)> {
    #[cfg(target_os = "macos")]
    {
        macos::take_key_stats()
    }
    #[cfg(windows)]
    {
        windows::take_key_stats()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Vec::new()
    }
}

/// 키보드 훅 생존 요약 — `kmd daemon status`로 노출한다.
///
/// Windows는 LL 훅을 조용히 제거하므로(Modern Standby 복귀, 콜백 타임아웃 등)
/// "데몬 실행 중"만으로는 키맵이 살아 있는지 알 수 없다. 훅 워치독이 남기는
/// 하트비트/재설치 횟수를 그대로 보여준다.
pub fn hook_health_summary() -> Option<String> {
    #[cfg(windows)]
    {
        let reinstalls = windows::hook_reinstall_count();
        let idle_ms = windows::hook_idle_ms()?;
        let reinstall_note = if reinstalls == 0 {
            String::new()
        } else {
            format!(" · 재설치 {reinstalls}회")
        };
        Some(format!(
            "설치됨 · 마지막 키 이벤트 {:.1}초 전{reinstall_note}",
            idle_ms as f64 / 1000.0
        ))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// 현재 플랫폼에 맞는 KeyboardBackend 생성
pub fn create_backend() -> Box<dyn KeyboardBackend> {
    #[cfg(windows)]
    {
        Box::new(windows::WindowsKeyboardBackend::new())
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacOSKeyboardBackend::new())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Box::new(StubBackend)
    }
}

/// 미구현 플랫폼 스텁
#[cfg(not(any(windows, target_os = "macos")))]
struct StubBackend;

#[cfg(not(any(windows, target_os = "macos")))]
impl KeyboardBackend for StubBackend {
    fn start(&mut self, _config: KeybindConfig) -> Result<(), String> {
        Err("이 플랫폼에서는 키 바인딩이 아직 지원되지 않습니다.".into())
    }
    fn stop(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vkey_from_name() {
        assert_eq!(VKey::from_name("h"), Some(VKey::H));
        assert_eq!(VKey::from_name("LAlt"), Some(VKey::LAlt));
        assert_eq!(VKey::from_name("escape"), Some(VKey::Escape));
        assert_eq!(VKey::from_name("esc"), Some(VKey::Escape));
        assert_eq!(VKey::from_name("pgup"), Some(VKey::PageUp));
        assert_eq!(VKey::from_name("unknown"), None);
    }

    /// kmd-core effective_keymap(단일 소스) 경유로 프리셋 → 엔진 설정 변환
    fn preset_config(profile: &str) -> Option<KeybindConfig> {
        let keymap = kmd_core::config::KeymapConfig {
            active_profile: profile.to_string(),
            ..Default::default()
        };
        let effective = kmd_core::keymap::effective_keymap(&keymap);
        KeybindConfig::from_config(&effective)
    }

    #[test]
    fn test_vim_nav_preset() {
        let config = preset_config("vim-nav").expect("vim-nav 프리셋은 항상 레이어를 가진다");
        assert_eq!(config.layers.len(), 2, "nav + mouse 레이어");

        // ── 마우스 레이어 (RAlt 홀드, 왼손 ESDF + W/R 클릭 + T/G 휠) ──
        let mouse = config
            .layers
            .iter()
            .find(|l| l.trigger == VKey::RAlt)
            .expect("RAlt 마우스 레이어");
        assert!(mouse.trigger_aliases.contains(&VKey::Hangul), "한/영 별칭");
        assert_eq!(
            mouse.tap_action,
            Some(VKey::Hangul),
            "RAlt 탭 = 한/영 (전 플랫폼 주 경로)"
        );
        assert_eq!(mouse.unmapped, UnmappedBehavior::Block);
        assert!(matches!(
            mouse.mappings.get(&VKey::E),
            Some(BindAction::Mouse(MouseBind::MoveUp))
        ));
        assert!(matches!(
            mouse.mappings.get(&VKey::F),
            Some(BindAction::Mouse(MouseBind::MoveRight))
        ));
        assert!(matches!(
            mouse.mappings.get(&VKey::W),
            Some(BindAction::Mouse(MouseBind::BtnLeft))
        ));
        assert!(matches!(
            mouse.mappings.get(&VKey::R),
            Some(BindAction::Mouse(MouseBind::BtnRight))
        ));
        assert!(matches!(
            mouse.mappings.get(&VKey::T),
            Some(BindAction::Mouse(MouseBind::WheelUp))
        ));
        assert!(matches!(
            mouse.mappings.get(&VKey::G),
            Some(BindAction::Mouse(MouseBind::WheelDown))
        ));
        assert!(
            !mouse.mappings.contains_key(&VKey::V) && !mouse.mappings.contains_key(&VKey::C),
            "구 배치(R/V 휠, C 우클릭) 폐기 — 클릭·휠은 공간 대응 배치로"
        );
        assert!(matches!(
            mouse.mappings.get(&VKey::Space),
            Some(BindAction::Mouse(MouseBind::BtnLeft))
        ));
        assert!(matches!(
            mouse.mappings.get(&VKey::K),
            Some(BindAction::Mouse(MouseBind::BtnRight))
        ));
        assert!(matches!(
            mouse.mappings.get(&VKey::LShift),
            Some(BindAction::Mouse(MouseBind::Slow))
        ));

        // ── 네비게이션 레이어 (0.13.0부터 기본 트리거 = CapsLock) ──
        let layer = config
            .layers
            .iter()
            .find(|l| l.trigger == VKey::CapsLock)
            .expect("CapsLock nav 레이어");
        assert_eq!(
            layer.tap_action, None,
            "CapsLock 탭 = 무동작 — 한영은 RAlt 탭으로 (2026-08-12)"
        );
        assert!(layer.mappings.contains_key(&VKey::H));
        assert!(layer.mappings.contains_key(&VKey::J));
        assert!(layer.mappings.contains_key(&VKey::N), "Alt+N은 PageUp 매핑");
        assert!(
            layer.mappings.contains_key(&VKey::Slash),
            "/는 더블탭 없는 평범한 Delete — 탭 연타 보존"
        );
        for k in [VKey::P, VKey::Y, VKey::U, VKey::Comma] {
            assert!(
                !layer.mappings.contains_key(&k),
                "P/Y/U/,는 제거 — 오발사 비용이 비싼 액션은 레이어에 두지 않는다"
            );
        }
        assert!(
            !layer.mappings.contains_key(&VKey::I) && !layer.mappings.contains_key(&VKey::O),
            "I/O는 double_tap_mappings 전용 — 평범한 매핑과 겹치지 않는다"
        );
        assert!(
            layer.double_tap_mappings.contains_key(&VKey::I),
            "I 더블탭 = 이전 단어 / 줄 처음"
        );
        assert!(
            layer.double_tap_mappings.contains_key(&VKey::O),
            "O 더블탭 = 다음 단어 / 줄 끝"
        );
        assert!(
            !layer.double_tap_mappings.contains_key(&VKey::U),
            "U/I 배치 폐기 (2026-08-20) — 위 행은 중지+약지, 검지는 H/J/N/M만"
        );
        assert!(
            !layer.double_tap_mappings.contains_key(&VKey::Slash),
            "삭제키 더블탭 제거 — 연타로 지우다 오발사되던 문제"
        );
        assert_eq!(
            config.combos.len(),
            1,
            "Shift+Space 한/영 콤보 (크로스플랫폼 통일)"
        );
        assert!(
            config.double_taps.is_empty(),
            "RShift 더블탭 기본 제거 — 한영 경로는 RAlt 탭으로 단일화"
        );
    }

    #[test]
    fn test_minimal_preset() {
        let config = preset_config("minimal").expect("minimal 프리셋은 CapsLock 기본을 가진다");
        #[cfg(target_os = "windows")]
        {
            // Windows: tap=Esc / hold=Ctrl 모드탭
            let th = config
                .tap_holds
                .iter()
                .find(|t| t.key == VKey::CapsLock)
                .expect("CapsLock tap-hold");
            assert_eq!(th.tap, Some(VKey::Escape));
            assert_eq!(th.hold, VKey::LCtrl);
            assert!(config.remaps.is_empty());
        }
        #[cfg(not(target_os = "windows"))]
        assert!(config.remaps.contains_key(&VKey::CapsLock));
        assert!(config.layers.is_empty());
        assert_eq!(
            config.combos.len(),
            1,
            "Shift+Space 한/영 콤보 (크로스플랫폼 통일)"
        );
        assert!(
            config.double_taps.is_empty(),
            "RShift 더블탭 기본 제거 (2026-08-12) — minimal은 물리 한/영 키가 그대로 동작"
        );
    }

    #[test]
    fn test_hangul_vkey() {
        assert_eq!(VKey::from_name("hangul"), Some(VKey::Hangul));
        assert_eq!(VKey::from_name("han"), Some(VKey::Hangul));
        assert_eq!(VKey::from_name("kor"), Some(VKey::Hangul));
        assert_eq!(VKey::from_name("hanja"), Some(VKey::Hanja));
    }

    #[test]
    fn test_parse_action_simple() {
        assert!(matches!(
            parse_action("Home"),
            Some(BindAction::SendKey(VKey::Home))
        ));
        assert!(matches!(
            parse_action("Escape"),
            Some(BindAction::SendKey(VKey::Escape))
        ));
        assert!(parse_action("").is_none());
        assert!(parse_action("nonexistent").is_none());
    }

    #[test]
    fn test_parse_action_combo() {
        let action = parse_action("Ctrl+Left").unwrap();
        match action {
            BindAction::SendCombo { modifiers, key } => {
                assert_eq!(modifiers, vec![VKey::LCtrl]);
                assert_eq!(key, VKey::Left);
            }
            _ => panic!("Ctrl+Left -> SendCombo 예상"),
        }

        let action = parse_action("Ctrl+Shift+End").unwrap();
        match action {
            BindAction::SendCombo { modifiers, key } => {
                assert_eq!(modifiers.len(), 2);
                assert_eq!(key, VKey::End);
            }
            _ => panic!("Ctrl+Shift+End -> SendCombo 예상"),
        }

        assert!(parse_action("Ctrl+").is_none(), "키 누락 콤보는 실패");
        assert!(parse_action("+End").is_none(), "수정자 누락 콤보는 실패");
    }

    #[test]
    fn test_parse_action_launch() {
        let action = parse_action("launch:kmd-desktop").unwrap();
        match action {
            BindAction::Launch(cmd) => assert_eq!(cmd, "kmd-desktop"),
            _ => panic!("launch 예상"),
        }
    }

    #[test]
    fn test_parse_action_macro() {
        let action = parse_action("macro:Home;Shift+End;Ctrl+C").unwrap();
        match action {
            BindAction::Macro(steps) => {
                assert_eq!(steps.len(), 4); // Home(press+release) + Shift+End(combo) + Ctrl+C(combo)
            }
            _ => panic!("macro 예상"),
        }
    }

    #[test]
    fn 매크로는_훅_콜백_입력_이벤트_상한을_지킨다() {
        let at_limit = std::iter::repeat_n("Home", MAX_MACRO_INPUT_EVENTS / 2)
            .collect::<Vec<_>>()
            .join(";");
        assert!(
            parse_action(&format!("macro:{at_limit}")).is_some(),
            "상한과 같은 매크로는 허용"
        );

        let over_limit = format!("{at_limit};Home");
        assert!(
            parse_action(&format!("macro:{over_limit}")).is_none(),
            "상한을 넘는 매크로는 설정 파싱 단계에서 거부"
        );

        let oversized_combo = format!("{}C", "Ctrl+".repeat(MAX_MACRO_INPUT_EVENTS / 2));
        assert!(
            parse_action(&format!("macro:{oversized_combo}")).is_none(),
            "단일 거대 콤보도 내부 Vec 할당 전에 거부"
        );
    }

    #[test]
    fn test_parse_combo_trigger() {
        let trigger = parse_combo_trigger("Shift+Space").unwrap();
        assert_eq!(trigger.modifiers, vec![Modifier::Shift]);
        assert_eq!(trigger.key, VKey::Space);

        assert!(
            parse_combo_trigger("Space").is_none(),
            "수정자 없는 트리거는 실패"
        );
        assert!(
            parse_combo_trigger("Shift+").is_none(),
            "키가 비어 있으면 실패"
        );
        assert!(
            parse_combo_trigger("+Space").is_none(),
            "수정자가 비어 있으면 실패"
        );
    }

    #[test]
    fn test_from_config_custom() {
        use kmd_core::config::*;
        let mut keymap = KeymapConfig {
            active_profile: "custom".to_string(),
            ..Default::default()
        };
        keymap.remaps.insert("CapsLock".into(), "Escape".into());
        keymap.combos.push(ComboToml {
            trigger: "Shift+Space".into(),
            action: "Hangul".into(),
        });
        keymap.double_taps.push(DoubleTapToml {
            key: "RShift".into(),
            action: "Hangul".into(),
            timeout_ms: Some(300),
        });

        let mut layer = LayerToml {
            trigger: "LAlt".into(),
            tap_action: Some("Escape".into()),
            ..Default::default()
        };
        layer.mappings.insert("H".into(), "Left".into());
        layer.mappings.insert("J".into(), "Down".into());
        layer.double_taps.insert(
            "I".into(),
            LayerDoubleTapToml {
                single: "Ctrl+Left".into(),
                double: "Home".into(),
                timeout_ms: Some(300),
            },
        );
        keymap.layers.insert("nav".into(), layer);

        let config = KeybindConfig::from_config(&keymap).unwrap();
        assert!(config.remaps.contains_key(&VKey::CapsLock));
        assert_eq!(config.layers.len(), 1);
        assert_eq!(config.layers[0].trigger, VKey::LAlt);
        assert!(config.layers[0].mappings.contains_key(&VKey::H));
        assert!(config.layers[0].double_tap_mappings.contains_key(&VKey::I));
        assert_eq!(config.combos.len(), 1);
        assert_eq!(config.double_taps.len(), 1);
    }
}
