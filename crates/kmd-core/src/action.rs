//! Action execution — launch programs, open files, open URLs

use std::process::Command;

use crate::process::HideConsole;

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
        // Linux 앱 항목의 path는 파일이 아니라 .desktop의 Exec **명령줄**이다
        // (index/apps.rs). 예전엔 다른 OS처럼 `xdg-open <명령줄>`로 열려다 실패했다 —
        // xdg-open은 파일·URL을 여는 도구라 `firefox --new-window`를 실행하지 못한다.
        #[cfg(target_os = "linux")]
        ItemKind::App => match linux_desktop_exec(&result.item.path).spawn() {
            Ok(_) => ActionResult::Launched,
            Err(e) => {
                ActionResult::Error(format!("Failed to launch '{}': {}", result.item.name, e))
            }
        },
        // App은 OS별로 갈라 둔다 — Linux에서 위 분기와 한 줄로 묶으면 App이 도달
        // 불가 패턴이 된다(macOS 빌드에선 위 분기가 없어 보이지 않던 경고).
        #[cfg(not(target_os = "linux"))]
        ItemKind::App => open_with_system(&result.item.path),
        ItemKind::Executable | ItemKind::File | ItemKind::Directory => {
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
        // 주소 전체에는 검색어·번역 원문·프롬프트가 들어 있고, 이 문구는 로그로 간다
        Err(e) => ActionResult::Error(format!("Failed to open URL ({}): {}", url_host(url), e)),
    }
}

/// 주소의 `스킴://호스트` 까지만 — 경로·질의는 뺀다. 주소가 아니면 `"?"`
pub fn url_host(url: &str) -> &str {
    let Some(scheme_end) = url.find("://") else {
        return "?";
    };
    let rest = &url[scheme_end + 3..];
    let host_len = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    &url[..scheme_end + 3 + host_len]
}

/// `url` 이 `https://` 로 시작하는지 (대소문자 무시, 앞 공백 무시) — 데몬이 브라우저로만 여는 주소의 기준
pub fn is_https_url(url: &str) -> bool {
    has_scheme(url.trim_start(), "https://")
}

fn has_scheme(url: &str, scheme: &str) -> bool {
    url.get(..scheme.len())
        .is_some_and(|s| s.eq_ignore_ascii_case(scheme))
}

/// Spawn the platform-specific "open" command for a path or URL
fn spawn_open(target: &str) -> std::io::Result<()> {
    spawn_open_child(target).map(crate::process::reap)
}

fn spawn_open_child(target: &str) -> std::io::Result<std::process::Child> {
    #[cfg(target_os = "windows")]
    {
        let t = target.trim();
        // explorer.exe + URL 은 탐색기만 뜨는 환경이 있어, http(s)는 기본 브라우저 핸들러로 연다.
        // 스킴은 대소문자를 가리지 않는다 — 데몬의 https 판정(is_https_url)과 같아야 `HTTPS://` 가
        // 파일 열기(explorer) 경로로 새지 않는다
        if has_scheme(t, "http://") || has_scheme(t, "https://") {
            Command::new("rundll32")
                .arg("url.dll,FileProtocolHandler")
                .arg(t)
                .hide_console()
                .spawn()
        } else {
            Command::new("explorer.exe")
                .arg(&*explorer_path(t))
                .hide_console()
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

/// Linux .desktop의 Exec 명령줄을 실행할 명령. `%f` 같은 필드 코드는 색인 단계에서
/// 이미 뺐다(index/apps.rs). 명령줄의 따옴표는 셸이 해석한다. 출력은 버린다 —
/// TUI 터미널에 앱 로그가 쏟아지지 않게.
#[cfg(any(target_os = "linux", test))]
fn linux_desktop_exec(exec: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c")
        .arg(exec)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    c
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
    command.hide_console();

    match command.spawn() {
        Ok(_) => ActionResult::Launched,
        Err(e) => ActionResult::Error(format!("Failed to execute '{}': {}", cmd.display_name, e)),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn 주소는_호스트까지만_남긴다() {
        assert_eq!(
            super::url_host("https://translate.google.com/?sl=auto&text=비밀"),
            "https://translate.google.com"
        );
        assert_eq!(
            super::url_host("https://chatgpt.com?q=x"),
            "https://chatgpt.com"
        );
        assert_eq!(
            super::url_host("https://example.com"),
            "https://example.com"
        );
        assert_eq!(super::url_host("/Users/me/file.txt"), "?");
    }

    use super::{explorer_path, linux_desktop_exec};

    #[test]
    fn linux_앱은_명령줄을_셸로_실행한다() {
        // xdg-open이 아니라 sh -c — Exec 명령줄은 파일이 아니다
        let c = linux_desktop_exec("firefox --new-window");
        assert_eq!(c.get_program(), "sh");
        let args: Vec<_> = c.get_args().collect();
        assert_eq!(args, ["-c", "firefox --new-window"]);
    }

    /// 실제로 실행되는지 — 셸이 있는 OS에서만 (명령은 아무 부작용 없는 true)
    #[cfg(unix)]
    #[test]
    fn linux_앱_명령줄은_실제로_실행된다() {
        let status = linux_desktop_exec("true").status().unwrap();
        assert!(status.success());
    }

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
