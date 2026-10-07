//! 데몬 자동 시작 등록/해제
//!
//! Windows: Startup 폴더에 VBS 스크립트 (콘솔 창 없이 백그라운드 실행)
//! macOS:   ~/Library/LaunchAgents/com.keymander.daemon.plist
//! Linux:   ~/.config/systemd/user/kmd-daemon.service

use std::path::{Path, PathBuf};

/// 사용자가 자동 시작을 한 번이라도 켜거나 끈 적이 있다는 표시.
/// 이 파일이 있으면 [`ensure_default`]가 더 이상 손대지 않는다.
const DECIDED_MARKER: &str = "autostart.decided";

/// 현재 실행 파일 경로로 자동 시작 등록
pub fn install() -> Result<String, String> {
    let detail = platform::install()?;
    mark_decided();
    Ok(detail)
}

/// 자동 시작 해제
pub fn uninstall() -> Result<(), String> {
    platform::uninstall()?;
    // 해제도 사용자의 결정이다 — 다음 데몬 시작 때 다시 켜지 않도록 남긴다.
    mark_decided();
    Ok(())
}

/// 자동 시작이 등록되어 있는지 확인
pub fn is_installed() -> bool {
    platform::is_installed()
}

/// 데몬 시작 시 호출 — 아직 아무도 결정하지 않았으면 자동 시작을 켠다.
///
/// 예전에는 opt-in(`kmd-daemon install`)이라 대부분 모른 채 지나갔고, 재부팅하면
/// 단축키가 죽어 있었다. 이제 첫 시작에 한 번 등록하고, 이후 사용자가 끄면
/// 마커가 남아 다시 켜지 않는다.
///
/// 건너뛰는 경우:
/// - USB용 포터블 — 시스템에 흔적을 남기지 않는 것이 포터블의 약속.
///   winget 설치본도 `kmd-data\`가 있어 포터블로 판정되지만 설치된 앱이므로 등록한다.
/// - 개발 빌드(`target/debug|release`) — 빌드 산출물 경로를 등록하면 안 됨
/// - macOS — 이미 떠 있는 데몬 옆에 `launchctl bootstrap`(RunAtLoad)이 하나를
///   더 띄우고, brew 설치 경로는 버전마다 바뀐다. 기존처럼 직접 등록한다.
pub fn ensure_default() -> Option<String> {
    if cfg!(target_os = "macos") || is_usb_portable() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    if is_dev_build(&exe) || marker_path().exists() {
        return None;
    }
    if is_installed() {
        mark_decided();
        return None;
    }
    match install() {
        Ok(detail) => Some(detail),
        Err(e) => {
            // 실패해도 매 시작마다 재시도하지 않는다 — 수동 등록은 여전히 가능
            mark_decided();
            eprintln!("자동 시작 기본 등록 실패: {e}");
            None
        }
    }
}

fn is_usb_portable() -> bool {
    kmd_core::portable::is_portable() && !kmd_core::portable::running_from_winget()
}

/// 마커는 포터블 여부와 무관하게 OS 표준 위치에 둔다 — winget 설치본의
/// `kmd-data\`(패키지 폴더 안)에 두면 업그레이드로 지워졌을 때 사용자가 꺼 둔
/// 자동 시작을 다시 켜게 된다.
fn marker_path() -> PathBuf {
    kmd_core::Config::standard_data_dir().join(DECIDED_MARKER)
}

fn mark_decided() {
    if is_usb_portable() {
        return; // 호스트 PC에 흔적을 남기지 않는다
    }
    let path = marker_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, "");
}

/// cargo 빌드 산출물(`.../target/debug/kmd-daemon`)인가
fn is_dev_build(exe: &Path) -> bool {
    let parts: Vec<String> = exe
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    parts
        .windows(2)
        .any(|w| w[0] == "target" && (w[1] == "debug" || w[1] == "release"))
}

/// systemd 유닛의 `ExecStart` 인자 하나를 따옴표로 감싼다.
///
/// 예전엔 `ExecStart={exe} start`로 그대로 써서, 경로에 공백이 있으면
/// (`~/My Apps/kmd-daemon`) systemd가 인자를 쪼개 자동 시작이 실패했다. Windows
/// VBS 경로는 이미 따옴표를 처리하고 있었다(같은 일, Linux만 빠짐).
/// systemd는 큰따옴표 안에서 `\`와 `"`를 역슬래시로 이스케이프한다.
#[cfg(any(target_os = "linux", test))]
fn systemd_quote(arg: &str) -> String {
    let escaped = arg.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_경로는_따옴표로_감싼다() {
        assert_eq!(
            systemd_quote("/home/u/My Apps/kmd-daemon"),
            r#""/home/u/My Apps/kmd-daemon""#
        );
        assert_eq!(systemd_quote(r#"/a"b\c"#), r#""/a\"b\\c""#);
    }

    // `\` 는 Windows에서만 경로 구분자라 OS별 경로로 검사한다
    #[cfg(windows)]
    #[test]
    fn 개발_빌드_경로는_자동_등록하지_않는다() {
        assert!(is_dev_build(Path::new(
            r"O:\repo\keymander-cli\target\debug\kmd-daemon.exe"
        )));
        assert!(!is_dev_build(Path::new(
            r"C:\Users\u\AppData\Local\Microsoft\WinGet\Packages\bullpae.keymander_x\keymander\kmd-daemon.exe"
        )));
    }

    #[cfg(not(windows))]
    #[test]
    fn 개발_빌드_경로는_자동_등록하지_않는다() {
        assert!(is_dev_build(Path::new(
            "/home/u/keymander/target/release/kmd-daemon"
        )));
        assert!(!is_dev_build(Path::new("/usr/bin/kmd-daemon")));
    }
}

// ── Windows ─────────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod platform {
    use std::path::PathBuf;

    const SCRIPT_NAME: &str = "kmd-daemon.vbs";

    fn startup_dir() -> PathBuf {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| {
            std::env::var("USERPROFILE")
                .map(|p| format!("{p}\\AppData\\Roaming"))
                .unwrap_or_else(|_| "C:\\Users\\Default\\AppData\\Roaming".into())
        });
        PathBuf::from(appdata).join("Microsoft\\Windows\\Start Menu\\Programs\\Startup")
    }

    pub fn install() -> Result<String, String> {
        let exe = std::env::current_exe().map_err(|e| format!("실행 파일 경로 확인 실패: {e}"))?;
        let vbs = format!(
            "Set s = CreateObject(\"WScript.Shell\")\ns.Run \"\"\"{}\"\" start\", 0, False",
            exe.display()
        );

        let path = startup_dir().join(SCRIPT_NAME);
        std::fs::write(&path, vbs).map_err(|e| format!("시작 프로그램 등록 실패: {e}"))?;

        Ok(format!("등록 위치: {}", path.display()))
    }

    pub fn uninstall() -> Result<(), String> {
        let path = startup_dir().join(SCRIPT_NAME);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("시작 프로그램 제거 실패: {e}"))?;
        }
        Ok(())
    }

    pub fn is_installed() -> bool {
        startup_dir().join(SCRIPT_NAME).exists()
    }
}

