//! File search providers — collect files from various backends
//!
//! Supports: walkdir builtin, fd, Everything (Windows), mdfind (macOS),
//! locate (Linux), and Windows PowerShell fallback.
//!
//! Strategy: "priority directories first" — Desktop, Documents, Downloads
//! are scanned before general home directory traversal so that user documents
//! are always indexed even if max_results is reached.

use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use walkdir::WalkDir;

use super::{IndexItem, ItemKind, Source};

/// Windows process creation flag to suppress console window popup.
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ============================================================================
// Public Types
// ============================================================================

/// Provider kind
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProviderKind {
    /// Built-in walkdir scanner (always available, no external tools)
    Builtin,
    /// fd / fdfind (cross-platform)
    Fd,
    /// voidtools Everything (Windows)
    Everything,
    /// macOS Spotlight (mdfind)
    Spotlight,
    /// plocate / mlocate (Linux)
    Locate,
    /// Windows built-in PowerShell scan
    WinFs,
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builtin => write!(f, "builtin (walkdir)"),
            Self::Fd => write!(f, "fd"),
            Self::Everything => write!(f, "everything"),
            Self::Spotlight => write!(f, "mdfind"),
            Self::Locate => write!(f, "locate"),
            Self::WinFs => write!(f, "winfs (PowerShell)"),
        }
    }
}

/// Provider configuration
#[derive(Clone)]
pub struct ProviderConfig {
    pub max_results: usize,
    pub search_depth: usize,
    pub search_paths: Vec<PathBuf>,
    pub ignore_patterns: Vec<String>,
    pub everything_path: Option<PathBuf>,
    /// Auto-scan available drive roots
    pub scan_drives: bool,
    /// Max depth when scanning drive roots
    pub drive_scan_depth: usize,
    /// Use emoji icons (true = emoji, false = ASCII 2-char)
    pub use_emoji: bool,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            max_results: 10000,
            search_depth: 6,
            search_paths: vec![],
            ignore_patterns: vec![
                ".git".to_string(),
                "node_modules".to_string(),
                "target".to_string(),
            ],
            everything_path: None,
            scan_drives: true,
            drive_scan_depth: 3,
            use_emoji: true,
        }
    }
}

// ============================================================================
// Priority Directories
// ============================================================================

/// Build the list of directories to scan, driven entirely by config.
///
/// 1. Start with `config.search_paths` (user-configured, visible in settings).
/// 2. If `config.scan_drives` is true, auto-discover available drive roots
///    that aren't already in the list.
fn scan_directories(config: &ProviderConfig) -> Vec<(PathBuf, usize)> {
    let mut dirs: Vec<(PathBuf, usize)> = Vec::new();

    // User-configured paths get full search_depth
    for p in &config.search_paths {
        if p.is_dir() {
            dirs.push((p.clone(), config.search_depth));
        }
    }

    // Auto-discover drive roots if enabled
    if config.scan_drives {
        #[cfg(target_os = "windows")]
        {
            for letter in 'C'..='Z' {
                let drive = PathBuf::from(format!("{}:\\", letter));
                if drive.is_dir() && !dirs.iter().any(|(d, _)| d == &drive) {
                    dirs.push((drive, config.drive_scan_depth));
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            let volumes = PathBuf::from("/Volumes");
            if volumes.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&volumes) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_dir() && !dirs.iter().any(|(d, _)| d == &p) {
                            dirs.push((p, config.drive_scan_depth));
                        }
                    }
                }
            }
        }

        #[cfg(target_os = "linux")]
        {
            for mount_point in &["/mnt", "/media"] {
                let mp = PathBuf::from(mount_point);
                if mp.is_dir() {
                    if let Ok(entries) = std::fs::read_dir(&mp) {
                        for entry in entries.flatten() {
                            let p = entry.path();
                            if p.is_dir() && !dirs.iter().any(|(d, _)| d == &p) {
                                dirs.push((p, config.drive_scan_depth));
                            }
                        }
                    }
                }
            }
        }
    }

    dirs
}

