//! `kmd upgrade` — 실행 중인 데몬·런처를 멈추고 업그레이드한 뒤 데몬을 다시 띄운다.
//!
//! winget 패키지는 `zip` + `portable` 방식이라 설치 전후에 스크립트를 걸 수 없다.
//! 그래서 `winget upgrade`를 그냥 돌리면 실행 중인 exe가 잠겨 교체가 실패하거나,
//! 성공해도 데몬이 꺼진 채로 남는다. 이 명령이 그 앞뒤 절차를 대신한다.
//!
//! 실행 중인 `kmd.exe` 자신도 같은 패키지 폴더에 있어 잠겨 있으므로, winget은
//! 새 콘솔 창의 PowerShell 스크립트가 이 프로세스가 끝난 뒤에 실행한다.

use color_eyre::Result;
#[cfg(windows)]
use kmd_core::ipc;
#[cfg(any(windows, test))]
use std::path::Path;

#[cfg(any(windows, test))]
const WINGET_ID: &str = "bullpae.keymander";

pub fn run() -> Result<()> {
    #[cfg(windows)]
    {
        run_windows()
    }
    #[cfg(not(windows))]
    {
        print_manual_steps();
        Ok(())
    }
}

#[cfg(not(windows))]
fn print_manual_steps() {
    if cfg!(target_os = "macos") {
        println!("Homebrew 설치본은 다음으로 업그레이드하세요:");
        println!("  brew upgrade keymander && kmd daemon restart");
    } else {
        println!("패키지 관리자로 업그레이드한 뒤 데몬을 다시 시작하세요:");
        println!("  sudo apt update && sudo apt upgrade   # 또는 sudo dnf upgrade keymander");
        println!("  kmd daemon restart");
    }
}

#[cfg(windows)]
fn run_windows() -> Result<()> {
    let exe = std::env::current_exe()?;
    let Some(pkg_dir) = exe
        .parent()
        .filter(|_| kmd_core::portable::is_winget_install(&exe))
    else {
        println!("winget으로 설치한 kmd가 아닙니다 ({}).", exe.display());
        println!("포터블 zip이라면 kmd daemon stop 후 새 zip으로 파일을 바꾸고");
        println!("kmd daemon start 로 다시 시작하세요.");
        return Ok(());
    };

    let daemon_was_running = super::daemon::daemon_alive();
    if daemon_was_running {
        println!("데몬을 멈춥니다...");
        super::daemon::send_command(ipc::Request::Shutdown, "stop")?;
    }

    let script = build_script(std::process::id(), pkg_dir, daemon_was_running);
    let script_path = std::env::temp_dir().join("kmd-upgrade.ps1");
    // PowerShell 5.1은 BOM 없는 UTF-8을 ANSI 코드페이지로 읽어 한글이 깨진다
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(script.as_bytes());
    std::fs::write(&script_path, bytes)?;

    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script_path)
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()?;

    println!("새 창에서 업그레이드를 진행합니다 (kmd 파일 잠금을 풀기 위해 이 프로세스는 종료).");
    Ok(())
}

/// PowerShell 작은따옴표 문자열 리터럴
#[cfg(any(windows, test))]
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// 업그레이드 스크립트 — 이 프로세스 종료 대기 → 남은 kmd 프로세스 정리 →
/// winget upgrade → (실행 중이었다면) 새 kmd로 데몬 재시작.
#[cfg(any(windows, test))]
fn build_script(parent_pid: u32, pkg_dir: &Path, restart_daemon: bool) -> String {
    let dir = pkg_dir.to_string_lossy();
    let dir_q = ps_quote(&dir);
    let kmd_q = ps_quote(&format!("{dir}\\kmd.exe"));
    let id = WINGET_ID;
    let restart = if restart_daemon { "$true" } else { "$false" };
    // 0x8A15002B (APPINSTALLER_CLI_ERROR_UPDATE_NOT_APPLICABLE) = 이미 최신
    format!(
        r#"$Host.UI.RawUI.WindowTitle = 'keymander 업그레이드'
Write-Host 'keymander 업그레이드를 시작합니다...'
Wait-Process -Id {parent_pid} -Timeout 30 -ErrorAction SilentlyContinue

# 패키지 폴더의 exe를 잡고 있는 프로세스 정리 (런처, 종료가 늦은 데몬)
$dir = {dir_q}
Get-Process kmd, kmd-desktop, kmd-daemon -ErrorAction SilentlyContinue |
    Where-Object {{ $_.Path -and $_.Path.StartsWith($dir, [StringComparison]::OrdinalIgnoreCase) }} |
    Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500

winget upgrade --id {id} --exact --accept-source-agreements --accept-package-agreements --disable-interactivity
$code = $LASTEXITCODE
Write-Host ''
if ($code -eq 0) {{
    Write-Host '업그레이드 완료' -ForegroundColor Green
}} elseif ($code -eq -1978335189) {{
    Write-Host '이미 최신 버전입니다.' -ForegroundColor Yellow
}} else {{
    Write-Host "업그레이드 실패 (winget 종료 코드 $code)" -ForegroundColor Red
}}

if ({restart}) {{
    Write-Host '데몬을 다시 시작합니다...'
    & {kmd_q} daemon start
}}

Write-Host ''
Read-Host 'Enter 키를 누르면 창을 닫습니다'
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 작은따옴표는_두_번_쓴다() {
        assert_eq!(ps_quote(r"C:\Users\O'Brien"), r"'C:\Users\O''Brien'");
    }

    #[test]
    fn 스크립트는_부모_종료를_기다린_뒤_업그레이드하고_데몬을_되살린다() {
        let dir = Path::new(r"C:\pkg\keymander");
        let s = build_script(4242, dir, true);
        let wait = s.find("Wait-Process -Id 4242").expect("부모 종료 대기");
        let upgrade = s
            .find("winget upgrade --id bullpae.keymander --exact")
            .expect("winget 업그레이드");
        let restart = s
            .find(r"& 'C:\pkg\keymander\kmd.exe' daemon start")
            .expect("새 kmd로 데몬 재시작");
        assert!(
            wait < upgrade && upgrade < restart,
            "순서: 대기 → 업그레이드 → 재시작"
        );
        assert!(s.contains("if ($true)"));
    }

    #[test]
    fn 데몬이_꺼져_있었으면_재시작하지_않는다() {
        let s = build_script(1, Path::new(r"C:\pkg\keymander"), false);
        assert!(s.contains("if ($false)"));
    }
}
