//! Platform-specific window tweaks.

/// Apply Windows 11 native rounded corners to the target window.
///
/// The launcher window is opaque on Windows (per-pixel transparency is not
/// reliably composited by wgpu/DX12, and renders black on VMs/software
/// renderers). Instead of drawing rounded corners with transparent pixels,
/// we ask DWM to clip the window itself: `DWMWA_WINDOW_CORNER_PREFERENCE =
/// DWMWCP_ROUND`. On Windows 10 (no corner preference API) this fails
/// gracefully and the window stays square — but never black.
#[cfg(target_os = "windows")]
pub fn apply_native_rounded_corners(raw_id: u64) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
        DWM_WINDOW_CORNER_PREFERENCE,
    };

    unsafe {
        let hwnd = HWND(raw_id as usize as *mut core::ffi::c_void);
        if hwnd.0.is_null() {
            tracing::warn!("apply_native_rounded_corners: invalid window id");
            return;
        }

        let preference = DWMWCP_ROUND;
        let result = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &preference as *const DWM_WINDOW_CORNER_PREFERENCE as *const _,
            std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );

        match result {
            Ok(()) => tracing::info!("Windows 11 native rounded corners applied"),
            Err(e) => tracing::warn!("DwmSetWindowAttribute failed: {e}"),
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn apply_native_rounded_corners(_raw_id: u64) {
    // No-op on non-Windows platforms.
}

/// Switch the active keyboard layout to English (US) for the foreground window.
///
/// On Windows, after using Korean/Japanese/Chinese IME, the input language
/// persists across application focus changes. This forces English so the
/// launcher starts in a state ready for commands (@, :, !, etc.).
#[cfg(target_os = "windows")]
pub fn force_english_ime(raw_id: u64) {
    use windows::Win32::Foundation::{BOOL, HWND};
    use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmReleaseContext, ImmSetOpenStatus};

    unsafe {
        let hwnd = HWND(raw_id as usize as *mut core::ffi::c_void);
        if hwnd.0.is_null() {
            tracing::warn!("force_english_ime: invalid window id");
            return;
        }

        // Safer than ActivateKeyboardLayout:
        // only close IME for this window/context without changing global layout.
        let himc = ImmGetContext(hwnd);
        if himc.0.is_null() {
            tracing::warn!("force_english_ime: ImmGetContext returned null");
            return;
        }

        let ok = ImmSetOpenStatus(himc, BOOL(0));
        let _ = ImmReleaseContext(hwnd, himc);

        if ok.as_bool() {
            tracing::debug!("IME closed for launcher window (English input mode)");
        } else {
            tracing::warn!("force_english_ime: ImmSetOpenStatus failed");
        }
    }
}

/// Switch macOS input source to English when the launcher opens.
///
/// We use Carbon Text Input Source APIs because Iced itself does not expose
/// a cross-platform IME mode switch hook. Selecting an English source here
/// keeps command-first typing predictable on open.
///
/// **바꾸기 전의 입력 소스를 기억해 둔다** — 런처를 닫을 때
/// [`restore_input_source`]가 되돌린다. Windows 구현은 런처 창의 IME만 닫아
/// 다른 앱에 영향이 없는데, macOS의 TIS는 **시스템 전역**이라 예전엔 한글로
/// 쓰던 앱으로 돌아가도 영문으로 남았다(2026-10-07 두 OS 동작 통일).
#[cfg(target_os = "macos")]
pub fn force_english_ime(_raw_id: u64) {
    // TIS(Text Input Source) API는 메인 스레드 + 윈도우 서버 세션을 요구한다.
    // 단위 테스트는 워커 스레드(헤드리스 CI)에서 실행되어 HIToolbox가
    // abort()를 호출하므로(SIGABRT), 테스트 빌드에서는 건너뛴다.
    // (Windows 구현은 잘못된 HWND에서 에러 코드만 반환하므로 가드 불필요)
    if cfg!(test) {
        return;
    }

    use core_foundation_sys::base::{CFEqual, CFRelease, CFTypeRef};
    use core_foundation_sys::string::{kCFStringEncodingUTF8, CFStringCreateWithCString};
    use std::ptr;

    unsafe {
        let lang =
            CFStringCreateWithCString(ptr::null(), c"en".as_ptr().cast(), kCFStringEncodingUTF8);
        if lang.is_null() {
            tracing::warn!("force_english_ime(macos): failed to allocate CFString(en)");
            return;
        }

        let source = mac_tis::TISCopyInputSourceForLanguage(lang);
        CFRelease(lang as CFTypeRef);

        if source.is_null() {
            tracing::warn!("force_english_ime(macos): no English input source found");
            return;
        }

        // 지금 소스가 이미 영문이면 되돌릴 것도 없다. 아니면 기억한다 — 단, 이미
        // 기억한 게 있으면(포커스 재시도로 두 번 불림) 덮지 않는다: 두 번째 호출의
        // "현재"는 우리가 바꾼 영문이다.
        let current = mac_tis::TISCopyCurrentKeyboardInputSource();
        if !current.is_null() {
            let already_english = CFEqual(current as CFTypeRef, source as CFTypeRef) != 0;
            let mut prev = mac_tis::PREVIOUS_SOURCE
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if already_english || prev.is_some() {
                CFRelease(current as CFTypeRef);
            } else {
                *prev = Some(current as usize); // Copy로 받은 참조를 그대로 보관
            }
        }

        let status = mac_tis::TISSelectInputSource(source);
        CFRelease(source as CFTypeRef);

        if status == 0 {
            tracing::debug!("macOS input source switched to English");
        } else {
            tracing::warn!("force_english_ime(macos): TISSelectInputSource failed ({status})");
        }
    }
}