/// Collect files from configured scan directories using walkdir.
/// Each directory is scanned at its own depth (user dirs = full depth,
/// drive roots = drive_scan_depth).
pub fn collect_priority_files(config: &ProviderConfig) -> Vec<IndexItem> {
    let scan_dirs = scan_directories(config);

    if scan_dirs.is_empty() {
        return Vec::new();
    }

    let ignore_set: HashSet<&str> = config.ignore_patterns.iter().map(|s| s.as_str()).collect();
    let limit = config.max_results / 2; // Reserve half for general scan

    let mut items = Vec::new();
    let mut seen = HashSet::new();

    for (dir, depth) in &scan_dirs {
        tracing::info!("Scanning: {} (depth {})", dir.display(), depth);

        let walker = WalkDir::new(dir)
            .max_depth(*depth)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !is_ignored_dir(e, &ignore_set));

        walkdir_into_items(walker, &mut items, &mut seen, limit, config.use_emoji);

        if items.len() >= limit {
            break;
        }
    }

    tracing::info!(
        "Scan directories: {} items (files + dirs) found",
        items.len()
    );
    items
}

// ============================================================================
// Provider Detection
// ============================================================================

/// Auto-detect the best available provider
pub fn detect_provider(preference: &str, everything_path: Option<&PathBuf>) -> ProviderKind {
    match preference.to_lowercase().as_str() {
        "builtin" | "walkdir" => return ProviderKind::Builtin,
        "fd" if which("fd").is_some() || which("fdfind").is_some() => return ProviderKind::Fd,
        "everything"
            if cfg!(target_os = "windows") && find_everything_cli(everything_path).is_some() =>
        {
            return ProviderKind::Everything;
        }
        "winfs" | "powershell" => {
            if cfg!(target_os = "windows") {
                return ProviderKind::WinFs;
            }
        }
        "mdfind" | "spotlight" => {
            if cfg!(target_os = "macos") {
                return ProviderKind::Spotlight;
            }
        }
        "locate" | "plocate" if which("plocate").is_some() || which("locate").is_some() => {
            return ProviderKind::Locate;
        }
        _ => {} // "auto" → fall through to auto-detect
    }

    // Auto-detect priority
    if cfg!(target_os = "windows") && find_everything_cli(everything_path).is_some() {
        return ProviderKind::Everything;
    }
    if which("fd").is_some() || which("fdfind").is_some() {
        return ProviderKind::Fd;
    }
    if cfg!(target_os = "macos") {
        return ProviderKind::Spotlight;
    }
    if which("plocate").is_some() || which("locate").is_some() {
        return ProviderKind::Locate;
    }

    // Always fall back to builtin (walkdir) — never return empty
    ProviderKind::Builtin
}

// ============================================================================
// Provider Implementations
// ============================================================================

/// Collect files using the specified provider.
/// NOTE: Priority directory files are collected separately in Index::build().
/// This function collects additional files from the general home directory.
pub fn collect_files(
    kind: ProviderKind,
    config: &ProviderConfig,
    existing_count: usize,
) -> Vec<IndexItem> {
    let remaining = config.max_results.saturating_sub(existing_count);
    if remaining == 0 {
        return Vec::new();
    }

    let mut limited_config = config.clone();
    limited_config.max_results = remaining;

    tracing::info!("File provider: {} (quota: {} files)", kind, remaining);

    let items = match kind {
        // walkdir는 걷는 동안 같은 규칙을 이미 적용한다 (is_ignored_dir·fsutil)
        ProviderKind::Builtin => return collect_builtin(&limited_config),
        ProviderKind::Fd => collect_fd(&limited_config),
        ProviderKind::Everything => collect_everything(&limited_config),
        ProviderKind::Spotlight => collect_spotlight(&limited_config),
        ProviderKind::Locate => collect_locate(&limited_config),
        ProviderKind::WinFs => collect_windows_fs(&limited_config),
    };
    apply_common_rules(items, &limited_config)
}

