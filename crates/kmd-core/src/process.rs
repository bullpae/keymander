//! 외부 프로세스 실행 규칙 중 OS마다 다른 것.

use std::process::Command;

/// Windows `CREATE_NO_WINDOW` — 콘솔 프로그램을 창 없이 실행한다.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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