/// 런처를 열 때 바꿔 둔 입력 소스를 원래대로 되돌린다.
///
/// **런처 창이 아직 앞에 있을 때 불러야 한다** (창을 닫기 직전). TIS로 한글 같은
/// CJK 입력기를 고르면, 그 순간 앞에 있는 앱의 입력 컨텍스트는 따라오지 않는
/// "반쪽 전환"이 생긴다(e773363 — 메뉴바만 바뀌고 실제 입력은 그대로). 런처가
/// 앞에 있을 때 되돌리고 닫으면, 이전 앱은 활성화되면서 그 소스를 이어받는다.
/// 그래서 다른 앱을 클릭해 런처가 닫히는 경로(이미 다른 앱이 앞)에서는 부르지 않는다.
#[cfg(target_os = "macos")]
pub fn restore_input_source() {
    if cfg!(test) {
        return;
    }
    use core_foundation_sys::base::{CFRelease, CFTypeRef};
    let prev = mac_tis::PREVIOUS_SOURCE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    let Some(source) = prev else {
        return; // 바꾼 적 없음 (옵션 꺼짐, 원래 영문)
    };
    unsafe {
        let source = source as mac_tis::TISInputSourceRef;
        let status = mac_tis::TISSelectInputSource(source);
        CFRelease(source as CFTypeRef);
        if status == 0 {
            tracing::debug!("macOS input source restored");
        } else {
            tracing::warn!("restore_input_source(macos): TISSelectInputSource failed ({status})");
        }
    }
}

#[cfg(target_os = "macos")]
mod mac_tis {
    use core_foundation_sys::string::CFStringRef;
    use std::ffi::c_void;
    use std::sync::Mutex;

    pub type TISInputSourceRef = *mut c_void;
    pub type OSStatus = i32;

    /// 런처를 열기 전의 입력 소스 (retain된 TISInputSourceRef를 usize로 보관 —
    /// 원시 포인터는 Send가 아니라 static에 둘 수 없다).
    pub static PREVIOUS_SOURCE: Mutex<Option<usize>> = Mutex::new(None);

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        pub fn TISCopyInputSourceForLanguage(language: CFStringRef) -> TISInputSourceRef;
        pub fn TISCopyCurrentKeyboardInputSource() -> TISInputSourceRef;
        pub fn TISSelectInputSource(input_source: TISInputSourceRef) -> OSStatus;
    }
}

/// Windows는 런처 창의 IME만 닫았으므로(전역 상태 불변) 되돌릴 것이 없다.
#[cfg(not(target_os = "macos"))]
pub fn restore_input_source() {}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn force_english_ime(_raw_id: u64) {
    // No-op on unsupported platforms.
}

