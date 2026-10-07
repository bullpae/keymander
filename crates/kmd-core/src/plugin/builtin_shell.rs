//! Built-in shell command extension
//!
//! Activated with `!` prefix — executes shell commands and shows output.
//! Also provides quick system-info actions.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::{Extension, ExtensionAction};
use crate::index::{IndexItem, ItemKind, Source};
use crate::process::HideConsole;

/// 셸 명령 최대 실행 시간 — `!ping -t` 같은 무한 명령이 런처를 멈추지 않도록.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
/// 캡처할 최대 출력 크기 (초과분은 버림)
const MAX_CAPTURE_BYTES: usize = 256 * 1024;

/// 파이프를 끝까지 읽되 MAX_CAPTURE_BYTES까지만 보관하는 리더 스레드.
/// cap 초과분도 계속 읽어 버린다 — 읽기를 멈추면 자식이 파이프 블로킹으로
/// 종료하지 못한다. 결과는 채널로 전달 — join과 달리 recv_timeout이
/// 가능해, 파이프를 물려받은 좀비 손자가 있어도 무한 대기하지 않는다.
fn spawn_capped_reader<R: Read + Send + 'static>(pipe: Option<R>) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut captured = Vec::new();
        if let Some(mut pipe) = pipe {
            let mut buf = [0u8; 8192];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if captured.len() < MAX_CAPTURE_BYTES {
                            let take = n.min(MAX_CAPTURE_BYTES - captured.len());
                            captured.extend_from_slice(&buf[..take]);
                        }
                    }
                }
            }
        }
        // Windows의 콘솔 코드페이지(CP949) 출력도 한글로 읽는다 (textenc 참조)
        let text = crate::textenc::decode_command_output(captured);
        let _ = tx.send(text.trim().to_string());
    });
    rx
}

