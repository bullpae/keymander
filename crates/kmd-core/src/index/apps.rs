//! OS-specific application discovery
//!
//! Discovers installed applications:
//! - Windows: Start Menu shortcuts (.lnk files)
//! - macOS: /Applications/*.app
//! - Linux: .desktop files in XDG directories

use super::{IndexItem, ItemKind, Source};

/// Icon for applications
fn app_icon(use_emoji: bool) -> String {
    if use_emoji {
        "\u{1F4E6}".into()
    } else {
        "Ap".into()
    } // 📦 / Ap
}

/// Collect installed applications from OS-specific locations
pub fn collect_apps(use_emoji: bool) -> Vec<IndexItem> {
    let mut items = Vec::new();

    #[cfg(target_os = "windows")]
    {
        items.extend(collect_windows_apps(use_emoji));
    }

    #[cfg(target_os = "macos")]
    {
        items.extend(collect_macos_apps(use_emoji));
    }

    #[cfg(target_os = "linux")]
    {
        items.extend(collect_linux_apps(use_emoji));
    }

    items
}

/// Windows: Discover installed applications from multiple sources.
///
/// 1. **`shell:AppsFolder`** — the authoritative Windows app list (includes
///    UWP/Store/MSIX apps like Telegram, Zed, Bitwarden, etc. as well as
///    traditional Win32 apps).  Enumerated via a short PowerShell script.
/// 2. **Start Menu `.lnk` files** — fallback that catches anything
///    `shell:AppsFolder` might miss (rare, but possible for legacy installers).
///
/// Results are deduplicated by name (case-insensitive).
#[cfg(target_os = "windows")]
fn collect_windows_apps(use_emoji: bool) -> Vec<IndexItem> {
    use std::collections::HashSet;
    use std::path::PathBuf;

    let mut items = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // ── Source 1: shell:AppsFolder (PowerShell COM) ─────────────────────
    // This is the same data source Windows Start menu search uses.
    items.extend(collect_shell_apps_folder(&mut seen, use_emoji));

    // ── Source 2: Start Menu .lnk files (fallback) ─────────────────────
    let mut dirs: Vec<PathBuf> = Vec::new();

    if let Ok(appdata) = std::env::var("APPDATA") {
        dirs.push(
            PathBuf::from(&appdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );
    }

    if let Ok(programdata) = std::env::var("PROGRAMDATA") {
        dirs.push(
            PathBuf::from(&programdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );
    }

    for dir in dirs {
        scan_lnk_dir(&dir, &mut items, &mut seen, use_emoji);
    }

    items
}

/// Enumerate `shell:AppsFolder` via PowerShell COM.
///
/// Runs a small PowerShell one-liner that outputs tab-separated lines:
///   `Name\tPath`
///
/// Apps whose path looks like a regular filesystem executable get that path.
/// UWP/Store apps get an `shell:appsFolder\<id>` URI that `explorer.exe` can launch.
///
/// Timeout: 5 seconds. On failure, returns an empty vec (the .lnk fallback
/// will still run).
#[cfg(target_os = "windows")]
fn collect_shell_apps_folder(
    seen: &mut std::collections::HashSet<String>,
    use_emoji: bool,
) -> Vec<IndexItem> {
    use crate::process::HideConsole;
    use std::process::Command;

    // PowerShell script: enumerate shell:AppsFolder, output Name<TAB>Path.
    // Uses StreamWriter with explicit UTF-8 (no BOM) to bypass console
    // encoding issues when CREATE_NO_WINDOW suppresses the console.
    let ps_script = r#"
$utf8 = New-Object System.Text.UTF8Encoding($false)
$sw = New-Object System.IO.StreamWriter([Console]::OpenStandardOutput(), $utf8)
$sw.AutoFlush = $true
$shell = New-Object -ComObject Shell.Application
$folder = $shell.NameSpace('shell:AppsFolder')
foreach ($item in $folder.Items()) {
    $n = $item.Name
    $p = $item.Path
    if ($n -and $p) { $sw.WriteLine("$n`t$p") }
}
$sw.Flush()
$sw.Close()
"#;

    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        ps_script,
    ]);

    cmd.hide_console();

    let output = cmd.output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            tracing::debug!(
                "shell:AppsFolder PowerShell failed (exit {:?})",
                o.status.code()
            );
            return Vec::new();
        }
        Err(e) => {
            tracing::debug!("shell:AppsFolder PowerShell error: {}", e);
            return Vec::new();
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut items = Vec::new();

    // Noise patterns to skip (framework/runtime/extension packages)
    let skip_patterns = [
        "Microsoft.WinAppRuntime",
        "Microsoft.VCLibs",
        "Microsoft.UI.Xaml",
        "Microsoft.NET.",
        "Microsoft.D3D",
        "Microsoft.DirectX",
        "Microsoft.Services",
        "MicrosoftWindows.UndockedDevKit",
        "Microsoft.ApplicationCompatibility",
        "Microsoft.Ink.Handwriting",
        "Microsoft.LanguageExperience",
        "DesktopAppInstaller",
        "StorePurchaseApp",
        "WidgetsPlatform",
        "WinAppRuntime",
        "CrossDevice",
        "ShellExperienceHost",
        "StartExperiencesApp",
        "Debuggable Package",
    ];

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Some((name, path)) = line.split_once('\t') else {
            continue;
        };

        let name = name.trim().to_string();
        let path = path.trim().to_string();

        if name.is_empty() || path.is_empty() {
            continue;
        }

        // Skip noise
        let name_lower = name.to_lowercase();
        if name_lower.contains("uninstall") || name_lower.contains("제거") {
            continue;
        }
        if skip_patterns.iter().any(|p| path.contains(p)) {
            continue;
        }

        // Dedup by lowercase name
        if !seen.insert(name_lower) {
            continue;
        }

        // Determine the launchable path:
        // - Filesystem paths start with a drive letter (e.g. "C:\...")
        // - Everything else (CLSID, AUMID, PackageFamilyName) needs shell:appsFolder\ URI
        let is_filesystem_path = path.len() >= 3
            && path.as_bytes()[0].is_ascii_alphabetic()
            && path.as_bytes()[1] == b':'
            && path.as_bytes()[2] == b'\\';
        let launch_path = if is_filesystem_path {
            path.clone()
        } else {
            format!("shell:appsFolder\\{}", path)
        };

        items.push(IndexItem {
            name,
            path: launch_path.clone(),
            kind: ItemKind::App,
            source: Source::Apps,
            icon: app_icon(use_emoji),
            keywords: format!("{} {}", launch_path, path),
            icon_path: None,
        });
    }

    tracing::info!("shell:AppsFolder discovered {} apps", items.len());
    items
}

