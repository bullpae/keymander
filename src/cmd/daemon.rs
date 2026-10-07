//! `kmd daemon` — 백그라운드 데몬 관리 (start/stop/status)

use color_eyre::Result;
use kmd_core::ipc;
use std::net::TcpStream;

pub enum Action {
    Start,
    Stop,
    Restart,
    Status,
    E2e,
    Install,
    Uninstall,
    PasteTest {
        delay_ms: u64,
    },
    ClipTest {
        slot: usize,
        delay_ms: u64,
        to_previous: bool,
    },
    ClipCapture,
}

pub fn run(action: Action) -> Result<()> {
    match action {
        Action::Start => start_daemon(),
        Action::Stop => send_command(ipc::Request::Shutdown, "stop"),
        Action::Restart => restart_daemon(),
        Action::E2e => e2e_selftest(),
        Action::Status => check_status(),
        Action::Install => run_daemon_cmd("install"),
        Action::Uninstall => run_daemon_cmd("uninstall"),
        Action::PasteTest { delay_ms } => paste_test(delay_ms),
        Action::ClipTest {
            slot,
            delay_ms,
            to_previous,
        } => clip_test(slot, delay_ms, to_previous),
        Action::ClipCapture => send_command(ipc::Request::ClipCaptureForeground, "clip-capture"),
    }
}

/// [P1/P2 검증] 지연 후 데몬에 클립보드 슬롯 붙여넣기를 요청한다.
/// `to_previous`면 런처 열기 전 앱으로 포커스를 되돌린 뒤 붙여넣는다(흐름 B).
fn clip_test(slot: usize, delay_ms: u64, to_previous: bool) -> Result<()> {
    let where_ = if to_previous {
        "이전 전경 앱"
    } else {
        "현재 전경 앱"
    };
    println!("{delay_ms}ms 후 슬롯 {slot}을 {where_}에 붙여넣습니다...");
    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    send_command(ipc::Request::ClipPaste { slot, to_previous }, "clip-test")
}

/// [P3 스파이크] 지연 후 데몬에 붙여넣기 주입을 요청한다.
/// 지연 동안 사용자가 붙여넣을 대상 앱으로 포커스를 옮긴다.
fn paste_test(delay_ms: u64) -> Result<()> {
    println!("{delay_ms}ms 후 전경 앱에 붙여넣기를 주입합니다 — 지금 대상 앱으로 전환하세요...");
    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    send_command(ipc::Request::InjectPaste, "paste-test")
}

/// 데몬 로그 파일 경로 (런타임 디렉터리 아래, 시작마다 새로 씀)
fn daemon_log_path() -> std::path::PathBuf {
    ipc::log_file_path()
}

/// 데몬 stdout/stderr 리다이렉트 대상. 로그 파일 생성 실패 시 null 폴백.
/// 이전에는 무조건 null이어서 키맵 파싱 경고 등 진단 정보가 전부 버려졌다.
fn daemon_log_stdio() -> (std::process::Stdio, std::process::Stdio) {
    let path = daemon_log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(file) = std::fs::File::create(&path) {
        if let Ok(clone) = file.try_clone() {
            return (file.into(), clone.into());
        }
    }
    (std::process::Stdio::null(), std::process::Stdio::null())
}

/// 데몬 재시작 — 실행 중이면 정상 종료(IPC Shutdown) 후 다시 시작한다.
///
/// launchd/systemd 밖에서 떠 있던 데몬(stray)도 IPC 종료 경로로 함께 정리되므로,
/// 권한 재부여 후 훅을 되살리는 표준 절차로 이 명령 하나면 된다.
fn restart_daemon() -> Result<()> {
    if daemon_alive() {
        send_command(ipc::Request::Shutdown, "stop")?;
    }
    start_daemon()
}

pub(crate) fn daemon_alive() -> bool {
    matches!(ipc::read_port(), Ok(port) if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok())
}

/// macOS: 자동실행 LaunchAgent가 등록되어 있으면 그 plist 경로.
#[cfg(target_os = "macos")]
fn launchagent_plist() -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(std::env::var_os("HOME")?)
        .join("Library/LaunchAgents/com.keymander.daemon.plist");
    path.exists().then_some(path)
}

