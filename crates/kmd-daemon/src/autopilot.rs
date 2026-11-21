//! LLM 오토파일럿 — URL로 자동 실행이 안 되는 LLM(ChatGPT/Claude/Gemini)에
//! 대해 브라우저 창을 검증한 뒤 키(Enter/Ctrl+V)를 주입해 자동 제출한다.
//!
//! 키보드 훅 엔진과 완전히 분리된 독립 모듈이다 (자체 SendInput + Win32 창 조회).
//! 안전 핵심: **전경창이 알려진 브라우저 + 기대 타이틀 마커일 때만** 주입한다.
//! 조건 불충족 시 조용히 포기 → 프리필/클립보드가 남아 사용자가 수동 완료(회귀 없음).
//!
//! Windows 우선 구현. 그 외 플랫폼은 스텁(프런트엔드가 URL 폴백을 씀).

use kmd_core::ipc::LlmJob;

/// 새 대화 열기 + 자동 제출. 즉시 반환하고 전용 스레드에서 시퀀스를 수행한다
/// (전경창 폴링이 수 초 걸리므로 IPC 응답을 막지 않는다).
pub fn run_autopilot(jobs: Vec<LlmJob>) {
    if jobs.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        #[cfg(windows)]
        win::autopilot_sequence(&jobs);
        #[cfg(not(windows))]
        {
            let _ = &jobs;
            tracing::info!("LLM 오토파일럿은 현재 Windows에서만 지원됩니다");
        }
    });
}

/// 이어서 질문 — 기억된 LLM 창들에 후속 프롬프트를 전달.
pub fn run_followup(prompt: String) {
    if prompt.trim().is_empty() {
        return;
    }
    std::thread::spawn(move || {
        #[cfg(windows)]
        win::followup_sequence(&prompt);
        #[cfg(not(windows))]
        {
            let _ = &prompt;
            tracing::info!("LLM 이어서 질문은 현재 Windows에서만 지원됩니다");
        }
    });
}