/// 외부 도구(fd·Everything·Spotlight·locate·PowerShell)의 결과에 **같은 규칙**을 건다.
///
/// 도구마다 같은 설정을 다르게 해석했다 — fd·Everything·PowerShell은 검색 폴더
/// 안에서 깊이·무시 목록을 지켰지만, Spotlight(macOS 기본)는 **첫 검색 폴더만**
/// 보면서 무시 목록·숨김·깊이를 모두 무시했고, locate(Linux)는 검색 폴더와
/// 상관없이 **파일시스템 전체**에서 할당량을 채웠다. 같은 config인데 OS에 따라
/// 색인에 `node_modules` 안의 문서나 `/usr/share` 파일이 섞였다.
/// 도구는 후보를 내고, 무엇을 받아들일지는 여기서 정한다.
fn apply_common_rules(items: Vec<IndexItem>, config: &ProviderConfig) -> Vec<IndexItem> {
    let roots = provider_roots(config);
    let ignore: HashSet<&str> = config.ignore_patterns.iter().map(|s| s.as_str()).collect();
    let before = items.len();
    let kept: Vec<IndexItem> = items
        .into_iter()
        .filter(|it| {
            allowed_by_common_rules(Path::new(&it.path), &roots, &ignore, config.search_depth)
        })
        .collect();
    if kept.len() != before {
        tracing::info!(
            "공통 규칙으로 제외: {}개 (범위 밖·숨김·무시·깊이)",
            before - kept.len()
        );
    }
    kept
}

/// 외부 도구의 검색 범위 — 검색 폴더, 없으면 홈 (fd·PowerShell과 같은 규칙).
fn provider_roots(config: &ProviderConfig) -> Vec<PathBuf> {
    if config.search_paths.is_empty() {
        dirs::home_dir().into_iter().collect()
    } else {
        config.search_paths.clone()
    }
}

/// `path`가 어떤 루트 아래에서 깊이 이내이고, 그 사이에 숨김·무시 폴더가 없는가.
/// walkdir 경로(`is_ignored_dir`·`max_depth`)와 같은 판정.
fn allowed_by_common_rules(
    path: &Path,
    roots: &[PathBuf],
    ignore: &HashSet<&str>,
    max_depth: usize,
) -> bool {
    roots.iter().any(|root| {
        if !crate::fsutil::path_within(path, root) {
            return false;
        }
        let rel: Vec<String> = path
            .components()
            .skip(root.components().count())
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        if rel.is_empty() || rel.len() > max_depth {
            return false; // 루트 자신은 항목이 아니다 (walkdir depth 0 제외와 같음)
        }
        !rel.iter().any(|name| {
            crate::fsutil::is_hidden_name(name)
                || name.starts_with('$')
                || ignore.contains(name.as_str())
        })
    })
}

// ── Builtin (walkdir) ────────────────────────────────────

fn collect_builtin(config: &ProviderConfig) -> Vec<IndexItem> {
    // General scan: always scan from home directory.
    // Priority scan already covered search_paths + drive roots,
    // so this fills in home directory content not already indexed.
    let roots: Vec<PathBuf> = dirs::home_dir().into_iter().collect();
    if roots.is_empty() {
        return Vec::new();
    }

    let ignore_set: HashSet<&str> = config.ignore_patterns.iter().map(|s| s.as_str()).collect();
    let priority_set: HashSet<PathBuf> = config.search_paths.iter().cloned().collect();

    let mut items = Vec::new();
    let mut seen = HashSet::new();

    for root in &roots {
        let walker = WalkDir::new(root)
            .max_depth(config.search_depth)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if is_ignored_dir(e, &ignore_set) {
                    return false;
                }
                // Skip priority directories (already scanned)
                if e.file_type().is_dir() && priority_set.contains(&e.path().to_path_buf()) {
                    return false;
                }
                true
            });

        walkdir_into_items(
            walker,
            &mut items,
            &mut seen,
            config.max_results,
            config.use_emoji,
        );

        if items.len() >= config.max_results {
            break;
        }
    }

    tracing::info!("Builtin provider: {} files found", items.len());
    items
}

// ── fd / fdfind ──────────────────────────────────────────

