//! 외부 프로세스 실행 규칙 중 OS마다 다른 것.

use std::process::Command;

/// Windows `CREATE_NO_WINDOW` — 콘솔 프로그램을 창 없이 실행한다.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `kmd-desktop`에 주면 창을 띄우지 않고 옆의 `kmd-daemon start`만 콘솔 없이
/// 실행한 뒤 끝난다. Windows 로그인 자동 시작(HKCU `Run`)이 쓴다 —
/// `kmd-daemon.exe`는 콘솔 서브시스템이라 직접 걸면 콘솔 창이 뜬다.
pub const DESKTOP_START_DAEMON_ARG: &str = "--start-daemon";

/// 기다리지 않을 자식 프로세스를 거둔다 — 끝날 때까지 다른 스레드에서 `wait` 한다.
///
/// Unix 는 부모가 `wait` 하지 않은 자식을 좀비로 남긴다. 몇 주씩 떠 있는 데몬이 런처·알림 창·
/// `open` 을 띄우고 `Child` 를 버리면 좀비가 쌓여 사용자별 프로세스 한도를 깎는다
/// (2026-11-28 실측: 가동 6일 데몬 밑에 11개). Windows 는 핸들만 닫으면 되지만 같게 둔다.
pub fn reap(mut child: std::process::Child) {
    let _ = std::thread::Builder::new()
        .name("reap-child".into())
        .spawn(move || {
            let _ = child.wait();
        });
}

/// `Command`에 "콘솔 창 없이"를 거는 확장.
///
/// Windows에서 GUI·백그라운드 프로세스가 콘솔 프로그램(powershell, taskkill 등)을
/// 실행하면 검은 창이 잠깐 떴다 사라진다. 예전엔 이를 막는 `#[cfg(windows)]`
/// 블록(상수 정의 + `creation_flags`)이 10개 파일 20여 곳에 복붙돼 있었다.
/// 다른 OS에선 아무것도 하지 않으므로 호출부에 cfg 분기가 필요 없다.
/// (`tests/audit_no_window.rs`가 `Command::new` 호출마다 이 처리를 확인한다)
pub trait HideConsole {
    fn hide_console(&mut self) -> &mut Self;
}

impl HideConsole for Command {
    fn hide_console(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}
