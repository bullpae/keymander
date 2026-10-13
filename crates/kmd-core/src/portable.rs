//! Portable mode detection and path resolution.
//!
//! When a `kmd-data/` directory exists next to the executable, keymander runs
//! in **portable mode**: all configuration, database, and cache files are stored
//! inside that directory instead of the OS-standard locations.
//!
//! This makes it possible to carry keymander on a USB drive with all settings
//! and history intact.

use std::path::PathBuf;

/// Name of the portable data directory (sits next to the executable).
pub const PORTABLE_DIR_NAME: &str = "kmd-data";

/// Return the directory that contains the running executable.
pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
}

/// Return the portable data directory path (may or may not exist on disk).
pub fn portable_data_dir() -> Option<PathBuf> {
    exe_dir().map(|d| d.join(PORTABLE_DIR_NAME))
}

/// Check whether portable mode is active.
///
/// Portable mode is active when a `kmd-data/` directory exists next to the
/// executable.
pub fn is_portable() -> bool {
    portable_data_dir().map(|d| d.is_dir()).unwrap_or(false)
}

/// 배포 채널 표시 파일 — exe 옆에 두고 내용으로 채널을 적는다 (`winget`).
///
/// winget 전용 zip에만 들어 있다. 설치 위치(`--location`, portable 루트 변경,
/// 관리자 설치)와 상관없이 채널을 알 수 있는 유일한 신호다.
pub const CHANNEL_FILE: &str = ".channel";

/// exe 옆의 [`CHANNEL_FILE`]에 적힌 배포 채널 (없으면 `None`)
pub fn install_channel(exe: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(exe.parent()?.join(CHANNEL_FILE)).ok()?;
    let channel = text.trim().to_ascii_lowercase();
    (!channel.is_empty()).then_some(channel)
}

/// winget 설치본인가.
///
/// winget 설치본은 USB용 포터블과 구분해야 한다(자동 시작 등록, `kmd upgrade`).
/// 1. winget 전용 zip의 [`CHANNEL_FILE`] — 정본
/// 2. 경로 `\Microsoft\WinGet\Packages\` — 표시 파일이 없던 v0.16.12 이전
///    설치본용 보조 판별. 기본 설치 위치에서만 맞는다
pub fn is_winget_install(exe: &std::path::Path) -> bool {
    if install_channel(exe).as_deref() == Some("winget") {
        return true;
    }
    exe.to_string_lossy()
        .to_lowercase()
        .replace('/', "\\")
        .contains("\\microsoft\\winget\\packages\\")
}

/// 실행 중인 exe가 winget 설치본인가
pub fn running_from_winget() -> bool {
    std::env::current_exe()
        .map(|p| is_winget_install(&p))
        .unwrap_or(false)
}

/// Enable portable mode by creating the `kmd-data/` directory.
///
/// Returns the path to the created directory.
pub fn enable() -> std::io::Result<PathBuf> {
    let dir = portable_data_dir()
        .ok_or_else(|| std::io::Error::other("Cannot determine exe directory"))?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Disable portable mode by removing the `kmd-data/` directory.
///
/// The directory must be empty or this will fail. Callers should migrate
/// files out first.
pub fn disable() -> std::io::Result<()> {
    let dir = portable_data_dir()
        .ok_or_else(|| std::io::Error::other("Cannot determine exe directory"))?;
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exe_dir_returns_some() {
        // Should always succeed in a test runner
        assert!(exe_dir().is_some());
    }

    #[test]
    fn winget_설치본_판별() {
        use std::path::Path;
        assert!(is_winget_install(Path::new(
            r"C:\Users\u\AppData\Local\Microsoft\WinGet\Packages\bullpae.keymander_Microsoft.Winget.Source_8wekyb3d8bbwe\keymander\kmd.exe"
        )));
        assert!(!is_winget_install(Path::new(r"D:\tools\keymander\kmd.exe")));
        assert!(!is_winget_install(Path::new(
            r"O:\repo\keymander-cli\target\debug\kmd.exe"
        )));
    }

    #[test]
    fn 채널_표시_파일이_있으면_어디에_설치돼도_winget() {
        // `winget install --location D:\Apps\...` 처럼 기본 경로가 아니어도 판별돼야 한다
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("kmd.exe");
        assert!(!is_winget_install(&exe), "표시 파일 없으면 경로로만 판단");

        std::fs::write(dir.path().join(CHANNEL_FILE), "winget\r\n").unwrap();
        assert_eq!(install_channel(&exe).as_deref(), Some("winget"));
        assert!(is_winget_install(&exe));

        std::fs::write(dir.path().join(CHANNEL_FILE), "portable").unwrap();
        assert!(!is_winget_install(&exe), "다른 채널은 winget이 아니다");
    }

    #[test]
    fn test_portable_data_dir_ends_with_kmd_data() {
        let dir = portable_data_dir().unwrap();
        assert!(dir.ends_with(PORTABLE_DIR_NAME));
    }
}