/// macOS에서 LaunchAgent 경유로 데몬을 (재)기동한다.
///
/// 반드시 launchd가 데몬을 띄워야 하는 이유: 터미널에서 직접 spawn하면 TCC의
/// 책임 프로세스(responsible process)가 터미널로 귀속되어, 손쉬운 사용을
/// 허용해 두고도 AXIsProcessTrusted=false가 나온다(훅 설치 실패).
/// kickstart가 아닌 bootout→bootstrap을 쓰는 이유: launchd는 서비스에 서명
/// 신원을 고정(pin)하므로 바이너리가 교체된 뒤 kickstart는
/// OS_REASON_CODESIGNING으로 즉사한다. 재등록은 항상 안전하다.
#[cfg(target_os = "macos")]
fn start_via_launchd(plist: &std::path::Path) -> Result<()> {
    let uid_out = std::process::Command::new("id").arg("-u").output()?;
    let uid = String::from_utf8_lossy(&uid_out.stdout).trim().to_string();
    let domain = format!("gui/{uid}");
    let service = format!("{domain}/com.keymander.daemon");

    // 이미 로드돼 있으면 내려서 재등록 (미로드 상태의 실패는 무시)
    let _ = std::process::Command::new("launchctl")
        .args(["bootout", &service])
        .output();
    let out = std::process::Command::new("launchctl")
        .args(["bootstrap", &domain, &plist.to_string_lossy()])
        .output()?;
    if !out.status.success() {
        eprintln!(
            "launchctl bootstrap 실패: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        eprintln!("직접 실행으로 폴백합니다 (훅 권한 귀속이 어긋날 수 있음).");
        return spawn_daemon_process();
    }
    println!("launchd 경유로 데몬을 시작합니다 ({})", plist.display());
    Ok(())
}

fn start_daemon() -> Result<()> {
    if let Ok(port) = ipc::read_port() {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            println!("데몬이 이미 실행 중입니다 (port={port}).");
            return Ok(());
        }
        ipc::cleanup_stale_files();
    }

    // 자동실행이 등록된 환경에서는 서비스 매니저 경유로 기동한다.
    #[cfg(target_os = "macos")]
    if let Some(plist) = launchagent_plist() {
        start_via_launchd(&plist)?;
        return wait_daemon_ready();
    }

    spawn_daemon_process()?;
    wait_daemon_ready()
}

/// 데몬 프로세스를 직접 spawn (서비스 매니저 미등록 환경)
fn spawn_daemon_process() -> Result<()> {
    let daemon_exe = find_sibling_exe("kmd-daemon");
    let (log_out, log_err) = daemon_log_stdio();

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        const DETACHED_PROCESS: u32 = 0x00000008;

        std::process::Command::new(&daemon_exe)
            .arg("start")
            .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
            .stdin(std::process::Stdio::null())
            .stdout(log_out)
            .stderr(log_err)
            .spawn()?;
    }

    #[cfg(not(windows))]
    {
        std::process::Command::new(&daemon_exe)
            .arg("start")
            .stdin(std::process::Stdio::null())
            .stdout(log_out)
            .stderr(log_err)
            .spawn()?;
    }

    Ok(())
}

fn wait_daemon_ready() -> Result<()> {
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if let Ok(port) = ipc::read_port() {
            if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
                println!("데몬 시작 완료 (port={port})");
                return Ok(());
            }
        }
    }

    println!("데몬 시작 중... (포트 파일이 아직 생성되지 않았습니다)");
    Ok(())
}

fn check_status() -> Result<()> {
    match ipc::send_request_result(&ipc::Request::Status) {
        // 서식은 kmd_core::ipc::Response::status_lines 한 곳에만 둔다
        // (데몬 바이너리의 `kmd-daemon status`와 같은 화면을 보장)
        Ok(resp) => match resp.status_lines(Some(format!(
            "  로그:       {}",
            daemon_log_path().display()
        ))) {
            Some(lines) => println!("{}", lines.join("\n")),
            None => println!("예기치 않은 응답"),
        },
        Err(ipc::IpcError::Io(_)) => {
            println!("데몬이 실행 중이지 않습니다. (포트 파일은 존재하나 연결 실패)");
            ipc::cleanup_stale_files();
        }
        Err(_) => {
            println!("데몬이 실행 중이지 않습니다.");
        }
    }
    Ok(())
}

