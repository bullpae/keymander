//! 설정 기본값 함수 — serde `default = "..."` 와 Default 구현이 참조

use super::*;

pub(super) fn default_index_refresh_minutes() -> u64 {
    360 // 6시간 — 앱/PATH 변동 빈도 대비 충분히 신선하면서 재빌드 비용 최소
}

pub(super) fn default_multi_llm_providers() -> Vec<String> {
    vec![
        "chatgpt".to_string(),
        "gemini".to_string(),
        "claude".to_string(),
        "grok".to_string(),
        "perplexity".to_string(),
    ]
}

pub(super) fn default_multi_llm_prefixes() -> Vec<String> {
    vec![
        "@llm".to_string(),
        "@ll".to_string(),
        "@multi".to_string(),
        "@cmp".to_string(),
        "@compare".to_string(),
    ]
}

pub(super) fn default_multi_web_providers() -> Vec<String> {
    vec![
        "google".to_string(),
        "naver_search".to_string(),
        "daum".to_string(),
    ]
}

pub(super) fn default_multi_web_prefixes() -> Vec<String> {
    vec![
        "@m".to_string(),
        "@mw".to_string(),
        "@msearch".to_string(),
        "@multisearch".to_string(),
        "@searchall".to_string(),
        "@krsearch".to_string(),
    ]
}

pub(super) fn default_spell_providers() -> Vec<String> {
    vec!["naver_spell".to_string(), "pusan_spell".to_string()]
}

pub(super) fn default_spell_prefixes() -> Vec<String> {
    vec!["@sp".to_string(), "@spell".to_string()]
}

pub(super) fn default_translate_providers() -> Vec<String> {
    vec![
        "google_translate".to_string(),
        "papago".to_string(),
        "deepl".to_string(),
    ]
}

pub(super) fn default_translate_prefixes() -> Vec<String> {
    vec!["@tr".to_string(), "@trko".to_string(), "@tren".to_string()]
}

/// Platform-specific default search paths (user directories).
/// `launcher.include_user_folders`가 켜져 있으면 `search_paths`에 더해진다 (docs/18).
///
/// **OS가 알려주는 실제 위치**를 먼저 쓴다. 예전엔 홈 아래 고정 이름
/// (`Desktop`·`Documents`·`Downloads`)만 봐서 실제 위치가 다르면 놓쳤다:
/// - Windows: OneDrive 백업을 켜면 바탕 화면·문서가 `OneDrive\바탕 화면` 등으로
///   옮겨진다(Known Folder 리디렉션). 회사 OneDrive 폴더는 `OneDrive - 회사명`이라
///   이름 `OneDrive`와도 맞지 않는다
/// - Linux: 지역화된 사용자 폴더(`~/바탕화면`)는 `user-dirs.dirs`에 적혀 있다
///
/// 옛 고정 이름 후보도 그대로 둔다 — 결과는 이전 동작의 상위 집합이라 기존
/// 사용자에게서 빠지는 폴더가 없다. 존재하는 폴더만, 중복 없이.
pub(crate) fn default_search_paths() -> Vec<PathBuf> {
    let mut candidates: Vec<Option<PathBuf>> = vec![
        dirs::desktop_dir(),
        dirs::document_dir(),
        dirs::download_dir(),
    ];

    if let Some(home) = dirs::home_dir() {
        for name in ["Desktop", "Documents", "Downloads"] {
            candidates.push(Some(home.join(name)));
        }
        #[cfg(target_os = "windows")]
        candidates.push(Some(home.join("OneDrive")));
    }

    // Windows가 설정하는 OneDrive 루트들 (개인·회사)
    #[cfg(target_os = "windows")]
    for key in ["OneDrive", "OneDriveCommercial", "OneDriveConsumer"] {
        candidates.push(std::env::var_os(key).map(PathBuf::from));
    }

    #[cfg(target_os = "linux")]
    for key in ["XDG_DESKTOP_DIR", "XDG_DOCUMENTS_DIR", "XDG_DOWNLOAD_DIR"] {
        candidates.push(std::env::var_os(key).map(PathBuf::from));
    }

    existing_unique_dirs(candidates)
}

/// 존재하는 폴더만 남기고 중복을 없앤다 (순서 유지). Windows는 대소문자 무시.
pub(super) fn existing_unique_dirs(
    candidates: impl IntoIterator<Item = Option<PathBuf>>,
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for dir in candidates.into_iter().flatten() {
        if !dir.is_dir() {
            continue;
        }
        let dup = out.iter().any(|seen| {
            crate::fsutil::path_within(seen, &dir) && crate::fsutil::path_within(&dir, seen)
        });
        if !dup {
            out.push(dir);
        }
    }
    out
}

/// 기본 제외 폴더 이름 — `use_default_ignores`가 켜져 있으면 `ignore_patterns`에 더해진다.
pub const DEFAULT_IGNORE_PATTERNS: &[&str] = &[
    // Version control
    ".git",
    ".svn",
    // Build artifacts / package caches
    "node_modules",
    "target",
    "__pycache__",
    ".tox",
    "dist",
    // Rust / Cargo
    ".cargo",
    ".rustup",
    // Node / npm / yarn / pnpm
    ".npm",
    ".yarn",
    ".pnpm-store",
    // .NET / NuGet
    ".nuget",
    // Java / Gradle / Maven
    ".gradle",
    ".m2",
    // Go — ~/go module cache
    "go",
    // General caches
    ".cache",
    ".local",
    ".tmp",
    // IDE / editor state
    ".vscode",
    ".cursor",
    ".idea",
    ".eclipse",
    // Windows specific — user profile
    "AppData",
    "$Recycle.Bin",
    "NTUSER.DAT",
    // Windows specific — system directories (drive root)
    "Windows",
    "Program Files",
    "Program Files (x86)",
    "ProgramData",
    "PerfLogs",
    "Recovery",
    "System Volume Information",
    "inetpub",
    // macOS specific
    "Library",
    ".Trash",
];