/// Bring the window to the foreground and give it keyboard focus.
///
/// Windows restricts `SetForegroundWindow` to the current foreground
/// process. We work around this by temporarily attaching our thread's
/// input queue to the foreground thread via `AttachThreadInput`, then
/// calling `ShowWindow(SW_SHOW)` + `SetForegroundWindow` + `SetFocus`.
///
/// This avoids the older `SendInput(ALT)` trick, which generated
/// spurious keyboard events that could cause UI flickering.
#[cfg(target_os = "windows")]
pub fn force_foreground(raw_id: u64) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow, ShowWindow, SW_SHOW,
    };

    unsafe {
        let hwnd = HWND(raw_id as usize as *mut core::ffi::c_void);
        if hwnd.0.is_null() {
            return;
        }

        let fg_hwnd = GetForegroundWindow();
        let fg_thread = GetWindowThreadProcessId(fg_hwnd, None);
        let my_thread = GetCurrentThreadId();

        if fg_thread != 0 && fg_thread != my_thread {
            let _ = AttachThreadInput(my_thread, fg_thread, true);
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(hwnd);
            let _ = AttachThreadInput(my_thread, fg_thread, false);
        } else {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(hwnd);
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn force_foreground(_raw_id: u64) {}

/// 현재 포그라운드 윈도우가 우리 윈도우인지 확인
#[cfg(target_os = "windows")]
pub fn is_our_window_foreground(raw_id: u64) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    unsafe {
        let hwnd = HWND(raw_id as usize as *mut core::ffi::c_void);
        if hwnd.0.is_null() {
            return false;
        }
        let fg = GetForegroundWindow();
        fg == hwnd
    }
}

// ─── 리사이즈 잔상(찢어짐) 방지 ────────────────────────────────────────────────
//
// 창 높이를 바꾸면 OS는 창을 즉시 새 크기로 만들지만, 렌더러(wgpu)가 새 크기의
// 프레임을 그려 present 하기까지는 최소 한 프레임의 공백이 있다. 그 사이 컴포지터가
// 무엇을 보여주는지가 "화면이 찌직 깨지는" 현상의 정체다.
//
// - macOS: wgpu 는 NSView 루트 레이어에 `CAMetalLayer` 서브레이어를 붙이고 KVO 로
//   루트 bounds 를 따라간다. 이 레이어의 `contentsGravity` 기본값이 `resize` 라서
//   이전 프레임이 새 창 크기로 **늘려/찌그러뜨려** 합성된다. 게다가 뷰가 직접 소유한
//   레이어가 아니므로 암묵적 애니메이션(기본 0.25초)까지 걸려 왜곡이 한참 보인다.
//   → gravity 를 `topLeft` 로 바꾸면 늘리는 대신 좌상단에 고정(확대 시 아래쪽은 빈
//     영역, 축소 시 위쪽만 남김)되고, actions 를 NSNull 로 덮어 애니메이션을 없앤다.
// - Windows: DXGI 스왑체인은 `DXGI_SCALING_STRETCH` 로 고정돼 있어 같은 교정이
//   불가능하다. 대신 리사이즈 순간에만 DWM 합성에서 창을 잠깐 제외(cloak)한다.

/// macOS: wgpu 가 만든 `CAMetalLayer` 의 리사이즈 거동을 교정한다.
///
/// 성공(레이어를 찾아 적용) 시 `true`. 소프트웨어 렌더러(tiny-skia) 폴백이거나
/// 아직 레이어가 만들어지지 않았으면 `false` — 호출부에서 재시도하면 된다.
#[cfg(target_os = "macos")]
pub fn stabilize_surface_layer() -> bool {
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{msg_send, MainThreadMarker};

    // AppKit 접근은 메인 스레드 전용. 단위 테스트는 워커 스레드에서 돌므로 건너뛴다.
    if cfg!(test) || MainThreadMarker::new().is_none() {
        return false;
    }

    unsafe {
        let Some(app_cls) = AnyClass::get(c"NSApplication") else {
            return false;
        };
        let app: *mut AnyObject = msg_send![app_cls, sharedApplication];
        if app.is_null() {
            return false;
        }
        let windows: *mut AnyObject = msg_send![app, windows];
        if windows.is_null() {
            return false;
        }
        let count: usize = msg_send![windows, count];

        let mut patched = 0usize;
        for i in 0..count {
            let win: *mut AnyObject = msg_send![windows, objectAtIndex: i];
            if win.is_null() {
                continue;
            }
            let view: *mut AnyObject = msg_send![win, contentView];
            if view.is_null() {
                continue;
            }
            let root: *mut AnyObject = msg_send![view, layer];
            if root.is_null() {
                continue;
            }
            if let Some(metal) = find_metal_layer(root, 0) {
                pin_layer_contents(metal);
                patched += 1;
            }
        }

        if patched > 0 {
            tracing::info!("CAMetalLayer 리사이즈 고정 적용 ({patched}개 레이어)");
        }
        patched > 0
    }
}

/// 레이어 트리에서 `CAMetalLayer` 를 찾는다 (루트 자신이거나 서브레이어).
#[cfg(target_os = "macos")]
unsafe fn find_metal_layer(
    layer: *mut objc2::runtime::AnyObject,
    depth: u32,
) -> Option<*mut objc2::runtime::AnyObject> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    // wgpu 는 루트 바로 아래에 한 겹만 붙이지만, 프레임워크가 래핑을 추가해도
    // 견디도록 몇 단계까지 훑는다.
    if depth > 4 {
        return None;
    }
    let metal_cls = AnyClass::get(c"CAMetalLayer")?;
    let is_metal: bool = msg_send![layer, isKindOfClass: metal_cls];
    if is_metal {
        return Some(layer);
    }

    let subs: *mut AnyObject = msg_send![layer, sublayers];
    if subs.is_null() {
        return None;
    }
    let count: usize = msg_send![subs, count];
    for i in 0..count {
        let sub: *mut AnyObject = msg_send![subs, objectAtIndex: i];
        if sub.is_null() {
            continue;
        }
        if let Some(found) = find_metal_layer(sub, depth + 1) {
            return Some(found);
        }
    }
    None
}