/// 키 주입 셀프테스트 — 데몬이 수 초간 주입·캡처를 수행하므로 대기를 늘린다.
fn e2e_selftest() -> Result<()> {
    println!("키 주입 셀프테스트 실행 중... (수 초 소요, 실행 중 타이핑 금지)");
    send_command_with_timeout(
        ipc::Request::KeybindSelfTest,
        std::time::Duration::from_secs(60),
    )
}

pub(crate) fn send_command(request: ipc::Request, _action_name: &str) -> Result<()> {
    send_command_with_timeout(request, std::time::Duration::from_secs(5))
}

fn send_command_with_timeout(request: ipc::Request, timeout: std::time::Duration) -> Result<()> {
    let is_shutdown = matches!(request, ipc::Request::Shutdown);
    match ipc::send_request_with_timeout(&request, timeout) {
        Ok(ipc::Response::Ok { message }) => {
            println!("{message}");
            if is_shutdown {
                wait_for_daemon_exit();
            }
        }
        Ok(ipc::Response::Error { message }) => eprintln!("에러: {message}"),
        Ok(_) => println!("완료"),
        Err(ipc::IpcError::Io(_)) => {
            // Io 에러 = "요청 중 연결 끊김"일 수도 있다. 데몬이 살아 있는데
            // 포트 파일을 지우면 살아있는 데몬이 유령이 되므로(재발견 불가),
            // 재연결 프로브로 생사를 가른 뒤에만 정리한다.
            if daemon_alive() {
                println!(
                    "요청 처리 중 연결이 끊어졌습니다 — 데몬은 실행 중입니다. 다시 시도하세요."
                );
            } else {
                println!("데몬이 이미 종료되었거나 종료 중입니다.");
                ipc::cleanup_stale_files();
            }
        }
        Err(ipc::IpcError::NoDaemon) => {
            println!("데몬이 실행 중이지 않습니다.");
        }
        Err(e) => eprintln!("{e}"),
    }
    Ok(())
}

/// Shutdown 응답 후 데몬이 실제 종료될 때까지 대기 (최대 3초)
fn wait_for_daemon_exit() {
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        match ipc::read_port() {
            Err(_) => return,
            Ok(port) => {
                if TcpStream::connect(format!("127.0.0.1:{port}")).is_err() {
                    ipc::cleanup_stale_files();
                    return;
                }
            }
        }
    }
    ipc::cleanup_stale_files();
}

/// kmd-desktop 바이너리를 실행
pub fn launch_desktop() -> Result<()> {
    let desktop_exe = find_sibling_exe("kmd-desktop");

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;

        std::process::Command::new(&desktop_exe)
            .creation_flags(DETACHED_PROCESS)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
    }

    #[cfg(not(windows))]
    {
        std::process::Command::new(&desktop_exe)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
    }

    Ok(())
}

fn find_sibling_exe(name: &str) -> std::path::PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default();

    #[cfg(windows)]
    let bin_name = format!("{name}.exe");
    #[cfg(not(windows))]
    let bin_name = name.to_string();

    let same_dir = exe_dir.join(&bin_name);
    if same_dir.exists() {
        return same_dir;
    }

    std::path::PathBuf::from(bin_name)
}

fn run_daemon_cmd(cmd: &str) -> Result<()> {
    let daemon_exe = find_sibling_exe("kmd-daemon");
    let output = std::process::Command::new(&daemon_exe)
        .arg(cmd)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();

    match output {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => {
            std::process::exit(status.code().unwrap_or(1));
        }
        Err(e) => {
            eprintln!("kmd-daemon 실행 실패: {e}");
            eprintln!("경로: {}", daemon_exe.display());
            std::process::exit(1);
        }
    }
}