/// 자식이 자기 프로세스 그룹의 리더가 되도록 설정 (Unix).
/// 타임아웃 시 그룹 전체를 kill 하기 위함 — child.kill()은 sh만 죽이고
/// 손자(sleep 등)는 살아남아 파이프를 계속 쥔다.
#[cfg(unix)]
fn setup_process_group(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        cmd.pre_exec(|| {
            libc::setpgid(0, 0);
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn setup_process_group(_cmd: &mut Command) {}

/// 타임아웃 시 프로세스 트리 전체를 종료.
/// - Windows: taskkill /T /F — cmd.exe만 죽이면 손자가 파이프를 쥐고 남는다
/// - Unix: 프로세스 그룹(-pid) 전체에 SIGKILL
fn kill_process_tree(child: &mut std::process::Child) {
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .hide_console()
            .output();
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// 명령을 타임아웃/출력 상한과 함께 실행하고 (성공 여부, stdout, stderr) 반환
fn run_with_timeout(
    mut cmd: Command,
    timeout: Duration,
) -> Result<(bool, String, String, Option<i32>), String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    setup_process_group(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to execute: {}", e))?;
    let stdout_rx = spawn_capped_reader(child.stdout.take());
    let stderr_rx = spawn_capped_reader(child.stderr.take());

    // 프로세스 종료 후 리더 결과 수거 대기 상한 — 트리 킬을 벗어난
    // (새 세션으로 분리된) 프로세스가 파이프를 쥐고 있어도 여기서 끊는다
    const READER_GRACE: Duration = Duration::from_secs(2);

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    kill_process_tree(&mut child);
                    let partial = stdout_rx.recv_timeout(READER_GRACE).unwrap_or_default();
                    let mut msg = format!("Timed out after {:.1}s", timeout.as_secs_f32());
                    if !partial.is_empty() {
                        msg.push_str(" — partial output:\n");
                        msg.push_str(&partial);
                    }
                    return Err(msg);
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(e) => {
                kill_process_tree(&mut child);
                return Err(format!("Failed to wait: {}", e));
            }
        }
    };

    let stdout = stdout_rx.recv_timeout(READER_GRACE).unwrap_or_default();
    let stderr = stderr_rx.recv_timeout(READER_GRACE).unwrap_or_default();
    Ok((status.success(), stdout, stderr, status.code()))
}

/// Quick action 한 개의 실행 방법.
///
/// 세 OS의 명령을 **모두** 표에 담고 실행 시점에 고른다. 예전에는
/// `cfg(windows)`/`cfg(not(windows))` 두 갈래뿐이라 **macOS가 Linux 명령을
/// 실행했다** — `uptime -p`, `df --total`, `free`, `grep -P`가 macOS에 없어
/// 8개 중 4개가 실패하거나 빈 결과였다. 표가 모든 빌드에 들어가므로 어느
/// OS에서도 컴파일되고, CI의 각 OS가 자기 명령을 실제로 실행해 검증한다.
enum QuickCmd {
    /// 실행 파일 + 인자 그대로
    Exec(&'static str, &'static [&'static str]),
    /// `sh -c <스크립트>` (macOS/Linux)
    Sh(&'static str),
    /// PowerShell 스크립트. 실행 시 출력 인코딩을 UTF-8로 고정한다 — 기본값은
    /// 콘솔 코드페이지(한국어 Windows는 CP949)라 한글이 깨진다.
    Ps(&'static str),
}

/// PowerShell 출력 인코딩 고정 — `apps.rs`·`files.rs`의 PowerShell 호출과 같은 규칙.
const PS_UTF8_PREAMBLE: &str = "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; ";

impl QuickCmd {
    fn to_command(&self) -> Command {
        match self {
            QuickCmd::Exec(program, args) => {
                let mut c = Command::new(program);
                c.args(*args);
                c
            }
            QuickCmd::Sh(script) => {
                let mut c = Command::new("sh");
                c.args(["-c", script]);
                c
            }
            QuickCmd::Ps(script) => {
                let mut c = Command::new("powershell");
                c.args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &format!("{PS_UTF8_PREAMBLE}{script}"),
                ]);
                c
            }
        }
    }
}

/// Quick action: a pre-defined system info command
struct QuickAction {
    name: &'static str,
    description: &'static str,
    icon: &'static str,
    windows: QuickCmd,
    macos: QuickCmd,
    linux: QuickCmd,
}

impl QuickAction {
    fn current(&self) -> &QuickCmd {
        if cfg!(windows) {
            &self.windows
        } else if cfg!(target_os = "macos") {
            &self.macos
        } else {
            &self.linux
        }
    }
}

/// Pre-defined quick actions for system information
static QUICK_ACTIONS: &[QuickAction] = &[
    QuickAction {
        name: "IP Address",
        description: "Show network IP addresses",
        icon: "\u{1F310}", // 🌐
        windows: QuickCmd::Ps(
            "(Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.InterfaceAlias -notmatch 'Loopback' } | Select-Object -ExpandProperty IPAddress) -join ', '",
        ),
        macos: QuickCmd::Sh(
            "ifconfig | awk '/inet / && $2 != \"127.0.0.1\" {print $2}' | head -5",
        ),
        // 예전 스크립트는 `ip`·`grep -P`가 없으면 파이프 끝의 head가 성공으로 끝나
        // `||` 대체가 안 탔다(빈 결과). 명령 존재 여부로 먼저 가른다.
        linux: QuickCmd::Sh(
            "if command -v ip >/dev/null 2>&1; then ip -4 -o addr show scope global | awk '{split($4, a, \"/\"); print a[1]}' | head -5; else hostname -I; fi",
        ),
    },
    QuickAction {
        name: "Hostname",
        description: "Show computer name",
        icon: "\u{1F4BB}", // 💻
        // hostname.exe는 콘솔 코드페이지로 출력한다 — 한글 PC 이름이 깨지지 않게 PowerShell로.
        windows: QuickCmd::Ps("[Environment]::MachineName"),
        macos: QuickCmd::Exec("hostname", &[]),
        linux: QuickCmd::Exec("hostname", &[]),
    },
    QuickAction {
        name: "Uptime",
        description: "Show system uptime",
        icon: "\u{23F1}\u{FE0F}", // ⏱️
        windows: QuickCmd::Ps(
            "$os = Get-CimInstance Win32_OperatingSystem; $up = (Get-Date) - $os.LastBootUpTime; '{0}d {1}h {2}m' -f $up.Days, $up.Hours, $up.Minutes",
        ),
        // macOS uptime엔 -p가 없다. "… up 3 days, 2:01, 2 users, …"에서 기간만 뽑는다.
        macos: QuickCmd::Sh("uptime | sed -E 's/.*up +//; s/, +[0-9]+ users?.*//'"),
        linux: QuickCmd::Exec("uptime", &["-p"]),
    },
    QuickAction {
        name: "Disk Usage",
        description: "Show disk space usage",
        icon: "\u{1F4BE}", // 💾
        windows: QuickCmd::Ps(
            "Get-PSDrive -PSProvider FileSystem | ForEach-Object { '{0}: {1:N1}GB free / {2:N1}GB' -f $_.Name, ($_.Free/1GB), (($_.Used+$_.Free)/1GB) }",
        ),
        // BSD df엔 --total이 없다.
        macos: QuickCmd::Sh("df -h / | awk 'NR==2 {print $4\" free / \"$2}'"),
        linux: QuickCmd::Exec("df", &["-h", "--total"]),
    },
    QuickAction {
        name: "Memory Usage",
        description: "Show memory usage",
        icon: "\u{1F9E0}", // 🧠
        windows: QuickCmd::Ps(
            "$os = Get-CimInstance Win32_OperatingSystem; $total = [math]::Round($os.TotalVisibleMemorySize/1MB,1); $free = [math]::Round($os.FreePhysicalMemory/1MB,1); $used = $total - $free; 'Used: {0}GB / Total: {1}GB ({2}%)' -f $used, $total, [math]::Round($used/$total*100)",
        ),
        // macOS엔 free가 없다. top 1회 샘플의 PhysMem 줄(약 0.7초).
        macos: QuickCmd::Sh("top -l 1 -s 0 | awk '/PhysMem/ {sub(/^PhysMem: /, \"\"); print}'"),
        linux: QuickCmd::Exec("free", &["-h"]),
    },
    QuickAction {
        name: "Public IP",
        description: "Show public IP address",
        icon: "\u{1F30D}", // 🌍
        windows: QuickCmd::Ps(
            "(Invoke-WebRequest -Uri 'https://api.ipify.org' -UseBasicParsing -TimeoutSec 5).Content",
        ),
        macos: QuickCmd::Exec("curl", &["-s", "--max-time", "5", "https://api.ipify.org"]),
        linux: QuickCmd::Exec("curl", &["-s", "--max-time", "5", "https://api.ipify.org"]),
    },
    QuickAction {
        name: "OS Version",
        description: "Show operating system version",
        icon: "\u{2699}\u{FE0F}", // ⚙️
        windows: QuickCmd::Ps(
            "$os = Get-CimInstance Win32_OperatingSystem; $os.Caption + ' ' + $os.Version",
        ),
        // uname -srm은 커널 버전(Darwin 25.x)이라 사용자가 아는 macOS 버전이 아니다.
        macos: QuickCmd::Sh("echo \"macOS $(sw_vers -productVersion) ($(uname -m))\""),
        linux: QuickCmd::Exec("uname", &["-srm"]),
    },
    QuickAction {
        name: "User Name",
        description: "Show current user",
        icon: "\u{1F464}", // 👤
        // whoami.exe도 콘솔 코드페이지 — 한글 사용자명이 깨진다.
        windows: QuickCmd::Ps("$env:USERDOMAIN + '\\' + $env:USERNAME"),
        macos: QuickCmd::Exec("whoami", &[]),
        linux: QuickCmd::Exec("whoami", &[]),
    },
];

/// 사용자 셸 명령을 새 터미널 창에서 실행한다 (결과가 화면에 유지됨).
///
/// TUI와 데스크톱이 공유하는 단일 구현:
/// - Windows: `cmd /k` 새 콘솔 — `raw_arg`로 명령줄을 그대로 전달한다.
///   `args()`는 내부 따옴표를 `\"`로 이스케이프하는데 cmd.exe는 그 문법을
///   모르므로 따옴표 포함 명령이 깨진다.
/// - macOS: 임시 `.command` 스크립트를 `open -a Terminal`로 연다 —
///   osascript(AppleEvent)와 달리 자동화(TCC) 권한이 필요 없어
///   번들이 아닌 단독 바이너리에서도 동작한다.
/// - Linux: 사용 가능한 첫 터미널 에뮬레이터로 실행
pub fn launch_in_terminal(cmd_line: &str) -> Result<(), String> {
    let cmd_line = cmd_line.trim();
    if cmd_line.is_empty() {
        return Err("Empty command".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        Command::new("cmd")
            .raw_arg("/k")
            .raw_arg(cmd_line)
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("터미널 실행 실패: {e}"))
    }

    #[cfg(target_os = "macos")]
    {
        let path =
            write_command_script(cmd_line).map_err(|e| format!("임시 스크립트 생성 실패: {e}"))?;
        Command::new("open")
            .args(["-a", "Terminal"])
            .arg(&path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("터미널 실행 실패: {e}"))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let escaped = cmd_line.replace('\'', "'\\''");
        for term in ["x-terminal-emulator", "gnome-terminal", "xterm"] {
            if Command::new(term)
                .args([
                    "-e",
                    &format!("sh -c '{escaped} ; read -p \"Press Enter...\"'"),
                ])
                .spawn()
                .is_ok()
            {
                return Ok(());
            }
        }
        Err("사용 가능한 터미널 에뮬레이터를 찾지 못했습니다".to_string())
    }
}

/// macOS: 명령을 담은 실행 가능한 `.command` 파일을 임시 디렉터리에 만든다.
/// 스크립트는 실행 직후 자신을 삭제하고(셸이 fd를 쥐고 있어 안전),
/// 종료 후 Enter 대기로 결과가 화면에 남는다.
#[cfg(target_os = "macos")]
fn write_command_script(cmd_line: &str) -> std::io::Result<std::path::PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("kmd-shell-{nanos}.command"));
    let script = format!(
        "#!/bin/sh\nrm -f \"$0\"\n{cmd_line}\nstatus=$?\n\
         printf '\\n[kmd] exit %s — Enter를 누르면 닫힙니다\\n' \"$status\"\n\
         read _\nexit $status\n"
    );
    let mut file = std::fs::File::create(&path)?;
    file.write_all(script.as_bytes())?;
    file.set_permissions(std::fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

/// Windows에서 콘솔 창 없이 cmd 실행하는 헬퍼
fn hidden_cmd() -> Command {
    let mut cmd = Command::new("cmd");
    cmd.hide_console();
    cmd
}

pub struct ShellExtension;

impl ShellExtension {
    /// Execute a shell command and capture its output
    pub fn execute_command(cmd_line: &str) -> Result<String, String> {
        let cmd_line = cmd_line.trim();
        if cmd_line.is_empty() {
            return Err("Empty command".to_string());
        }

        let cmd = if cfg!(target_os = "windows") {
            let mut c = hidden_cmd();
            c.args(["/c", cmd_line]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", cmd_line]);
            c
        };

        let (success, stdout, stderr, code) = run_with_timeout(cmd, COMMAND_TIMEOUT)?;

        if success {
            if stdout.is_empty() {
                Ok("(no output)".to_string())
            } else {
                Ok(stdout)
            }
        } else {
            let msg = if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                format!("Exit code: {}", code.unwrap_or(-1))
            };
            Err(msg)
        }
    }

    /// Quick action 이름인지 확인
    pub fn is_quick_action(name: &str) -> bool {
        QUICK_ACTIONS
            .iter()
            .any(|a| a.name.eq_ignore_ascii_case(name))
    }

    /// Execute a quick action by name
    pub fn execute_quick_action(name: &str) -> Result<String, String> {
        let action = QUICK_ACTIONS
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("Unknown quick action: {}", name))?;

        let mut cmd = action.current().to_command();
        cmd.hide_console();
        let (success, stdout, stderr, code) = run_with_timeout(cmd, COMMAND_TIMEOUT)?;

        // 비정상 종료를 성공으로 넘기지 않는다 — 예전에는 종료 코드와 stderr를
        // 버려서, 명령이 실패해도 빈 출력이 "(no output)"으로 표시됐다.
        if !success {
            let detail = if !stderr.trim().is_empty() {
                stderr.trim().to_string()
            } else if !stdout.trim().is_empty() {
                stdout.trim().to_string()
            } else {
                "출력 없음".to_string()
            };
            let where_ = match code {
                Some(c) => format!("종료 코드 {c}"),
                None => "신호로 종료됨".to_string(),
            };
            return Err(format!("{} 실패 ({where_}): {detail}", action.name));
        }

        if stdout.is_empty() {
            Ok("(no output)".to_string())
        } else {
            Ok(stdout)
        }
    }

    /// List quick actions, optionally filtered
    fn list_quick_actions(filter: &str) -> Vec<IndexItem> {
        let filter_lower = filter.to_lowercase();
        QUICK_ACTIONS
            .iter()
            .filter(|a| {
                filter.is_empty()
                    || a.name.to_lowercase().contains(&filter_lower)
                    || a.description.to_lowercase().contains(&filter_lower)
            })
            .map(|a| IndexItem {
                name: format!("{} {}", a.icon, a.name),
                path: a.name.to_string(), // used as key for execute_quick_action
                kind: ItemKind::Shell,
                source: Source::Plugin,
                icon: a.icon.to_string(),
                keywords: a.description.to_string(),
                icon_path: None,
            })
            .collect()
    }
}

impl Extension for ShellExtension {
    fn name(&self) -> &str {
        "Shell"
    }

    fn prefix(&self) -> Option<&str> {
        Some("!")
    }

    fn search(&self, query: &str) -> Vec<IndexItem> {
        let query = query.trim();

        if query.is_empty() {
            // Show quick actions when just "!" is typed
            return Self::list_quick_actions("");
        }

        // Check if it's a quick-action filter
        let mut results = Self::list_quick_actions(query);

        // Also show the raw command as an option to execute
        results.push(IndexItem {
            name: format!("\u{1F4DF} Run: {}", query), // 📟
            path: query.to_string(),
            kind: ItemKind::Shell,
            source: Source::Plugin,
            icon: "\u{1F4DF}".to_string(), // 📟
            keywords: format!("shell execute run command {}", query),
            icon_path: None,
        });

        results
    }

    fn execute(&self, item: &IndexItem) -> ExtensionAction {
        // If path matches a quick action name, execute it
        // is_quick_action과 같은 규칙(대소문자 무시)으로 가른다 — 예전엔 여기만
        // 정확히 일치를 봐서, 이름이 조금만 달라도 quick action 이름을 셸 명령으로
        // 실행하려 들었다.
        if Self::is_quick_action(&item.path) {
            match Self::execute_quick_action(&item.path) {
                Ok(output) => ExtensionAction::CopyToClipboard(output),
                Err(e) => ExtensionAction::Display(format!("Error: {}", e)),
            }
        } else {
            // Execute as raw shell command
            match Self::execute_command(&item.path) {
                Ok(output) => ExtensionAction::CopyToClipboard(output),
                Err(e) => ExtensionAction::Display(format!("Error: {}", e)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 실패를 성공으로 보고하지 않는다 (REF-02) ──────────────────────
    //
    // 예전에는 run_with_timeout의 종료 코드·stderr를 버려서, 명령이 실패해도
    // 빈 출력이 "(no output)"으로 표시됐다.

    #[test]
    fn 비정상_종료는_오류로_전달된다() {
        // 반드시 실패하는 명령을 직접 돌려 동일한 판정 로직을 확인한다
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo to-stderr >&2; exit 3"]);
        let (success, _stdout, stderr, code) =
            run_with_timeout(cmd, COMMAND_TIMEOUT).expect("실행 자체는 된다");
        assert!(!success, "exit 3은 실패로 판정되어야 한다");
        assert_eq!(code, Some(3));
        assert!(stderr.contains("to-stderr"), "stderr가 보존되어야 한다");
    }

    #[test]
    fn 알_수_없는_quick_action은_오류() {
        let err = ShellExtension::execute_quick_action("no-such-action").unwrap_err();
        assert!(err.contains("Unknown quick action"), "{err}");
    }

    /// 이 OS의 quick action을 **실제로 실행**해 본다. CI가 Windows·macOS·Linux에서
    /// 돌므로 세 OS의 명령이 각자 검증된다 — 예전에 macOS가 Linux 명령(`uptime -p`,
    /// `df --total`, `free`)을 실행해 8개 중 4개가 실패한 걸 아무 테스트도 못 잡았다.
    /// 네트워크가 필요한 Public IP만 뺀다.
    #[test]
    fn 이_os의_quick_action은_모두_성공하고_내용이_있다() {
        for action in QUICK_ACTIONS.iter().filter(|a| a.name != "Public IP") {
            let out = ShellExtension::execute_quick_action(action.name)
                .unwrap_or_else(|e| panic!("{} 실패: {e}", action.name));
            assert!(
                !out.trim().is_empty() && out != "(no output)",
                "{}: 출력이 비었다 — 명령은 성공했지만 결과를 못 뽑았다",
                action.name
            );
            assert!(
                !out.contains('\u{FFFD}'),
                "{}: 깨진 문자 — 출력 인코딩 문제: {out}",
                action.name
            );
        }
    }

    #[test]
    fn quick_action_이름은_대소문자_무관하게_실행된다() {
        // execute()가 is_quick_action과 다른 규칙을 쓰면 이름을 셸 명령으로 실행하려 든다.
        let item = IndexItem {
            name: String::new(),
            path: "hostname".into(), // 표의 이름은 "Hostname"
            kind: ItemKind::Shell,
            source: Source::Plugin,
            icon: String::new(),
            keywords: String::new(),
            icon_path: None,
        };
        assert!(matches!(
            ShellExtension.execute(&item),
            ExtensionAction::CopyToClipboard(_)
        ));
    }

    #[test]
    fn test_quick_actions_list() {
        let actions = ShellExtension::list_quick_actions("");
        assert!(
            actions.len() >= 5,
            "Expected at least 5 quick actions, got {}",
            actions.len()
        );
    }

    #[test]
    fn test_quick_actions_filter() {
        let actions = ShellExtension::list_quick_actions("ip");
        assert!(
            !actions.is_empty(),
            "Expected at least 1 result for 'ip' filter"
        );
    }

    #[test]
    fn test_search_empty_shows_quick_actions() {
        let ext = ShellExtension;
        let results = ext.search("");
        assert!(results.len() >= 5);
    }

    #[test]
    fn test_search_command_appends_run() {
        let ext = ShellExtension;
        let results = ext.search("echo hello");
        // Last result should be "Run: echo hello"
        let last = results.last().unwrap();
        assert!(last.name.contains("Run:"));
        assert_eq!(last.path, "echo hello");
    }

    #[test]
    fn test_execute_simple_command() {
        let result = ShellExtension::execute_command("echo test123");
        assert!(result.is_ok());
        assert!(result.unwrap().contains("test123"));
    }

    #[test]
    fn test_execute_hostname() {
        let result = ShellExtension::execute_quick_action("Hostname");
        assert!(result.is_ok());
        assert!(!result.unwrap().is_empty());
    }

    #[test]
    fn test_run_with_timeout_kills_hanging_command() {
        // 종료되지 않는 명령이 타임아웃으로 중단되는지 확인
        let cmd = if cfg!(target_os = "windows") {
            let mut c = hidden_cmd();
            c.args(["/c", "ping -n 60 127.0.0.1"]);
            c
        } else {
            let mut c = Command::new("sh");
            // "; true"로 sh의 exec 최적화를 막아 sleep이 손자 프로세스가
            // 되게 한다 — 그룹 킬 없이는 sleep이 파이프를 쥐고 살아남는
            // 시나리오(CI 회귀)를 확실히 재현
            c.args(["-c", "sleep 60; true"]);
            c
        };

        let start = Instant::now();
        let result = run_with_timeout(cmd, Duration::from_millis(500));
        let elapsed = start.elapsed();

        assert!(result.is_err(), "타임아웃 시 Err 반환");
        assert!(
            result.unwrap_err().contains("Timed out"),
            "타임아웃 메시지 포함"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "타임아웃(0.5s) 부근에서 반환되어야 함: {:?}",
            elapsed
        );
    }

    /// 실제 터미널 창을 여는 수동 검증용 —
    /// `cargo test -p kmd-core manual_launch -- --ignored`
    #[test]
    #[ignore = "실제 터미널 창을 연다 — 수동 실행 전용"]
    fn manual_launch_in_terminal() {
        launch_in_terminal("echo kmd-terminal-test").expect("터미널 실행 실패");
    }

    #[test]
    fn test_run_with_timeout_normal_completion() {
        let cmd = if cfg!(target_os = "windows") {
            let mut c = hidden_cmd();
            c.args(["/c", "echo done123"]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", "echo done123"]);
            c
        };

        let (success, stdout, _stderr, _code) =
            run_with_timeout(cmd, Duration::from_secs(10)).unwrap();
        assert!(success);
        assert!(stdout.contains("done123"));
    }
}
