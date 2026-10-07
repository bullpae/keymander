//! Action execution — launch programs, open files, open URLs

use std::process::Command;

use crate::index::{system_commands, ItemKind};
use crate::search::SearchResult;

/// The result of executing an action
#[derive(Debug)]
pub enum ActionResult {
    /// Successfully launched
    Launched,
    /// Opened URL in browser
    OpenedUrl(String),
    /// Requires user confirmation before executing
    NeedsConfirmation(String),
    /// Error
    Error(String),
}

/// Execute the action for a search result
pub fn execute(result: &SearchResult) -> ActionResult {
    match result.item.kind {
        ItemKind::App | ItemKind::Executable | ItemKind::File | ItemKind::Directory => {
            open_with_system(&result.item.path)
        }
        ItemKind::SystemCommand => execute_system_command(&result.item.name),
        ItemKind::WebSearch => {
            let url_from_keywords = result
                .item
                .keywords
                .split_whitespace()
                .find(|s| s.starts_with("http://") || s.starts_with("https://"));
            if let Some(url) = url_from_keywords {
                return open_url(url);
            }
            let path = result.item.path.trim();
            if path.starts_with("http://") || path.starts_with("https://") {
                open_url(path)
            } else {
                ActionResult::Error(format!(
                    "웹 항목에 열 수 있는 URL이 없습니다: {}",
                    result.item.name
                ))
            }
        }
        ItemKind::Calculator => {
            // Calculator results are handled by the TUI (clipboard copy)
            ActionResult::Launched
        }
        ItemKind::Emoji => {
            // Emoji results are handled by the TUI (clipboard copy)
            ActionResult::Launched
        }
        ItemKind::Shell => {
            // Shell commands are handled by the TUI (execute + show output)
            ActionResult::Launched
        }
    }
}

/// Open a file/app using the system's default handler
pub fn open_with_system(path: &str) -> ActionResult {
    match spawn_open(path) {
        Ok(_) => ActionResult::Launched,
        Err(e) => ActionResult::Error(format!("Failed to open '{}': {}", path, e)),
    }
}

/// Open a URL in the default browser
pub fn open_url(url: &str) -> ActionResult {
    match spawn_open(url) {
        Ok(_) => ActionResult::OpenedUrl(url.to_string()),
        Err(e) => ActionResult::Error(format!("Failed to open URL '{}': {}", url, e)),
    }
}

/// Spawn the platform-specific "open" command for a path or URL
fn spawn_open(target: &str) -> std::io::Result<std::process::Child> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        let t = target.trim();
        // explorer.exe + URL 은 탐색기만 뜨는 환경이 있어, http(s)는 기본 브라우저 핸들러로 연다.
        if t.starts_with("http://") || t.starts_with("https://") {
            Command::new("rundll32")
                .arg("url.dll,FileProtocolHandler")
                .arg(t)
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
        } else {
            Command::new("explorer.exe")
                .arg(&*explorer_path(t))
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
        }
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(target).spawn()
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Command::new("xdg-open").arg(target).spawn()
    }
}

/// 탐색기에 넘길 경로 — 로컬 파일 경로면 `/`를 `\`로 바꾼다.
///
/// `explorer.exe`는 `C:\work/sub`처럼 `/`가 섞인 경로를 알아보지 못하고
/// **조용히 기본 폴더(문서/내 PC)를 연다.** 실패가 아니라 엉뚱한 창이 떠서
/// 원인을 찾기 어렵다. config의 `search_paths = ["C:/work"]`처럼 `/`로 적은
/// 경로, `~/` 확장 결과 등 섞인 경로가 들어올 길이 여럿이라 여는 쪽에서 막는다.
///
/// 드라이브 상대 경로(`d:`, `d:My Data`)도 같은 증상이다 — "D의 현재 폴더"
/// 기준이라 탐색기가 알아보지 못한다. 루트 기준(`d:\`, `d:\My Data`)으로 본다.
///
/// 드라이브 문자(`C:`)나 UNC(`\\`, `//`)로 시작할 때만 바꾼다 —
/// `ms-settings:` 같은 URI(스킴이 두 글자 이상)나 셸 명령에는 손대지 않는다.
#[cfg_attr(not(windows), allow(dead_code))]
fn explorer_path(target: &str) -> std::borrow::Cow<'_, str> {
    let b = target.as_bytes();
    let has_drive = b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
    let drive_relative = has_drive && !matches!(b.get(2), Some(b'/' | b'\\'));
    let unc = target.starts_with(r"\\") || target.starts_with("//");

    if drive_relative {
        let rest = target[2..].replace('/', r"\");
        std::borrow::Cow::Owned(format!(r"{}\{rest}", &target[..2]))
    } else if (has_drive || unc) && target.contains('/') {
        std::borrow::Cow::Owned(target.replace('/', r"\"))
    } else {
        std::borrow::Cow::Borrowed(target)
    }
}

/// Execute a system command
fn execute_system_command(display_name: &str) -> ActionResult {
    let Some(cmd) = system_commands::find_by_display_name(display_name) else {
        return ActionResult::Error(format!("Unknown system command: {}", display_name));
    };

    if cmd.confirm {
        return ActionResult::NeedsConfirmation(display_name.to_string());
    }

    do_execute_system_command(cmd)
}

/// Actually run a system command (after confirmation if needed)
pub fn do_execute_system_command(cmd: &system_commands::SystemCommand) -> ActionResult {
    let mut command = Command::new(cmd.command);
    command.args(cmd.args);

    // Hide the console window on Windows so it doesn't flash.
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    match command.spawn() {
        Ok(_) => ActionResult::Launched,
        Err(e) => ActionResult::Error(format!("Failed to execute '{}': {}", cmd.display_name, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::explorer_path;

    #[test]
    fn 탐색기_경로는_섞인_구분자를_역슬래시로_통일한다() {
        // 섞인 경로를 그대로 넘기면 explorer.exe는 기본 폴더를 연다.
        assert_eq!(
            explorer_path(r"C:\Users\me/Documents\a.txt"),
            r"C:\Users\me\Documents\a.txt"
        );
        assert_eq!(explorer_path("C:/work/sub"), r"C:\work\sub");
        assert_eq!(explorer_path("d:/x"), r"d:\x");
        assert_eq!(explorer_path(r"\\server\share/dir"), r"\\server\share\dir");
        assert_eq!(explorer_path("//server/share"), r"\\server\share");
    }

    #[test]
    fn 탐색기_경로_이미_정상이면_그대로() {
        assert!(matches!(
            explorer_path(r"C:\work\sub"),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn 파일경로가_아니면_건드리지_않는다() {
        // URI·설정 스킴·상대 경로는 대상이 아니다.
        assert_eq!(explorer_path("ms-settings:display"), "ms-settings:display");
        assert_eq!(
            explorer_path("shell:::{20D04FE0}/x"),
            "shell:::{20D04FE0}/x"
        );
        assert_eq!(explorer_path("foo/bar"), "foo/bar");
    }

    #[test]
    fn 드라이브_상대_경로는_루트_기준으로() {
        // `d:My Data`는 "D의 현재 폴더" 기준이라 탐색기가 문서 폴더를 열었다
        assert_eq!(explorer_path("C:"), r"C:\");
        assert_eq!(explorer_path("d:My Data"), r"d:\My Data");
        assert_eq!(explorer_path("d:My Data/sub"), r"d:\My Data\sub");
    }
}
