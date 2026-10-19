//! Index system — discovers and caches applications, files, and commands.

pub mod apps;
pub mod files;
pub mod path;
pub mod store;
pub mod system_commands;

use serde::{Deserialize, Serialize};

/// A single indexed item (app, file, executable, system command, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexItem {
    /// Display name (e.g. "Firefox", "report.pdf")
    pub name: String,
    /// Full path or command value
    pub path: String,
    /// Item kind
    pub kind: ItemKind,
    /// Where this item came from
    pub source: Source,
    /// Icon (emoji)
    pub icon: String,
    /// Search keywords (joined subtitle/keywords for matching)
    pub keywords: String,
    /// OS-specific icon path (Linux: Icon= from .desktop, macOS: derived at runtime)
    #[serde(default)]
    pub icon_path: Option<String>,
}

/// Item classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemKind {
    App,
    File,
    Executable,
    SystemCommand,
    WebSearch,
    Directory,
    Calculator,
    Emoji,
    Shell,
}

impl ItemKind {
    /// 실행 이력(history.item_type)에 저장하는 정규 태그.
    ///
    /// 예전엔 UI마다 표기가 달라(TUI/CLI `"App"`·`"Dir"`, 데스크톱 `"app"`·`"dir"`·`"misc"`,
    /// CLI 경로 실행 `"file"`) 같은 경로가 유니크 키 `(item_type, value)`에서 두 행으로
    /// 갈리고 frecency가 낮게 잡혔다. 기록은 반드시 이 태그로 한다(db v4가 기존 행 정규화).
    pub fn history_tag(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::File => "file",
            Self::Executable => "exe",
            Self::SystemCommand => "system",
            Self::WebSearch => "web",
            Self::Directory => "dir",
            Self::Calculator => "calc",
            Self::Emoji => "emoji",
            Self::Shell => "shell",
        }
    }

    /// 이력 태그 → ItemKind. 대소문자와 예전 표기(`Directory`, `Exe` 등)를 허용하고,
    /// 모르는 태그(예전 데스크톱의 `misc`)는 None.
    pub fn from_history_tag(tag: &str) -> Option<Self> {
        Some(match tag.to_ascii_lowercase().as_str() {
            "app" => Self::App,
            "file" => Self::File,
            "exe" | "executable" => Self::Executable,
            "system" => Self::SystemCommand,
            "web" => Self::WebSearch,
            "dir" | "directory" => Self::Directory,
            "calc" => Self::Calculator,
            "emoji" => Self::Emoji,
            "shell" => Self::Shell,
            _ => return None,
        })
    }
}

impl std::fmt::Display for ItemKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::App => write!(f, "App"),
            Self::File => write!(f, "File"),
            Self::Executable => write!(f, "Exe"),
            Self::SystemCommand => write!(f, "System"),
            Self::WebSearch => write!(f, "Web"),
            Self::Directory => write!(f, "Dir"),
            Self::Calculator => write!(f, "Calc"),
            Self::Emoji => write!(f, "Emoji"),
            Self::Shell => write!(f, "Shell"),
        }
    }
}

/// Where the item was discovered
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    /// OS application directories
    Apps,
    /// PATH executables
    Path,
    /// File search provider (fd, Everything, etc.)
    FileProvider,
    /// Built-in system commands
    SystemCommand,
    /// Plugin-provided
    Plugin,
}

/// The full index: a collection of items with metadata
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Index {
    pub items: Vec<IndexItem>,
    /// ISO 8601 timestamp of last rebuild
    pub last_updated: Option<String>,
    /// Version marker — cache is invalidated when this doesn't match current version
    #[serde(default)]
    pub version: String,
}

impl Index {
    /// Create a new empty index
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the full index from all sources.
    ///
    /// Strategy: "priority directories first"
    /// 1. PATH executables, system commands, OS apps (always)
    /// 2. Priority directories (Desktop, Documents, Downloads) — guaranteed
    /// 3. General file provider scan — fills remaining quota
    /// 4. Deduplicate by path
    pub fn build(config: &crate::config::LauncherConfig, use_emoji: bool) -> Self {
        Self::build_inner(config, use_emoji, true)
    }

    /// Lightweight build: PATH + system commands only (no apps, no files).
    /// Used for instant boot — the full index replaces this shortly after.
    pub fn build_quick(use_emoji: bool) -> Self {
        let mut items = Vec::new();

        let executables = path::collect_executables(use_emoji);
        tracing::info!("PATH executables: {} items", executables.len());
        items.extend(executables);

        let sys_cmds = system_commands::collect_system_commands(use_emoji);
        tracing::info!("System commands: {} items", sys_cmds.len());
        items.extend(sys_cmds);

        tracing::info!("Quick index: {} items total", items.len());

        Self {
            items,
            last_updated: Some(chrono_now()),
            version: Self::current_version().to_string(),
        }
    }

