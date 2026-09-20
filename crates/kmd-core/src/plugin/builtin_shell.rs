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
        let _ = tx.send(String::from_utf8_lossy(&captured).trim().to_string());
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
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
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

/// Quick action: a pre-defined system info command
struct QuickAction {
    name: &'static str,
    description: &'static str,
    icon: &'static str,
    #[cfg(windows)]
    command: &'static str,
    #[cfg(windows)]
    args: &'static [&'static str],
    #[cfg(not(windows))]
    command: &'static str,
    #[cfg(not(windows))]
    args: &'static [&'static str],
}

/// Pre-defined quick actions for system information
static QUICK_ACTIONS: &[QuickAction] = &[
    QuickAction {
        name: "IP Address",
        description: "Show network IP addresses",
        icon: "\u{1F310}", // 🌐
        #[cfg(windows)]
        command: "powershell",
        #[cfg(windows)]
        args: &["-NoProfile", "-Command",
            "(Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.InterfaceAlias -notmatch 'Loopback' } | Select-Object -ExpandProperty IPAddress) -join \", \""],
        #[cfg(not(windows))]
        command: "sh",
        #[cfg(not(windows))]
        args: &["-c", "ip -4 addr show 2>/dev/null | grep -oP 'inet \\K[\\d.]+' | grep -v '^127\\.' | head -5 || ifconfig 2>/dev/null | grep 'inet ' | grep -v '127.0.0.1' | awk '{print $2}' | head -5 || hostname -I 2>/dev/null"],
    },
    QuickAction {
        name: "Hostname",
        description: "Show computer name",
        icon: "\u{1F4BB}", // 💻
        #[cfg(windows)]
        command: "hostname",
        #[cfg(windows)]
        args: &[],
        #[cfg(not(windows))]
        command: "hostname",
        #[cfg(not(windows))]
        args: &[],
    },
    QuickAction {
        name: "Uptime",
        description: "Show system uptime",
        icon: "\u{23F1}\u{FE0F}", // ⏱️
        #[cfg(windows)]
        command: "powershell",
        #[cfg(windows)]
        args: &["-NoProfile", "-Command",
            "$os = Get-CimInstance Win32_OperatingSystem; $up = (Get-Date) - $os.LastBootUpTime; \"{0}d {1}h {2}m\" -f $up.Days, $up.Hours, $up.Minutes"],
        #[cfg(not(windows))]
        command: "uptime",
        #[cfg(not(windows))]
        args: &["-p"],
    },
    QuickAction {
        name: "Disk Usage",
        description: "Show disk space usage",
        icon: "\u{1F4BE}", // 💾
        #[cfg(windows)]
        command: "powershell",
        #[cfg(windows)]
        args: &["-NoProfile", "-Command",
            "Get-PSDrive -PSProvider FileSystem | ForEach-Object { \"{0}: {1:N1}GB free / {2:N1}GB\" -f $_.Name, ($_.Free/1GB), (($_.Used+$_.Free)/1GB) }"],
        #[cfg(not(windows))]
        command: "df",
        #[cfg(not(windows))]
        args: &["-h", "--total"],
    },
    QuickAction {
        name: "Memory Usage",
        description: "Show memory usage",
        icon: "\u{1F9E0}", // 🧠
        #[cfg(windows)]
        command: "powershell",
        #[cfg(windows)]
        args: &["-NoProfile", "-Command",
            "$os = Get-CimInstance Win32_OperatingSystem; $total = [math]::Round($os.TotalVisibleMemorySize/1MB,1); $free = [math]::Round($os.FreePhysicalMemory/1MB,1); $used = $total - $free; \"Used: {0}GB / Total: {1}GB ({2}%)\" -f $used, $total, [math]::Round($used/$total*100)"],
        #[cfg(not(windows))]
        command: "free",
        #[cfg(not(windows))]
        args: &["-h"],
    },
    QuickAction {
        name: "Public IP",
        description: "Show public IP address",
        icon: "\u{1F30D}", // 🌍
        #[cfg(windows)]
        command: "powershell",
        #[cfg(windows)]
        args: &["-NoProfile", "-Command",
            "(Invoke-WebRequest -Uri 'https://api.ipify.org' -UseBasicParsing -TimeoutSec 5).Content"],
        #[cfg(not(windows))]
        command: "curl",
        #[cfg(not(windows))]
        args: &["-s", "--max-time", "5", "https://api.ipify.org"],
    },
    QuickAction {
        name: "OS Version",
        description: "Show operating system version",
        icon: "\u{2699}\u{FE0F}", // ⚙️
        #[cfg(windows)]
        command: "powershell",
        #[cfg(windows)]
        args: &["-NoProfile", "-Command",
            "(Get-CimInstance Win32_OperatingSystem).Caption + ' ' + (Get-CimInstance Win32_OperatingSystem).Version"],
        #[cfg(not(windows))]
        command: "uname",
        #[cfg(not(windows))]
        args: &["-srm"],
    },
    QuickAction {
        name: "User Name",
        description: "Show current user",
        icon: "\u{1F464}", // 👤
        #[cfg(windows)]
        command: "whoami",
        #[cfg(windows)]
        args: &[],
        #[cfg(not(windows))]
        command: "whoami",
        #[cfg(not(windows))]
        args: &[],
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
#[cfg(target_os = "windows")]
fn hidden_cmd() -> Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new("cmd");
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

#[cfg(not(target_os = "windows"))]
fn hidden_cmd() -> Command {
    Command::new("cmd")
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

        let mut cmd = Command::new(action.command);
        cmd.args(action.args);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
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
        if QUICK_ACTIONS.iter().any(|a| a.name == item.path) {
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