fn collect_fd(config: &ProviderConfig) -> Vec<IndexItem> {
    let cmd_name = if which("fd").is_some() {
        "fd"
    } else if which("fdfind").is_some() {
        "fdfind"
    } else {
        return Vec::new();
    };

    let mut args = vec![
        ".".to_string(),
        "--type".to_string(),
        "f".to_string(),
        "--type".to_string(),
        "d".to_string(),
        "--max-results".to_string(),
        config.max_results.to_string(),
        "--max-depth".to_string(),
        config.search_depth.to_string(),
        "--color".to_string(),
        "never".to_string(),
    ];

    for pattern in &config.ignore_patterns {
        args.push("--exclude".to_string());
        args.push(pattern.clone());
    }

    let search_dirs: Vec<PathBuf> = if config.search_paths.is_empty() {
        dirs::home_dir().into_iter().collect()
    } else {
        config.search_paths.clone()
    };

    let mut all_items = Vec::new();

    for dir in &search_dirs {
        let mut cmd = Command::new(cmd_name);
        cmd.args(&args)
            .arg(dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let output = cmd.output();

        match output {
            Ok(output) => {
                parse_line_output_into(
                    &output.stdout,
                    &mut all_items,
                    config.max_results,
                    config.use_emoji,
                );
            }
            Err(e) => {
                tracing::warn!("fd failed for {}: {}", dir.display(), e);
            }
        }

        if all_items.len() >= config.max_results {
            break;
        }
    }

    tracing::info!("fd provider: {} files found", all_items.len());
    all_items
}

// ── Everything (Windows) ────────────────────────────────

fn collect_everything(config: &ProviderConfig) -> Vec<IndexItem> {
    let es_path = match find_everything_cli(config.everything_path.as_ref()) {
        Some(p) => p,
        None => return Vec::new(),
    };

    let mut args = vec![
        "-n".to_string(),
        config.max_results.to_string(),
        "-s".to_string(),
    ];

    if !config.search_paths.is_empty() {
        let path_filter: Vec<String> = config
            .search_paths
            .iter()
            .map(|p| format!("\"{}\"", p.to_string_lossy()))
            .collect();
        args.push(path_filter.join(" | "));
    }

    let mut cmd = Command::new(&es_path);
    cmd.args(&args).stdout(Stdio::piped()).stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    match cmd.output() {
        Ok(output) => {
            let mut items = Vec::new();
            parse_line_output_into(
                &output.stdout,
                &mut items,
                config.max_results,
                config.use_emoji,
            );
            tracing::info!("Everything provider: {} files found", items.len());
            items
        }
        Err(e) => {
            tracing::warn!("Everything CLI failed: {}", e);
            Vec::new()
        }
    }
}

// ── mdfind (macOS) ──────────────────────────────────────

fn collect_spotlight(config: &ProviderConfig) -> Vec<IndexItem> {
    let mut args = vec!["kind:document OR kind:folder".to_string()];

    // 모든 검색 폴더 — 예전엔 첫 번째만 넘겨 나머지 폴더는 이 도구로 찾지 못했다.
    // (-onlyin은 반복할 수 있다)
    for dir in provider_roots(config) {
        args.push("-onlyin".to_string());
        args.push(dir.to_string_lossy().to_string());
    }

    match Command::new("mdfind")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        Ok(output) => {
            let mut items = Vec::new();
            parse_line_output_into(
                &output.stdout,
                &mut items,
                config.max_results,
                config.use_emoji,
            );
            tracing::info!("Spotlight provider: {} files found", items.len());
            items
        }
        Err(e) => {
            tracing::warn!("mdfind failed: {}", e);
            Vec::new()
        }
    }
}

// ── locate / plocate (Linux) ────────────────────────────

fn collect_locate(config: &ProviderConfig) -> Vec<IndexItem> {
    let cmd = if which("plocate").is_some() {
        "plocate"
    } else if which("locate").is_some() {
        "locate"
    } else {
        return Vec::new();
    };

    // `/` 전체가 아니라 검색 범위로 — 할당량을 시스템 파일에 쓰지 않는다.
    // locate의 패턴은 경로 부분 문자열이고 여러 개면 OR다. 정확한 범위·깊이는
    // apply_common_rules가 다시 거른다.
    let roots: Vec<String> = provider_roots(config)
        .iter()
        .map(|r| r.to_string_lossy().into_owned())
        .collect();
    if roots.is_empty() {
        return Vec::new();
    }
    match Command::new(cmd)
        .args(["--limit", &config.max_results.to_string(), "--existing"])
        .args(&roots)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        Ok(output) => {
            let mut items = Vec::new();
            parse_line_output_into(
                &output.stdout,
                &mut items,
                config.max_results,
                config.use_emoji,
            );
            tracing::info!("locate provider: {} files found", items.len());
            items
        }
        Err(e) => {
            tracing::warn!("locate failed: {}", e);
            Vec::new()
        }
    }
}

// ── Windows FS (PowerShell fallback) ────────────────────