/// 오토파일럿이 이번 세션에서 연 창이 있는지 (이어서 질문 가능 여부).
pub fn has_session() -> bool {
    #[cfg(windows)]
    {
        win::session_len() > 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
mod win {
    use kmd_core::ipc::{LlmInject, LlmJob};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{CloseHandle, HWND};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsWindow,
        SetForegroundWindow,
    };

    // ── 타이밍 상수 ──────────────────────────────────────────────────────────
    const POLL_INTERVAL: Duration = Duration::from_millis(120);
    const POLL_TIMEOUT: Duration = Duration::from_millis(8000);
    /// 창이 매치된 뒤 입력창 포커스가 안정될 때까지 대기 후 재검증
    const SETTLE_BEFORE_INJECT: Duration = Duration::from_millis(450);
    /// Ctrl+V 붙여넣기 반영 대기 (Enter 전)
    const PASTE_SETTLE: Duration = Duration::from_millis(250);
    /// SetForegroundWindow 후 포커스 안정 대기
    const FOCUS_SETTLE: Duration = Duration::from_millis(300);

    /// 알려진 브라우저 실행 파일 basename (소문자). 이 목록의 창에만 주입한다.
    const BROWSER_EXES: &[&str] = &[
        "chrome.exe",
        "msedge.exe",
        "firefox.exe",
        "brave.exe",
        "opera.exe",
        "vivaldi.exe",
        "whale.exe",
        "arc.exe",
        "chromium.exe",
        "librewolf.exe",
    ];

    /// 이어서 질문할 창 — 창 하나에 탭이 여럿이라 제목 마커도 함께 기억한다
    #[derive(Clone)]
    struct Session {
        service_id: String,
        /// HWND는 raw pointer라 isize로 저장해 스레드 간 이동을 안전하게 한다
        hwnd: isize,
        title_markers: Vec<String>,
    }

    static SESSION: Mutex<Vec<Session>> = Mutex::new(Vec::new());

    pub(super) fn session_len() -> usize {
        SESSION.lock().map(|g| g.len()).unwrap_or(0)
    }

    fn registry_upsert(service_id: &str, hwnd: isize, title_markers: &[String]) {
        if let Ok(mut g) = SESSION.lock() {
            g.retain(|s| s.service_id != service_id);
            g.push(Session {
                service_id: service_id.to_string(),
                hwnd,
                title_markers: title_markers.to_vec(),
            });
        }
    }

    // ── Win32 헬퍼 ───────────────────────────────────────────────────────────

    fn foreground_window() -> HWND {
        unsafe { GetForegroundWindow() }
    }

    fn window_title(hwnd: HWND) -> String {
        unsafe {
            let mut buf = [0u16; 512];
            let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            if len <= 0 {
                return String::new();
            }
            String::from_utf16_lossy(&buf[..len as usize])
        }
    }

    /// 창을 소유한 프로세스의 실행 파일 basename (소문자). 실패 시 빈 문자열.
    fn window_process_exe(hwnd: HWND) -> String {
        unsafe {
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == 0 {
                return String::new();
            }
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return String::new();
            }
            let mut buf = [0u16; 260];
            let mut size = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size);
            CloseHandle(handle);
            if ok == 0 || size == 0 {
                return String::new();
            }
            let full = String::from_utf16_lossy(&buf[..size as usize]);
            full.rsplit(['\\', '/'])
                .next()
                .unwrap_or(&full)
                .to_lowercase()
        }
    }

    fn is_browser(hwnd: HWND) -> bool {
        let exe = window_process_exe(hwnd);
        BROWSER_EXES.iter().any(|b| b == &exe)
    }

    fn title_matches(hwnd: HWND, markers: &[String]) -> bool {
        let title = window_title(hwnd);
        // 빈 마커는 모든 제목과 맞으므로 세지 않는다
        markers
            .iter()
            .any(|m| !m.is_empty() && title.contains(m.as_str()))
    }

    /// 전경창이 브라우저 + 기대 마커가 될 때까지 폴링. 매치되면 그 HWND 반환.
    fn poll_foreground_match(markers: &[String]) -> Option<HWND> {
        let start = Instant::now();
        while start.elapsed() < POLL_TIMEOUT {
            let fg = foreground_window();
            if !fg.is_null() && is_browser(fg) && title_matches(fg, markers) {
                return Some(fg);
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        None
    }

    // ── 키 주입 (자체 SendInput) ─────────────────────────────────────────────

    fn key_event(vk: u16, keyup: bool) {
        unsafe {
            let mut input: INPUT = std::mem::zeroed();
            input.r#type = INPUT_KEYBOARD;
            input.Anonymous.ki.wVk = vk;
            input.Anonymous.ki.dwFlags = if keyup { KEYEVENTF_KEYUP } else { 0 };
            SendInput(1, &input, std::mem::size_of::<INPUT>() as i32);
        }
    }

    fn press(vk: u16) {
        key_event(vk, false);
        key_event(vk, true);
    }

    fn send_enter() {
        press(VK_RETURN);
    }

    /// Ctrl+V (down/up 원자적 순서 — modifier가 홀드로 남지 않게)
    fn send_paste() {
        key_event(VK_CONTROL, false);
        press(0x56); // 'V'
        key_event(VK_CONTROL, true);
    }

    fn set_clipboard(text: &str) -> bool {
        match arboard::Clipboard::new() {
            Ok(mut cb) => cb.set_text(text.to_string()).is_ok(),
            Err(e) => {
                tracing::warn!("클립보드 접근 실패: {e}");
                false
            }
        }
    }

    // ── 시퀀스 ───────────────────────────────────────────────────────────────

    pub(super) fn autopilot_sequence(jobs: &[LlmJob]) {
        // 잡은 순차 처리 — 전경창은 공유 자원이라 병렬 주입 불가
        for job in jobs {
            if matches!(job.method, LlmInject::PasteEnter) {
                // 붙여넣기형: URL 열기 전에 클립보드부터 세팅
                if !set_clipboard(&job.prompt) {
                    tracing::warn!("[{}] 클립보드 세팅 실패 — 잡 건너뜀", job.service_id);
                    continue;
                }
            }

            if let kmd_core::action::ActionResult::Error(e) = kmd_core::action::open_url(&job.url) {
                tracing::warn!("[{}] URL 열기 실패: {e}", job.service_id);
                continue;
            }

            let Some(hwnd) = poll_foreground_match(&job.title_markers) else {
                // 게이트 미통과 — 자동 제출 포기(프리필/클립보드 남아 수동 완료 가능)
                tracing::info!(
                    "[{}] 전경창 검증 실패(타임아웃) — 자동 제출 건너뜀, 수동 완료 가능",
                    job.service_id
                );
                continue;
            };

            // 로딩 중 포커스가 튈 수 있으므로 안정 대기 후 재검증
            std::thread::sleep(SETTLE_BEFORE_INJECT);
            let fg = foreground_window();
            if fg != hwnd || !is_browser(fg) || !title_matches(fg, &job.title_markers) {
                tracing::info!(
                    "[{}] settle 후 재검증 실패 — 자동 제출 건너뜀",
                    job.service_id
                );
                continue;
            }

            match job.method {
                LlmInject::EnterOnly => send_enter(),
                LlmInject::PasteEnter => {
                    send_paste();
                    std::thread::sleep(PASTE_SETTLE);
                    // 붙여넣기 후에도 여전히 같은 창인지 마지막 확인
                    if foreground_window() == hwnd {
                        send_enter();
                    }
                }
            }

            registry_upsert(&job.service_id, hwnd as isize, &job.title_markers);
            tracing::info!("[{}] 자동 제출 완료", job.service_id);
        }
    }

    pub(super) fn followup_sequence(prompt: &str) {
        let entries: Vec<Session> = match SESSION.lock() {
            Ok(g) => g.clone(),
            Err(_) => return,
        };
        if entries.is_empty() {
            tracing::info!("이어서 질문할 LLM 창이 없습니다 (먼저 @llm 등으로 여세요)");
            return;
        }

        let mut still_alive: Vec<Session> = Vec::new();
        for session in entries {
            let service_id = session.service_id.as_str();
            let hwnd = session.hwnd as HWND;
            unsafe {
                if IsWindow(hwnd) == 0 {
                    continue; // 창이 닫힘 — 레지스트리에서 제거
                }
                SetForegroundWindow(hwnd);
            }
            std::thread::sleep(FOCUS_SETTLE);

            // 포커스가 실제로 그 창으로 갔고, 브라우저이며, 보이는 탭이 그 LLM 인지 검증.
            // 창은 같아도 다른 탭(메일·결제 폼)이 앞에 있으면 거기에 붙여넣고 제출하게 된다.
            let fg = foreground_window();
            if fg != hwnd || !is_browser(fg) || !title_matches(fg, &session.title_markers) {
                tracing::info!("[{service_id}] 창 또는 탭이 다름 — 이어서 질문 건너뜀");
                still_alive.push(session);
                continue;
            }

            if !set_clipboard(prompt) {
                still_alive.push(session);
                continue;
            }
            send_paste();
            std::thread::sleep(PASTE_SETTLE);
            if foreground_window() == hwnd {
                send_enter();
                tracing::info!("[{service_id}] 이어서 질문 전달 완료");
            }
            still_alive.push(session);
        }

        if let Ok(mut g) = SESSION.lock() {
            *g = still_alive;
        }
    }
}