    fn build_inner(
        config: &crate::config::LauncherConfig,
        use_emoji: bool,
        include_apps: bool,
    ) -> Self {
        use std::collections::HashSet;

        let mut items = Vec::new();

        // 1. PATH executables (always included)
        let executables = path::collect_executables(use_emoji);
        tracing::info!("PATH executables: {} items", executables.len());
        items.extend(executables);

        // 2. System commands (always included)
        let sys_cmds = system_commands::collect_system_commands(use_emoji);
        tracing::info!("System commands: {} items", sys_cmds.len());
        items.extend(sys_cmds);

        // 3. OS applications (skipped for quick boot)
        if include_apps {
            let apps = apps::collect_apps(use_emoji);
            tracing::info!("OS applications: {} items", apps.len());
            items.extend(apps);
        }

        // 4. File search: priority directories FIRST
        let provider_config = files::ProviderConfig {
            max_results: config.max_results,
            search_depth: config.search_depth,
            search_paths: config.effective_search_paths(),
            ignore_patterns: config.effective_ignore_patterns(),
            ignore_extensions: config.ignored_extension_set(),
            everything_path: config.everything_path.clone(),
            scan_drives: config.scan_drives,
            drive_scan_depth: config.drive_scan_depth,
            use_emoji,
        };

        let priority_files = files::collect_priority_files(&provider_config);
        let priority_count = priority_files.len();
        tracing::info!("Priority directory files: {} items", priority_count);
        items.extend(priority_files);

        // 5. General file provider scan (fills remaining quota)
        let provider_kind = files::detect_provider(
            &config.file_search_provider,
            config.everything_path.as_ref(),
        );
        tracing::info!("Detected file provider: {}", provider_kind);

        let general_files = files::collect_files(provider_kind, &provider_config, priority_count);
        tracing::info!("General file scan: {} items", general_files.len());
        items.extend(general_files);

        // 6. Deduplicate by path (preserve order — priority files come first)
        //    + 제외 확장자 — walkdir 스캔은 순회 중에 거르지만 fd·Everything 등
        //    외부 프로바이더 결과는 여기서 걸러야 한다. 파일 프로바이더 결과만 대상
        //    (앱·PATH 실행 파일은 사용자가 고른 스캔 범위가 아니다), 폴더는 제외.
        let mut seen_paths = HashSet::new();
        let ignore_ext = &provider_config.ignore_extensions;
        items.retain(|item| {
            let ignored = item.source == Source::FileProvider
                && item.kind != ItemKind::Directory
                && files::has_ignored_extension(std::path::Path::new(&item.path), ignore_ext);
            !ignored && seen_paths.insert(item.path.clone())
        });

        tracing::info!("Total index items after dedup: {}", items.len());

        let now = chrono_now();

        Self {
            items,
            last_updated: Some(now),
            version: Self::current_version().to_string(),
        }
    }

    /// Current index schema/cache version.
    ///
    /// Keep this independent from product SemVer so patch/minor releases
    /// do not force unnecessary full reindexing on first launch.
    /// Bump this only when index format/compatibility changes.
    pub fn current_version() -> &'static str {
        "schema-3"
    }

    /// Total number of indexed items
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the index is empty
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Simple UTC timestamp without chrono dependency
fn chrono_now() -> String {
    // Use std::time for a basic timestamp
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Simple ISO-like format: just the unix timestamp for now
    // In production, we'd format this properly
    format!("{}", secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_new_is_empty() {
        let idx = Index::new();
        assert!(idx.is_empty());
        assert_eq!(idx.len(), 0);
    }

    #[test]
    fn test_item_kind_display() {
        assert_eq!(format!("{}", ItemKind::App), "App");
        assert_eq!(format!("{}", ItemKind::File), "File");
        assert_eq!(format!("{}", ItemKind::SystemCommand), "System");
    }

    #[test]
    fn test_build_quick_excludes_apps() {
        let idx = Index::build_quick(false);
        let has_apps = idx.items.iter().any(|i| i.source == Source::Apps);
        assert!(
            !has_apps,
            "quick 인덱스에 Apps 소스가 포함되면 부팅이 느려짐 (PowerShell 호출)"
        );
    }

    #[test]
    fn test_build_quick_excludes_file_provider() {
        let idx = Index::build_quick(false);
        let has_files = idx.items.iter().any(|i| i.source == Source::FileProvider);
        assert!(
            !has_files,
            "quick 인덱스에 FileProvider 소스가 포함되면 부팅이 느려짐"
        );
    }

    #[test]
    fn test_build_quick_contains_only_fast_sources() {
        let idx = Index::build_quick(false);
        assert!(!idx.is_empty(), "quick 인덱스가 비어 있으면 안됨");
        for item in &idx.items {
            assert!(
                item.source == Source::Path || item.source == Source::SystemCommand,
                "quick 인덱스에 허용되지 않는 소스: {:?} (item: {})",
                item.source,
                item.name
            );
        }
    }
}