fn collect_windows_fs(config: &ProviderConfig) -> Vec<IndexItem> {
    if !cfg!(target_os = "windows") {
        return Vec::new();
    }

    let roots: Vec<PathBuf> = if config.search_paths.is_empty() {
        // Scan home directory (priority dirs already indexed separately)
        std::env::var("USERPROFILE")
            .ok()
            .map(PathBuf::from)
            .into_iter()
            .collect()
    } else {
        config.search_paths.clone()
    };

    // Build exclude list for PowerShell (escape single quotes)
    let exclude_dirs: String = config
        .ignore_patterns
        .iter()
        .map(|p| format!("'{}'", p.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",");

    let mut items = Vec::new();

    for root in roots {
        let root_s = root.to_string_lossy().replace('\'', "''");
        // Use Where-Object to filter out ignored directories from the path
        let script = format!(
            "$utf8 = New-Object System.Text.UTF8Encoding($false); \
             $sw = New-Object System.IO.StreamWriter([Console]::OpenStandardOutput(), $utf8); \
             $excludeDirs = @({}); \
             Get-ChildItem -LiteralPath '{}' -File -Recurse -Depth {} \
             -ErrorAction SilentlyContinue | \
             Where-Object {{ $p = $_.FullName; -not ($excludeDirs | Where-Object {{ $p -like \"*\\$_\\*\" }}) }} | \
             Select-Object -First {} -ExpandProperty FullName | \
             ForEach-Object {{ $sw.WriteLine($_) }}; \
             $sw.Flush(); $sw.Close()",
            exclude_dirs,
            root_s,
            config.search_depth,
            config.max_results,
        );

        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-Command", &script])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        match cmd.output() {
            Ok(output) => {
                parse_line_output_into(
                    &output.stdout,
                    &mut items,
                    config.max_results,
                    config.use_emoji,
                );
            }
            Err(e) => {
                tracing::warn!("PowerShell scan failed for {}: {}", root.display(), e);
            }
        }

        if items.len() >= config.max_results {
            items.truncate(config.max_results);
            break;
        }
    }

    tracing::info!("WinFs provider: {} files found", items.len());
    items
}

// ============================================================================
// Helpers
// ============================================================================

/// Shared walkdir entry → IndexItem conversion.
/// Processes entries from a walkdir iterator, deduplicates, and appends to `items`.
fn walkdir_into_items<I>(
    walker: I,
    items: &mut Vec<IndexItem>,
    seen: &mut HashSet<String>,
    limit: usize,
    use_emoji: bool,
) where
    I: Iterator<Item = walkdir::Result<walkdir::DirEntry>>,
{
    for entry in walker {
        let Ok(entry) = entry else { continue };

        let is_file = entry.file_type().is_file();
        let is_dir = entry.file_type().is_dir();

        // Skip entries that are neither files nor directories (e.g. symlinks)
        if !is_file && !is_dir {
            continue;
        }
        // Skip the root scan directory itself (depth 0)
        if is_dir && entry.depth() == 0 {
            continue;
        }

        let path = entry.path();
        let path_str = path.to_string_lossy().to_string();

        // Deduplicate
        if !seen.insert(path_str.clone()) {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        // 숨김 규칙은 OS마다 다르다(Windows는 속성) — fsutil 참조
        if name.is_empty() || crate::fsutil::is_hidden_walk_entry(&entry) {
            continue;
        }

        items.push(IndexItem {
            name: name.clone(),
            path: path_str.clone(),
            kind: if is_dir {
                ItemKind::Directory
            } else {
                ItemKind::File
            },
            source: Source::FileProvider,
            icon: if is_dir {
                dir_icon(use_emoji)
            } else {
                icon_for_path(path, use_emoji)
            },
            keywords: path_str,
            icon_path: None,
        });

        if items.len() >= limit {
            break;
        }
    }
}

/// Check if a walkdir entry should be ignored (directory name matches ignore pattern)
/// (content_index의 본문 스캔도 같은 무시 규칙을 공유한다)
pub(crate) fn is_ignored_dir(entry: &walkdir::DirEntry, ignore_set: &HashSet<&str>) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    let name = entry.file_name().to_str().unwrap_or("");
    // 숨김 폴더 — `.`으로 시작하거나 Windows 숨김 속성(AppData 등). fsutil 참조
    if crate::fsutil::is_hidden_walk_entry(entry) {
        return true;
    }
    // Skip system directories starting with '$' (e.g. $Recycle.Bin, $WinREAgent)
    if name.starts_with('$') {
        return true;
    }
    ignore_set.contains(name)
}