#[cfg(target_os = "windows")]
fn scan_lnk_dir(
    dir: &std::path::Path,
    items: &mut Vec<IndexItem>,
    seen: &mut std::collections::HashSet<String>,
    use_emoji: bool,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_lnk_dir(&path, items, seen, use_emoji);
            continue;
        }

        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };

        if ext.to_lowercase() != "lnk" {
            continue;
        }

        let name = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        // 소문자로 넣는다 — 위 AppsFolder 단계가 같은 `seen`에 소문자를 넣는다.
        // 예전엔 여기만 원래 대소문자라 "google chrome" ≠ "Google Chrome"으로
        // 중복 제거가 안 돼 같은 앱이 두 번 나왔다.
        let name_lower = name.to_lowercase();
        if name.is_empty() || !seen.insert(name_lower.clone()) {
            continue;
        }

        // Skip common uninstallers
        if name_lower.contains("uninstall") || name_lower.contains("제거") {
            continue;
        }

        let full_path = path.to_string_lossy().to_string();

        items.push(IndexItem {
            name,
            path: full_path.clone(),
            kind: ItemKind::App,
            source: Source::Apps,
            icon: app_icon(use_emoji),
            keywords: full_path,
            icon_path: None,
        });
    }
}

/// macOS: Scan application directories (재귀) + Homebrew Caskroom
#[cfg(target_os = "macos")]
fn collect_macos_apps(use_emoji: bool) -> Vec<IndexItem> {
    use std::collections::HashSet;
    use std::path::PathBuf;

    let mut items = Vec::new();
    let mut seen = HashSet::new();

    let app_dirs = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        dirs::home_dir()
            .map(|h| h.join("Applications"))
            .unwrap_or_default(),
    ];

    for dir in app_dirs {
        scan_macos_app_dir(&dir, &mut items, &mut seen, use_emoji, 3);
    }

    // Homebrew Caskroom — 심볼릭 링크가 /Applications에 없는 경우 보완
    let caskroom = PathBuf::from("/opt/homebrew/Caskroom");
    if caskroom.is_dir() {
        scan_macos_app_dir(&caskroom, &mut items, &mut seen, use_emoji, 4);
    }
    let caskroom_local = PathBuf::from("/usr/local/Caskroom");
    if caskroom_local.is_dir() {
        scan_macos_app_dir(&caskroom_local, &mut items, &mut seen, use_emoji, 4);
    }

    items
}

/// .app 번들을 재귀 탐색 (max_depth 제한으로 과도한 탐색 방지)
#[cfg(target_os = "macos")]
fn scan_macos_app_dir(
    dir: &std::path::Path,
    items: &mut Vec<IndexItem>,
    seen: &mut std::collections::HashSet<String>,
    use_emoji: bool,
    max_depth: u32,
) {
    if max_depth == 0 {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            if path.is_dir() {
                scan_macos_app_dir(&path, items, seen, use_emoji, max_depth - 1);
            }
            continue;
        };

        if ext == "app" {
            let name = path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            if name.is_empty() || !seen.insert(name.clone()) {
                continue;
            }

            let full_path = path.to_string_lossy().to_string();

            items.push(IndexItem {
                name,
                path: full_path.clone(),
                kind: ItemKind::App,
                source: Source::Apps,
                icon: app_icon(use_emoji),
                keywords: full_path,
                icon_path: None,
            });
        } else if path.is_dir() {
            scan_macos_app_dir(&path, items, seen, use_emoji, max_depth - 1);
        }
    }
}