// ── macOS ───────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod platform {
    use std::path::PathBuf;

    const LABEL: &str = "com.keymander.daemon";

    fn plist_path() -> PathBuf {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"));
        home.join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist"))
    }

    pub fn install() -> Result<String, String> {
        let exe = std::env::current_exe().map_err(|e| format!("실행 파일 경로 확인 실패: {e}"))?;

        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>start</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
</dict>
</plist>"#,
            exe = exe.display(),
        );

        let path = plist_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, plist).map_err(|e| e.to_string())?;

        // load(구식)가 아닌 bootout→bootstrap: 이전 등록이 다른 바이너리 경로를
        // 가리키고 있어도 새 plist로 확실히 재바인딩된다.
        let (domain, service) = launchd_target();
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &service])
            .output();
        let _ = std::process::Command::new("launchctl")
            .args(["bootstrap", &domain, &path.to_string_lossy()])
            .output();

        Ok(format!("등록 위치: {}", path.display()))
    }

    pub fn uninstall() -> Result<(), String> {
        let path = plist_path();
        if path.exists() {
            let (_, service) = launchd_target();
            let _ = std::process::Command::new("launchctl")
                .args(["bootout", &service])
                .output();
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// launchctl 도메인(`gui/<uid>`)과 서비스 타깃 문자열
    fn launchd_target() -> (String, String) {
        let uid = std::process::Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "501".into());
        let domain = format!("gui/{uid}");
        let service = format!("{domain}/{LABEL}");
        (domain, service)
    }

    pub fn is_installed() -> bool {
        plist_path().exists()
    }
}

// ── Linux ───────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod platform {
    use std::path::PathBuf;

    const SERVICE_NAME: &str = "kmd-daemon";

    fn service_path() -> PathBuf {
        // dirs::config_dir = $XDG_CONFIG_HOME 또는 ~/.config. 예전의 /tmp 폴백은
        // 재부팅하면 사라지는 곳에 유닛을 만들어 "등록됨"으로 보고했다.
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from(".config"))
            .join("systemd/user")
            .join(format!("{SERVICE_NAME}.service"))
    }

    pub fn install() -> Result<String, String> {
        let exe = std::env::current_exe().map_err(|e| format!("실행 파일 경로 확인 실패: {e}"))?;

        let unit = format!(
            r#"[Unit]
Description=keymander Daemon

[Service]
ExecStart={exe} start
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target"#,
            exe = super::systemd_quote(&exe.to_string_lossy()),
        );

        let path = service_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, unit).map_err(|e| e.to_string())?;

        let _ = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();
        // enable 결과를 확인한다 — 예전엔 버려서, systemd 사용자 세션이 없는 환경
        // (WSL 등)에서도 "등록됨"으로 보고했다. is_installed는 파일 존재만 본다.
        let enabled = std::process::Command::new("systemctl")
            .args(["--user", "enable", SERVICE_NAME])
            .output();
        match enabled {
            Ok(o) if o.status.success() => Ok(format!("등록 위치: {}", path.display())),
            Ok(o) => Ok(format!(
                "유닛 파일은 만들었지만 활성화에 실패했습니다 ({}): {} — `systemctl --user enable {SERVICE_NAME}`로 직접 활성화하세요",
                path.display(),
                String::from_utf8_lossy(&o.stderr).trim()
            )),
            Err(e) => Ok(format!(
                "유닛 파일은 만들었지만 systemctl을 실행하지 못했습니다 ({}): {e}",
                path.display()
            )),
        }
    }

    pub fn uninstall() -> Result<(), String> {
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "disable", SERVICE_NAME])
            .output();

        let path = service_path();
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }

        let _ = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();

        Ok(())
    }

    pub fn is_installed() -> bool {
        service_path().exists()
    }
}

// ── 미지원 플랫폼 ──────────────────────────────────────────────────────────

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod platform {
    pub fn install() -> Result<String, String> {
        Err("이 플랫폼에서는 자동 시작이 지원되지 않습니다.".into())
    }

    pub fn uninstall() -> Result<(), String> {
        Ok(())
    }

    pub fn is_installed() -> bool {
        false
    }
}