/// Parse line-by-line command output into IndexItems
fn parse_line_output_into(stdout: &[u8], items: &mut Vec<IndexItem>, max: usize, use_emoji: bool) {
    let reader = BufReader::new(stdout);
    for line in reader.lines().map_while(Result::ok) {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let path = PathBuf::from(&line);
        let is_dir = path.is_dir();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&line)
            .to_string();

        items.push(IndexItem {
            name: name.clone(),
            path: line.clone(),
            kind: if is_dir {
                ItemKind::Directory
            } else {
                ItemKind::File
            },
            source: Source::FileProvider,
            icon: if is_dir {
                dir_icon(use_emoji)
            } else {
                icon_for_path(&path, use_emoji)
            },
            keywords: line,
            icon_path: None,
        });

        if items.len() >= max {
            break;
        }
    }
}

/// Check if an executable exists in PATH
fn which(name: &str) -> Option<PathBuf> {
    let candidates: Vec<String> = if cfg!(target_os = "windows") {
        vec![
            name.to_string(),
            format!("{}.exe", name),
            format!("{}.cmd", name),
        ]
    } else {
        vec![name.to_string()]
    };

    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            candidates.iter().find_map(|candidate| {
                let full = dir.join(candidate);
                if full.is_file() {
                    Some(full)
                } else {
                    None
                }
            })
        })
    })
}

/// Find Everything CLI (es.exe)
fn find_everything_cli(configured: Option<&PathBuf>) -> Option<PathBuf> {
    if let Some(path) = configured {
        if path.is_file() {
            return Some(path.clone());
        }
    }

    if let Some(path) = which("es") {
        return Some(path);
    }

    if cfg!(target_os = "windows") {
        let common_paths = [
            r"C:\Program Files\Everything\es.exe",
            r"C:\Program Files (x86)\Everything\es.exe",
            r"C:\Program Files\Everything 1.5a\es.exe",
        ];
        for path_str in &common_paths {
            let path = PathBuf::from(path_str);
            if path.is_file() {
                return Some(path);
            }
        }

        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let path = PathBuf::from(&local).join("Everything").join("es.exe");
            if path.is_file() {
                return Some(path);
            }
        }
    }

    None
}

/// Icon width in display columns — all icons are exactly this wide.
pub const ICON_WIDTH: usize = 2;

// ── Icon mapping table (emoji, ASCII) ────────────────────────────────────────

/// (extensions, emoji, ascii)
const ICON_TABLE: &[(&[&str], &str, &str)] = &[
    // Programming
    (&["rs"], "\u{1F980}", "Rs"),                               // 🦀
    (&["py", "pyw"], "\u{1F40D}", "Py"),                        // 🐍
    (&["js", "ts", "jsx", "tsx", "mjs"], "\u{1F4DC}", "Js"),    // 📜
    (&["go"], "\u{1F535}", "Go"),                               // 🔵
    (&["java", "kt", "kts"], "\u{2615}", "Jv"),                 // ☕
    (&["c", "cpp", "h", "hpp", "cc", "cxx"], "\u{2699}", "C+"), // ⚙
    (&["cs"], "\u{1F7E3}", "C#"),                               // 🟣
    (
        &["sh", "bash", "zsh", "fish", "ps1", "bat", "cmd"],
        "\u{1F41A}",
        "$>",
    ), // 🐚
    // Documents
    (&["md", "txt", "rtf", "log"], "\u{1F4DD}", "Tx"), // 📝
    (&["pdf"], "\u{1F4D5}", "Pd"),                     // 📕
    (&["hwp", "hwpx"], "\u{1F4D8}", "Hw"),             // 📘
    (&["doc", "docx", "odt"], "\u{1F4C4}", "Dc"),      // 📄
    (&["xls", "xlsx", "ods", "csv"], "\u{1F4CA}", "Xl"), // 📊
    (&["ppt", "pptx", "odp"], "\u{1F4CA}", "Pt"),      // 📊
    // Data / Config
    (
        &["json", "yaml", "yml", "toml", "xml", "ini", "conf"],
        "\u{1F4CB}",
        "{}",
    ), // 📋
    (&["sql", "db", "sqlite", "sqlite3"], "\u{1F5C3}", "Db"), // 🗃
    (&["html", "htm", "css", "scss", "less"], "\u{1F310}", "<>"), // 🌐
    // Media
    (
        &[
            "png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "ico", "tiff",
        ],
        "\u{1F5BC}",
        "Im",
    ), // 🖼
    (
        &["mp3", "wav", "flac", "aac", "ogg", "m4a", "wma"],
        "\u{1F3B5}",
        "Au",
    ), // 🎵
    (
        &["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm"],
        "\u{1F3AC}",
        "Vd",
    ), // 🎬
    // Archives
    (
        &["zip", "tar", "gz", "7z", "rar", "bz2", "xz", "zst"],
        "\u{1F4E6}",
        "Pk",
    ), // 📦
    // Executables / Installers
    (
        &["exe", "msi", "appimage", "deb", "rpm", "dmg"],
        "\u{1F4E6}",
        "Ex",
    ), // 📦
    // Fonts
    (&["ttf", "otf", "woff", "woff2"], "\u{1F524}", "Ft"), // 🔤
];

