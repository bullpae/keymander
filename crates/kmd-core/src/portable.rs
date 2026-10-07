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

/// winget 설치본인가 — `%LOCALAPPDATA%\Microsoft\WinGet\Packages\<id>_...\`.
///
/// winget zip에는 `kmd-data\`가 들어 있어 [`is_portable`]이 참이 되지만, 실제로는
/// 한 PC에 설치된 앱이다. USB용 포터블과 구분해야 하는 곳(자동 시작 등록,
/// `kmd upgrade`)에서 쓴다.
pub fn is_winget_install(exe: &std::path::Path) -> bool {
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
    fn test_portable_data_dir_ends_with_kmd_data() {
        let dir = portable_data_dir().unwrap();
        assert!(dir.ends_with(PORTABLE_DIR_NAME));
    }
}