/// 이전 프레임을 늘리지 않고 좌상단에 고정 + 암묵적 애니메이션 제거.
#[cfg(target_os = "macos")]
unsafe fn pin_layer_contents(layer: *mut objc2::runtime::AnyObject) {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSString;

    // kCAGravityTopLeft 의 실제 값은 문자열 "topLeft" (QuartzCore 상수).
    let gravity = NSString::from_str("topLeft");
    let _: () = msg_send![layer, setContentsGravity: &*gravity];

    // 뷰가 소유한 레이어가 아니므로 AppKit 이 암묵적 애니메이션을 꺼주지 않는다.
    // bounds/position 이 애니메이션되면 리사이즈 왜곡이 0.25초간 그대로 보인다.
    let (Some(dict_cls), Some(null_cls)) = (
        AnyClass::get(c"NSMutableDictionary"),
        AnyClass::get(c"NSNull"),
    ) else {
        return;
    };
    let dict: *mut AnyObject = msg_send![dict_cls, dictionary];
    let null: *mut AnyObject = msg_send![null_cls, null];
    if dict.is_null() || null.is_null() {
        return;
    }
    for key in [
        "bounds",
        "position",
        "frame",
        "contents",
        "sublayers",
        "onOrderIn",
        "onOrderOut",
        "hidden",
    ] {
        let k = NSString::from_str(key);
        let _: () = msg_send![dict, setObject: null, forKey: &*k];
    }
    let _: () = msg_send![layer, setActions: dict];
}

#[cfg(not(target_os = "macos"))]
pub fn stabilize_surface_layer() -> bool {
    // macOS 외에는 교정할 레이어가 없다 (Windows 는 cloak 경로를 쓴다).
    false
}

/// Windows: DWM 합성에서 창을 잠깐 제외한다(cloak).
///
/// `DXGI_SCALING_STRETCH` 로 고정된 스왑체인 때문에 리사이즈 직후 한두 프레임은
/// 이전 프레임이 새 창 크기로 늘어나 합성된다. cloak 은 포커스·Z오더·항상 위
/// 속성을 건드리지 않고 합성 대상에서만 빼므로, 새 프레임이 준비될 때까지
/// 왜곡된 프레임 대신 아무것도 보이지 않게 만든다.
///
/// DWM 이 없는 환경(서버 코어 등)에서는 실패를 반환하고 호출부가 그대로 진행한다.
#[cfg(target_os = "windows")]
pub fn set_window_cloaked(raw_id: u64, cloaked: bool) -> bool {
    use windows::Win32::Foundation::{BOOL, HWND};
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};

    unsafe {
        let hwnd = HWND(raw_id as usize as *mut core::ffi::c_void);
        if hwnd.0.is_null() {
            return false;
        }
        let value = BOOL(i32::from(cloaked));
        let result = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CLOAK,
            &value as *const BOOL as *const _,
            std::mem::size_of::<BOOL>() as u32,
        );
        match result {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("DwmSetWindowAttribute(DWMWA_CLOAK={cloaked}) 실패: {e}");
                false
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn set_window_cloaked(_raw_id: u64, _cloaked: bool) -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
pub fn is_our_window_foreground(_raw_id: u64) -> bool {
    // macOS/Linux: 앱 내부에서 window::Event::Focused/Unfocused로 추적.
    // CheckUnfocusedExit, EnsureFocus는 #[cfg(target_os="windows")]로 호출하지만
    // 시그니처 일관성을 위해 유지.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 플랫폼 함수 시그니처 존재 검증 (컴파일 가드).
    /// 이 함수 중 하나라도 삭제/이름 변경되면 컴파일 에러로 즉시 감지.
    /// v0.3.6 회귀 방지: force_foreground 누락 시 키보드 포커스 미동작.
    #[test]
    fn test_platform_functions_compile() {
        let _: fn(u64) = apply_native_rounded_corners;
        let _: fn(u64) = force_english_ime;
        let _: fn() = restore_input_source;
        let _: fn(u64) = force_foreground;
        let _: fn() -> bool = stabilize_surface_layer;
        let _: fn(u64, bool) -> bool = set_window_cloaked;
    }

    /// 헤드리스(워커 스레드) 환경에서 호출해도 패닉 없이 false 를 반환해야 한다.
    /// macOS 구현이 AppKit 을 건드리므로 메인 스레드 가드가 살아 있는지 검증한다.
    #[test]
    fn test_stabilize_surface_layer_is_headless_safe() {
        assert!(!stabilize_surface_layer());
    }
}