const DEFAULT_EMOJI: &str = "\u{1F4C4}"; // 📄
const DEFAULT_ASCII: &str = "--";
const DIR_EMOJI: &str = "\u{1F4C1}"; // 📁
const DIR_ASCII: &str = ">>";

/// Get an icon for a file path.  When `use_emoji` is true, rich emoji icons
/// are returned (requires a modern terminal like Windows Terminal, iTerm2,
/// kitty).  When false, simple 2-char ASCII labels are used that work
/// everywhere including the Windows legacy console (conhost).
pub fn icon_for_path(path: &Path, use_emoji: bool) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if ext.is_empty() && path.is_dir() {
        return dir_icon(use_emoji);
    }

    for &(exts, emoji, ascii) in ICON_TABLE {
        if exts.iter().any(|e| *e == ext) {
            return if use_emoji { emoji } else { ascii }.into();
        }
    }

    if use_emoji {
        DEFAULT_EMOJI
    } else {
        DEFAULT_ASCII
    }
    .into()
}

/// Directory icon appropriate for the given mode.
pub fn dir_icon(use_emoji: bool) -> String {
    if use_emoji {
        DIR_EMOJI.into()
    } else {
        DIR_ASCII.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 외부 도구 결과의 공통 규칙 (OS마다 갈라졌던 해석) ──

    fn rules_ok(path: &str, roots: &[&str], depth: usize) -> bool {
        let roots: Vec<PathBuf> = roots.iter().map(PathBuf::from).collect();
        let ignore: HashSet<&str> = ["node_modules", "Library"].into_iter().collect();
        allowed_by_common_rules(Path::new(path), &roots, &ignore, depth)
    }

    #[test]
    fn 공통규칙_검색범위_밖은_뺀다() {
        // locate가 `/` 전체를 훑어 들어오던 시스템 파일
        assert!(!rules_ok(
            "/usr/share/doc/a.txt",
            &["/home/me/Documents"],
            4
        ));
        assert!(rules_ok(
            "/home/me/Documents/a.txt",
            &["/home/me/Documents"],
            4
        ));
        // 두 번째 검색 폴더도 범위다 (Spotlight가 첫 번째만 보던 문제)
        assert!(rules_ok(
            "/home/me/Downloads/b.pdf",
            &["/home/me/Documents", "/home/me/Downloads"],
            4
        ));
    }

    #[test]
    fn 공통규칙_숨김_무시_폴더_아래는_뺀다() {
        let root = ["/home/me/Desktop"];
        assert!(!rules_ok(
            "/home/me/Desktop/proj/node_modules/x/README.md",
            &root,
            9
        ));
        assert!(!rules_ok("/home/me/Desktop/.git/HEAD", &root, 9));
        assert!(!rules_ok("/home/me/Desktop/.secret.txt", &root, 9));
        assert!(!rules_ok("/home/me/Desktop/$RECYCLE.BIN/x", &root, 9));
        assert!(rules_ok("/home/me/Desktop/proj/notes.md", &root, 9));
    }

    #[test]
    fn 공통규칙_깊이는_walkdir와_같다() {
        let root = ["/r"];
        assert!(rules_ok("/r/a", &root, 1), "depth 1 = 루트 바로 아래");
        assert!(!rules_ok("/r/a/b", &root, 1));
        assert!(!rules_ok("/r", &root, 4), "루트 자신은 항목이 아니다");
    }

    fn must_mkdir(path: &Path) {
        std::fs::create_dir_all(path).expect("디렉터리 생성 실패");
    }

    fn must_write(path: &Path, content: &str) {
        std::fs::write(path, content).expect("파일 쓰기 실패");
    }

    #[test]
    fn walkdir_into_items_skips_hidden_and_root_and_respects_limit() {
        let tmp = tempfile::tempdir().expect("tempdir 생성 실패");
        let root = tmp.path();

        let visible = root.join("visible.txt");
        let hidden = root.join(".hidden.txt");
        let docs = root.join("docs");
        let nested = docs.join("note.md");

        must_write(&visible, "ok");
        must_write(&hidden, "hidden");
        must_mkdir(&docs);
        must_write(&nested, "nested");

        let walker = WalkDir::new(root).max_depth(3).into_iter();
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        walkdir_into_items(walker, &mut items, &mut seen, 2, false);

        // hidden 파일은 제외되고, root(depth=0) 디렉터리는 제외됨
        assert!(!items.iter().any(|it| it.name.starts_with('.')));
        assert!(!items.iter().any(|it| it.path == root.to_string_lossy()));
        assert!(items.len() <= 2, "limit를 넘기면 안 됨");
    }

    #[test]
    fn walkdir_into_items_deduplicates_seen_paths() {
        let tmp = tempfile::tempdir().expect("tempdir 생성 실패");
        let root = tmp.path();
        let file = root.join("a.txt");
        must_write(&file, "a");

        let mut items = Vec::new();
        let mut seen = HashSet::new();

        let walker1 = WalkDir::new(root).max_depth(2).into_iter();
        walkdir_into_items(walker1, &mut items, &mut seen, 100, false);
        let len_after_first = items.len();

        let walker2 = WalkDir::new(root).max_depth(2).into_iter();
        walkdir_into_items(walker2, &mut items, &mut seen, 100, false);
        let len_after_second = items.len();

        assert_eq!(
            len_after_first, len_after_second,
            "seen dedup으로 중복 추가가 없어야 함"
        );
    }

    #[test]
    fn parse_line_output_into_sets_kind_and_respects_max() {
        let tmp = tempfile::tempdir().expect("tempdir 생성 실패");
        let root = tmp.path();
        let dir = root.join("folder");
        let file = root.join("report.txt");
        must_mkdir(&dir);
        must_write(&file, "r");

        let stdout =
            format!("{}\n{}\n\n", dir.to_string_lossy(), file.to_string_lossy()).into_bytes();

        let mut items = Vec::new();
        parse_line_output_into(&stdout, &mut items, 1, false);
        assert_eq!(items.len(), 1, "max=1이면 1개만 파싱되어야 함");

        let mut items = Vec::new();
        parse_line_output_into(&stdout, &mut items, 10, false);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, ItemKind::Directory);
        assert_eq!(items[1].kind, ItemKind::File);
    }

    #[test]
    fn is_ignored_dir_handles_hidden_dollar_and_patterns() {
        let tmp = tempfile::tempdir().expect("tempdir 생성 실패");
        let root = tmp.path();
        must_mkdir(&root.join(".git"));
        must_mkdir(&root.join("$Recycle.Bin"));
        must_mkdir(&root.join("node_modules"));
        must_mkdir(&root.join("keep"));

        let ignore_set: HashSet<&str> = ["node_modules"].into_iter().collect();
        let mut states = std::collections::HashMap::new();

        for entry in WalkDir::new(root)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .flatten()
        {
            let name = entry.file_name().to_string_lossy().to_string();
            states.insert(name, is_ignored_dir(&entry, &ignore_set));
        }

        assert_eq!(states.get(".git"), Some(&true));
        assert_eq!(states.get("$Recycle.Bin"), Some(&true));
        assert_eq!(states.get("node_modules"), Some(&true));
        assert_eq!(states.get("keep"), Some(&false));
    }
}
