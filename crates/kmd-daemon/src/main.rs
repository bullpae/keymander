//! kmd-daemon — keymander 백그라운드 데몬
//!
//! TCP localhost IPC 서버로 검색 엔진을 메모리에 상주시키고,
//! 글로벌 핫키와 키 바인딩을 관리한다.

mod alarms;
mod autopilot;
mod autostart;
mod clipboard;
mod fs_watch;
mod keybind;
mod location;
mod prewarm;
mod server;
// Windows 붙여넣기 주입/민감정보 정책(순수 로직) — 모든 플랫폼에서 컴파일·
// 테스트되고 실제 사용은 clipboard.rs의 cfg(windows) 경로뿐이라 그 외에선
// dead_code 허용.
#[cfg_attr(not(windows), allow(dead_code))]
mod winclip;

use color_eyre::Result;
use kmd_core::ipc;
use std::env;

/// 로그 파일이 이보다 크면 시작할 때 `.1` 로 밀어낸다 (한 세대만 보관)
const LOG_ROTATE_BYTES: u64 = 5 * 1024 * 1024;

/// `start` 일 때만 로그 파일을 연다 — launchd·작업 스케줄러 아래에서는 stdout 이
/// 버려져 데몬 기록이 남지 않았다 (2026-09 이후 daemon.log 가 멈춰 있었음)
fn open_log_file() -> Option<std::fs::File> {
    let path = ipc::log_file_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok()?;
    }
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_ROTATE_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
}

fn main() -> Result<()> {
    use tracing_subscriber::prelude::*;

    color_eyre::install()?;
    let args: Vec<String> = env::args().collect();
    let command = args.get(1).map(|s| s.as_str()).unwrap_or("start");

    // 파일 쓰기는 백그라운드 스레드로 — 키 훅 스레드가 디스크 I/O 로 멈추지 않게
    let (file_writer, _log_guard) = match (command == "start").then(open_log_file).flatten() {
        Some(file) => {
            let (writer, guard) = tracing_appender::non_blocking(file);
            (Some(writer), Some(guard))
        }
        None => (None, None),
    };
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        // 파일에 쓰는 동안 stdout 은 터미널일 때만 — `kmd daemon start` 는 stdout 을 같은
        // daemon.log 로 돌려 두므로 그대로 두면 모든 줄이 두 번 남는다
        .with(
            (file_writer.is_none() || std::io::IsTerminal::is_terminal(&std::io::stdout()))
                .then(tracing_subscriber::fmt::layer),
        )
        .with(file_writer.map(|w| {
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(w)
        }))
        .init();

    match command {
        "start" => {
            if let Some(detail) = autostart::ensure_default() {
                tracing::info!(
                    "자동 시작을 기본으로 등록했습니다 ({detail}) — 해제: kmd-daemon uninstall"
                );
            }
            server::run()?
        }
        "stop" => send_shutdown()?,
        "status" => send_status()?,
        "install" => cmd_install()?,
        "uninstall" => cmd_uninstall()?,
        other => {
            eprintln!("알 수 없는 명령: {other}");
            eprintln!("사용법: kmd-daemon [start|stop|status|install|uninstall]");
            std::process::exit(1);
        }
    }

    Ok(())
}

fn autostart_label() -> &'static str {
    if autostart::is_installed() {
        "등록됨"
    } else {
        "미등록"
    }
}

fn send_shutdown() -> Result<()> {
    match ipc::send_request_result(&ipc::Request::Shutdown) {
        Ok(ipc::Response::Ok { message }) => println!("{message}"),
        Ok(ipc::Response::Error { message }) => eprintln!("에러: {message}"),
        Ok(other) => println!("응답: {other:?}"),
        Err(e) => eprintln!("{e}"),
    }
    Ok(())
}

fn send_status() -> Result<()> {
    match ipc::send_request_result(&ipc::Request::Status) {
        // 서식은 kmd_core::ipc::Response::status_lines 한 곳에만 둔다
        // (CLI의 `kmd daemon status`와 같은 화면을 보장)
        Ok(resp) => match resp.status_lines(Some(format!("  자동 시작:  {}", autostart_label())))
        {
            Some(lines) => println!("{}", lines.join("\n")),
            None => println!("응답: {resp:?}"),
        },
        Err(ipc::IpcError::Io(_)) => {
            println!("데몬이 실행 중이지 않습니다. (포트 파일은 존재하나 연결 실패)");
            println!("  자동 시작:  {}", autostart_label());
            ipc::cleanup_stale_files();
        }
        Err(_) => {
            println!("데몬이 실행 중이지 않습니다.");
            println!("  자동 시작:  {}", autostart_label());
        }
    }
    Ok(())
}

/// 부팅 시 자동 시작 등록
fn cmd_install() -> Result<()> {
    match autostart::install() {
        Ok(detail) => {
            println!("자동 시작 등록 완료");
            println!("  {detail}");
            println!("  다음 로그인부터 자동으로 데몬이 시작됩니다.");
        }
        Err(e) => eprintln!("자동 시작 등록 실패: {e}"),
    }
    Ok(())
}

/// 부팅 시 자동 시작 해제
fn cmd_uninstall() -> Result<()> {
    match autostart::uninstall() {
        Ok(()) => println!("자동 시작 해제 완료"),
        Err(e) => eprintln!("자동 시작 해제 실패: {e}"),
    }
    Ok(())
}