/// Linux: Parse .desktop files from XDG data directories
#[cfg(target_os = "linux")]
fn collect_linux_apps(use_emoji: bool) -> Vec<IndexItem> {
    use std::collections::HashSet;
    use std::path::PathBuf;

    let mut items = Vec::new();
    let mut seen = HashSet::new();

    let mut dirs: Vec<PathBuf> = vec![
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
    ];

    if let Some(data_home) = dirs::data_dir() {
        dirs.push(data_home.join("applications"));
    }

    // XDG_DATA_DIRS
    if let Ok(xdg) = std::env::var("XDG_DATA_DIRS") {
        for dir in xdg.split(':') {
            dirs.push(PathBuf::from(dir).join("applications"));
        }
    }

    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if ext != "desktop" {
                continue;
            }

            if let Some(item) = parse_desktop_file(&path, &mut seen, use_emoji) {
                items.push(item);
            }
        }
    }

    items
}

/// Parse a .desktop file and extract name/exec
#[cfg(target_os = "linux")]
fn parse_desktop_file(
    path: &std::path::Path,
    seen: &mut std::collections::HashSet<String>,
    use_emoji: bool,
) -> Option<IndexItem> {
    let content = std::fs::read_to_string(path).ok()?;

    let mut name = None;
    let mut exec = None;
    let mut icon_field = None;
    let mut no_display = false;
    let mut in_desktop_entry = false;

    for line in content.lines() {
        let line = line.trim();
        if line == "[Desktop Entry]" {
            in_desktop_entry = true;
            continue;
        }
        if line.starts_with('[') {
            in_desktop_entry = false;
            continue;
        }
        if !in_desktop_entry {
            continue;
        }

        if let Some(val) = line.strip_prefix("Name=") {
            name = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("Exec=") {
            let clean = val
                .split_whitespace()
                .filter(|s| !s.starts_with('%'))
                .collect::<Vec<_>>()
                .join(" ");
            exec = Some(clean);
        } else if let Some(val) = line.strip_prefix("Icon=") {
            let v = val.trim();
            if !v.is_empty() {
                icon_field = Some(v.to_string());
            }
        } else if let Some(val) = line.strip_prefix("NoDisplay=") {
            no_display = val.trim().eq_ignore_ascii_case("true");
        }
    }

    if no_display {
        return None;
    }

    let name = name?;
    let exec = exec.unwrap_or_default();

    if !seen.insert(name.clone()) {
        return None;
    }

    let resolved_icon = icon_field.and_then(|ic| resolve_linux_icon_path(&ic));

    Some(IndexItem {
        name: name.clone(),
        path: exec.clone(),
        kind: ItemKind::App,
        source: Source::Apps,
        icon: app_icon(use_emoji),
        keywords: format!("{} {}", name, exec),
        icon_path: resolved_icon,
    })
}

/// `.desktop` Icon= 값을 실제 파일 경로로 해석.
/// 절대 경로면 그대로, 테마 아이콘 이름이면 표준 디렉토리에서 PNG 검색.
#[cfg(target_os = "linux")]
fn resolve_linux_icon_path(icon: &str) -> Option<String> {
    use std::path::Path;

    if icon.is_empty() {
        return None;
    }

    // 절대 경로 → 파일 존재 확인
    if icon.starts_with('/') {
        let p = Path::new(icon);
        if p.exists() {
            return Some(icon.to_string());
        }
        // .png 확장자 없이 저장된 경우
        let with_png = format!("{icon}.png");
        if Path::new(&with_png).exists() {
            return Some(with_png);
        }
        return None;
    }

    // 테마 아이콘 이름 → 표준 경로에서 검색 (PNG 우선)
    let sizes = ["128x128", "64x64", "48x48", "256x256", "scalable"];
    let categories = ["apps", "mimetypes"];

    for size in &sizes {
        for cat in &categories {
            let path = format!("/usr/share/icons/hicolor/{size}/{cat}/{icon}.png");
            if Path::new(&path).exists() {
                return Some(path);
            }
        }
    }

    // pixmaps 폴백
    for ext in ["png", "xpm"] {
        let path = format!("/usr/share/pixmaps/{icon}.{ext}");
        if Path::new(&path).exists() {
            return Some(path);
        }
    }

    // SVG (scalable)
    let svg = format!("/usr/share/icons/hicolor/scalable/apps/{icon}.svg");
    if Path::new(&svg).exists() {
        return Some(svg);
    }

    None
}
