//! TOML-based configuration management
//!
//! Launcher-focused config with keybindings and provider settings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Top-level configuration
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub launcher: LauncherConfig,
    pub keybindings: KeybindingsConfig,
    pub clipboard: ClipboardConfig,

    /// Config file path (excluded from serialization)
    #[serde(skip)]
    pub config_path: Option<PathBuf>,
}

/// 클립보드 히스토리 설정 (docs/12). 데몬이 시스템 클립보드를 감시해 링 버퍼에
/// 쌓고, `clip:N` 레이어 바인딩이 n번째 최근 항목을 붙여넣는다.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ClipboardConfig {
    /// 히스토리 수집 활성화. 기본 false(opt-in) — 비밀번호 관리자의 Concealed
    /// 마크 제외가 아직 없어(P1.1), 사용자가 명시적으로 켜야 한다.
    pub history_enabled: bool,
    /// 링 버퍼 상한. 초과 시 오래된 것부터 밀려난다.
    pub history_size: usize,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            history_enabled: false,
            history_size: 50,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// TUI render FPS
    pub render_fps: u64,
    /// Show preview panel
    pub show_preview: bool,
    /// Preview panel width percentage
    pub preview_width_percent: u16,
    /// Color theme name
    pub theme: String,
    /// External editor command
    pub editor: Option<String>,
    /// Use emoji icons (true = emoji, false = ASCII 2-char icons)
    /// Set to false for legacy terminals (conhost/cmd.exe)
    pub emoji_icons: bool,
    /// Reset input method to English when the desktop launcher opens.
    /// Useful because most commands start with English characters (@, :, etc.).
    pub reset_ime_on_launch: bool,
    /// Desktop launcher base font size in pixels.
    /// All UI elements scale proportionally. Range: 12–32, default: 16.
    pub font_size: f32,
    /// Desktop launcher visible result rows. Range: 4–20, default: 8.
    pub visible_rows: usize,
    /// Desktop launcher renderer: "auto" (GPU → software fallback),
    /// "software" (tiny-skia — VM/원격 데스크톱처럼 GPU가 부실한 환경에서
    /// 어댑터 프로빙을 생략해 부팅이 빨라진다), "gpu" (wgpu 강제).
    pub renderer: String,
    /// Desktop launcher brand icon style: "color" (official full-color logos)
    /// or "mono" (theme-tinted monochrome glyphs — unified look).
    pub brand_icons: String,
    /// Desktop window transparency: "auto" or "off".
    ///
    /// `auto` — 투명 창을 쓴다. 창 높이를 고정하고 빈 영역을 투명으로 두므로
    /// 결과가 생기고 사라질 때 창 리사이즈가 없다(= 화면 찢어짐 없음).
    /// Windows 는 DirectComposition 스왑체인으로 알파를 합성한다.
    /// `off` — 불투명 창 + 결과에 맞춘 창 리사이즈(구버전 동작). 투명 합성이
    /// 안 되는 환경에서 빈 영역이 검게 보이면 이 값으로 되돌린다.
    pub window_transparency: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            render_fps: 30,
            show_preview: true,
            preview_width_percent: 40,
            theme: "default".to_string(),
            editor: None,
            emoji_icons: true,
            reset_ime_on_launch: true,
            font_size: 16.0,
            visible_rows: 8,
            renderer: "auto".to_string(),
            brand_icons: "color".to_string(),
            window_transparency: "auto".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LauncherConfig {
    /// File search provider: auto | builtin | fd | everything | mdfind | locate | winfs
    pub file_search_provider: String,
    /// Everything CLI (es.exe) path (Windows). Empty = auto-detect.
    pub everything_path: Option<PathBuf>,
    /// Directories to search for files
    pub search_paths: Vec<PathBuf>,
    /// Maximum search results from file providers
    pub max_results: usize,
    /// Maximum recursive directory depth for file scanning (default: 4)
    pub search_depth: usize,
    /// Patterns to ignore during file indexing
    pub ignore_patterns: Vec<String>,
    /// Quit kmd after launching a program/file
    pub quit_on_launch: bool,
    /// Whether to index directories (not just files)
    pub index_directories: bool,
    /// Auto-scan available drive roots (Windows: C:\~Z:\, macOS/Linux: /)
    pub scan_drives: bool,
    /// Max depth when scanning drive roots (shallow to avoid system dirs)
    pub drive_scan_depth: usize,
    /// 데몬 백그라운드 인덱스 리프레시 주기 (분). 0 = 비활성.
    /// 데몬이 이 주기로 인덱스를 재빌드해 공유 캐시를 갱신하므로,
    /// kmd-desktop은 실행 시 항상 신선한 캐시를 즉시 로드한다.
    #[serde(default = "default_index_refresh_minutes")]
    pub index_refresh_minutes: u64,
    /// Search result priority weights by item kind (0-100, higher = boosted)
    pub kind_weights: KindWeights,
    /// Custom web services
    #[serde(default)]
    pub web_services: Vec<CustomWebService>,
    /// LLM IDs to open for `@llm` multi-prompt compare.
    /// Supported: chatgpt, gemini, claude, grok, perplexity
    #[serde(default = "default_multi_llm_providers")]
    pub multi_llm_providers: Vec<String>,
    /// LLM 오토파일럿 — URL로 자동 실행이 안 되는 서비스(chatgpt/claude/gemini)에
    /// 데몬이 전경창 검증 후 키(Enter/Ctrl+V)를 주입해 자동 제출한다.
    /// 기본 off (자동 키 주입은 opt-in). 데몬 실행 + Windows에서만 동작 (docs/09).
    #[serde(default)]
    pub llm_autopilot: bool,
    /// Command aliases for multi-LLM compare.
    /// Example: @llm, @ll, @cmp
    #[serde(default = "default_multi_llm_prefixes")]
    pub multi_llm_prefixes: Vec<String>,
    /// Search engine IDs for `@msearch` multi-web search.
    /// Supported: google, naver_search, daum
    #[serde(default = "default_multi_web_providers")]
    pub multi_web_providers: Vec<String>,
    /// Command aliases for multi-web search.
    /// Example: @m, @mw, @msearch
    #[serde(default = "default_multi_web_prefixes")]
    pub multi_web_prefixes: Vec<String>,
    /// Providers used by spelling check command.
    /// Supported: naver_spell, pusan_spell
    #[serde(default = "default_spell_providers")]
    pub spell_providers: Vec<String>,
    /// Command aliases for spelling check.
    /// Example: @sp, @spell
    #[serde(default = "default_spell_prefixes")]
    pub spell_prefixes: Vec<String>,
    /// Providers used by translate command.
    /// Supported: google_translate, papago, deepl
    #[serde(default = "default_translate_providers")]
    pub translate_providers: Vec<String>,
    /// Command aliases for translate.
    /// Example: @tr, @trko, @tren
    #[serde(default = "default_translate_prefixes")]
    pub translate_prefixes: Vec<String>,
    /// Keymap integration settings (prototype).
    #[serde(default)]
    pub keymap: KeymapConfig,
    /// 저장된 프롬프트 템플릿 목록 (`@ll :name query` 형태로 사용).
    #[serde(default)]
    pub prompt_templates: Vec<PromptTemplate>,
    /// 문서 본문 검색(FTS5) 설정 (docs/15).
    #[serde(default)]
    pub content_search: ContentSearchConfig,
}

/// 문서 본문 검색(FTS5) 설정 — `[launcher.content_search]` (docs/15).
///
/// 스캔 범위는 `launcher.search_paths` + `ignore_patterns` + `search_depth`를
/// 그대로 공유하고, 이 섹션은 본문 인덱싱 고유의 노브만 갖는다.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ContentSearchConfig {
    /// 본문 인덱싱 활성화 여부 (기본 true)
    pub enabled: bool,
    /// 파일당 최대 크기 (KB, 기본 1024). 초과 파일은 본문 인덱싱에서 제외.
    pub max_file_kb: u64,
    /// 인덱싱할 확장자 목록. 비어 있으면 내장 기본 목록(플레인 텍스트·소스)을
    /// 사용하고, 지정하면 통째로 대체한다. 점 없이 소문자 (예: ["md", "txt"]).
    pub extensions: Vec<String>,
    /// 본문 인덱스 대상 파일 수 상한 (기본 20000). 초과분은 건너뛰고 로그만 남긴다.
    pub max_files: usize,
    /// 파일명에 포함되면 인덱싱에서 제외하는 마커 (소문자 부분일치).
    /// 비어 있으면 내장 기본 목록(secret/credential/password/passwd)을 사용하고,
    /// 지정하면 통째로 대체한다. 시크릿성 파일 본문이 인덱스 DB에 복제되는
    /// 것을 막는 안전장치다.
    pub exclude_names: Vec<String>,
}

impl Default for ContentSearchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_file_kb: 1024,
            extensions: Vec::new(),
            max_files: 20_000,
            exclude_names: Vec::new(),
        }
    }
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            file_search_provider: "auto".to_string(),
            everything_path: None,
            search_paths: default_search_paths(),
            // Conservative defaults to reduce cold-start indexing cost.
            max_results: 5000,
            search_depth: 4,
            ignore_patterns: vec![
                // Version control
                ".git".to_string(),
                ".svn".to_string(),
                // Build artifacts / package caches
                "node_modules".to_string(),
                "target".to_string(),
                "__pycache__".to_string(),
                ".tox".to_string(),
                "dist".to_string(),
                // Rust / Cargo
                ".cargo".to_string(),
                ".rustup".to_string(),
                // Node / npm / yarn / pnpm
                ".npm".to_string(),
                ".yarn".to_string(),
                ".pnpm-store".to_string(),
                // .NET / NuGet
                ".nuget".to_string(),
                // Java / Gradle / Maven
                ".gradle".to_string(),
                ".m2".to_string(),
                // Go
                "go".to_string(), // ~/go module cache
                // General caches
                ".cache".to_string(),
                ".local".to_string(),
                ".tmp".to_string(),
                // IDE / editor state
                ".vscode".to_string(),
                ".cursor".to_string(),
                ".idea".to_string(),
                ".eclipse".to_string(),
                // Windows specific — user profile
                "AppData".to_string(),
                "$Recycle.Bin".to_string(),
                "NTUSER.DAT".to_string(),
                // Windows specific — system directories (C:\ root)
                "Windows".to_string(),
                "Program Files".to_string(),
                "Program Files (x86)".to_string(),
                "ProgramData".to_string(),
                "PerfLogs".to_string(),
                "Recovery".to_string(),
                "System Volume Information".to_string(),
                "inetpub".to_string(),
                // macOS specific
                "Library".to_string(),
                ".Trash".to_string(),
            ],
            quit_on_launch: true,
            index_directories: true,
            scan_drives: false,
            drive_scan_depth: 2,
            index_refresh_minutes: default_index_refresh_minutes(),
            kind_weights: KindWeights::default(),
            web_services: vec![],
            multi_llm_providers: default_multi_llm_providers(),
            multi_llm_prefixes: default_multi_llm_prefixes(),
            llm_autopilot: false,
            multi_web_providers: default_multi_web_providers(),
            multi_web_prefixes: default_multi_web_prefixes(),
            spell_providers: default_spell_providers(),
            spell_prefixes: default_spell_prefixes(),
            translate_providers: default_translate_providers(),
            translate_prefixes: default_translate_prefixes(),
            keymap: KeymapConfig::default(),
            prompt_templates: vec![],
            content_search: ContentSearchConfig::default(),
        }
    }
}

fn default_index_refresh_minutes() -> u64 {
    360 // 6시간 — 앱/PATH 변동 빈도 대비 충분히 신선하면서 재빌드 비용 최소
}

fn default_multi_llm_providers() -> Vec<String> {
    vec![
        "chatgpt".to_string(),
        "gemini".to_string(),
        "claude".to_string(),
        "grok".to_string(),
        "perplexity".to_string(),
    ]
}

fn default_multi_llm_prefixes() -> Vec<String> {
    vec![
        "@llm".to_string(),
        "@ll".to_string(),
        "@multi".to_string(),
        "@cmp".to_string(),
        "@compare".to_string(),
    ]
}

fn default_multi_web_providers() -> Vec<String> {
    vec![
        "google".to_string(),
        "naver_search".to_string(),
        "daum".to_string(),
    ]
}

fn default_multi_web_prefixes() -> Vec<String> {
    vec![
        "@m".to_string(),
        "@mw".to_string(),
        "@msearch".to_string(),
        "@multisearch".to_string(),
        "@searchall".to_string(),
        "@krsearch".to_string(),
    ]
}

fn default_spell_providers() -> Vec<String> {
    vec!["naver_spell".to_string(), "pusan_spell".to_string()]
}

fn default_spell_prefixes() -> Vec<String> {
    vec!["@sp".to_string(), "@spell".to_string()]
}

fn default_translate_providers() -> Vec<String> {
    vec![
        "google_translate".to_string(),
        "papago".to_string(),
        "deepl".to_string(),
    ]
}

fn default_translate_prefixes() -> Vec<String> {
    vec!["@tr".to_string(), "@trko".to_string(), "@tren".to_string()]
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct KeymapConfig {
    /// Backend type. Current prototype supports `kanata`.
    pub backend: String,
    /// Path to kanata executable. Empty = PATH lookup.
    pub kanata_path: Option<PathBuf>,
    /// Directory containing keymap profile files.
    pub profile_dir: Option<PathBuf>,
    /// Active profile: "vim-nav" | "minimal" | "none" | "custom"
    /// "custom"이면 아래 remaps/layers/combos/double_taps 사용
    pub active_profile: String,

    /// 단순 리맵 (항상 활성). 키이름 = "대상키" 또는 "Ctrl+키"
    #[serde(default)]
    pub remaps: HashMap<String, String>,
    /// 레이어 정의. 이름 = { trigger, mappings, ... }
    #[serde(default)]
    pub layers: HashMap<String, LayerToml>,
    /// 수정자+키 콤보 (예: Shift+Space → Hangul)
    #[serde(default)]
    pub combos: Vec<ComboToml>,
    /// 글로벌 더블탭
    #[serde(default)]
    pub double_taps: Vec<DoubleTapToml>,
    /// tap-hold(모드탭): 키이름 = { tap, hold, timeout_ms }
    /// 짧게 탭 = tap 키, 홀드 중 다른 키 = hold 수정자 조합 (HHKB 스타일)
    #[serde(default)]
    pub tap_holds: HashMap<String, TapHoldToml>,
}

impl Default for KeymapConfig {
    fn default() -> Self {
        Self {
            backend: "kanata".to_string(),
            kanata_path: None,
            profile_dir: None,
            active_profile: "vim-nav".to_string(),
            remaps: HashMap::new(),
            layers: HashMap::new(),
            combos: Vec::new(),
            double_taps: Vec::new(),
            tap_holds: HashMap::new(),
        }
    }
}

/// TOML tap-hold(모드탭) 설정.
/// 짧게 탭하면 `tap` 키, 홀드 중 다른 키를 누르면 `hold` 수정자로 동작한다.
/// 예: CapsLock = { tap = "CapsLock", hold = "LCtrl" } → HHKB 스타일 캡스락
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TapHoldToml {
    /// 짧게 탭했을 때 보낼 키 (예: "CapsLock", "Escape"). 생략 시 탭 무동작
    pub tap: Option<String>,
    /// 홀드 중 다른 키와 조합할 수정자 키 (예: "LCtrl")
    pub hold: String,
    /// tap-hold 판정 시간 (ms, 기본 200)
    pub timeout_ms: Option<u32>,
}

/// TOML 레이어 설정
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct LayerToml {
    /// 레이어 활성화 트리거 키 (예: "LAlt")
    pub trigger: String,
    /// 짧게 탭했을 때 보낼 키 (예: "Escape")
    pub tap_action: Option<String>,
    /// tap-hold 판정 시간 (ms, 기본 200)
    pub tap_hold_ms: Option<u32>,
    /// 미매핑 키 동작: "plain"(기본, 맨키 통과) | "passthrough"(트리거 조합
    /// 그대로 OS로, 예: Alt+Tab 유지) | "block"(억제)
    pub unmapped: Option<String>,
    /// 레이어 내 키 매핑: 키이름 = "대상" (예: H = "Left", P = "Ctrl+V")
    pub mappings: HashMap<String, String>,
    /// 레이어 내 더블탭: 키이름 = { single, double, timeout_ms }
    pub double_taps: HashMap<String, LayerDoubleTapToml>,
}

/// TOML 레이어 내 더블탭 설정
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LayerDoubleTapToml {
    /// 한 번 탭 시 액션 (예: "Ctrl+Left")
    pub single: String,
    /// 더블탭 시 액션 (예: "Home")
    pub double: String,
    /// 타임아웃 (ms, 기본 300)
    pub timeout_ms: Option<u32>,
}

/// TOML 콤보 설정
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ComboToml {
    /// 트리거 (예: "Shift+Space")
    pub trigger: String,
    /// 실행 액션 (예: "Hangul")
    pub action: String,
}

/// TOML 글로벌 더블탭 설정
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DoubleTapToml {
    /// 키 (예: "RShift")
    pub key: String,
    /// 액션 (예: "Hangul")
    pub action: String,
    /// 타임아웃 (ms, 기본 300)
    pub timeout_ms: Option<u32>,
}

/// 재사용 가능한 프롬프트 템플릿. `@ll :name question` 형태로 LLM에 전달.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PromptTemplate {
    /// 고유 이름 (영문/숫자/하이픈, 예: code-review)
    pub name: String,
    /// 본문 — `{query}` 자리표시자가 있으면 실제 질문으로 치환.
    /// 없으면 `body + "\n\n" + query` 형태로 결합.
    pub body: String,
}

/// Platform-specific default search paths (user directories).
/// These become the default for `launcher.search_paths`, editable in settings.
fn default_search_paths() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(target_os = "windows")]
    {
        if let Ok(profile) = std::env::var("USERPROFILE") {
            let base = PathBuf::from(&profile);
            for name in &["Desktop", "Documents", "Downloads", "OneDrive"] {
                let dir = base.join(name);
                if dir.is_dir() {
                    dirs.push(dir);
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            for name in &["Desktop", "Documents", "Downloads"] {
                let dir = home.join(name);
                if dir.is_dir() {
                    dirs.push(dir);
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(home) = dirs::home_dir() {
            for name in &["Desktop", "Documents", "Downloads"] {
                let dir = home.join(name);
                if dir.is_dir() {
                    dirs.push(dir);
                }
            }
        }
        for env_key in &["XDG_DESKTOP_DIR", "XDG_DOCUMENTS_DIR", "XDG_DOWNLOAD_DIR"] {
            if let Ok(val) = std::env::var(env_key) {
                let dir = PathBuf::from(val);
                if dir.is_dir() && !dirs.contains(&dir) {
                    dirs.push(dir);
                }
            }
        }
    }

    dirs
}

/// Search result priority weights per item kind.
/// Higher values push results toward the top (0-100 range recommended).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct KindWeights {
    pub directory: u32,
    pub app: u32,
    pub file: u32,
    pub executable: u32,
    pub system_cmd: u32,
    pub web_search: u32,
}

impl Default for KindWeights {
    fn default() -> Self {
        Self {
            directory: 80,
            app: 70,
            file: 50,
            executable: 40,
            system_cmd: 30,
            web_search: 20,
        }
    }
}

impl KindWeights {
    /// Get the weight for a given ItemKind
    pub fn weight_for(&self, kind: crate::index::ItemKind) -> u32 {
        use crate::index::ItemKind;
        match kind {
            ItemKind::Directory => self.directory,
            ItemKind::App => self.app,
            ItemKind::File => self.file,
            ItemKind::Executable => self.executable,
            ItemKind::SystemCommand => self.system_cmd,
            ItemKind::WebSearch => self.web_search,
            ItemKind::Calculator => 0, // handled separately
            ItemKind::Emoji => 0,      // handled separately
            ItemKind::Shell => 0,      // handled separately
        }
    }
}

/// User-defined custom web service
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CustomWebService {
    pub name: String,
    pub prefixes: Vec<String>,
    pub icon: String,
    pub url_template: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct KeybindingsConfig {
    /// Global hotkey for daemon mode
    pub global_hotkey: String,
    /// Toggle daemon keymap processing on/off while keeping this hotkey alive.
    pub toggle_keymap: String,
    pub quit: String,
    pub next: String,
    pub prev: String,
    pub select: String,
    pub toggle_preview: String,
}

impl Default for KeybindingsConfig {
    fn default() -> Self {
        Self {
            global_hotkey: String::new(),
            toggle_keymap: "ctrl+alt+k".to_string(),
            quit: "ctrl+c".to_string(),
            next: "down".to_string(),
            prev: "up".to_string(),
            select: "enter".to_string(),
            toggle_preview: "ctrl+p".to_string(),
        }
    }
}

// ── 단순 설정 키 레지스트리 ──────────────────────────────────────────────────
//
// 설정 키는 오랫동안 `get_value`와 `set_value` 두 개의 거대한 match에 **각각**
// 손으로 적혀 있었다. 둘을 잇는 컴파일 타임 장치가 없어서 한쪽만 적는 실수가
// 조용히 통과했다 — 실제로 `launcher.everything_path`가 설정 화면에만 있고
// 양쪽에 다 없어서 "입력해도 버려지는" 버그가 있었다 (2026-08-27).
//
// 아래 매크로는 **한 줄 선언에서 get/set 양쪽을 함께 생성**한다. 여기 적힌 키는
// 비대칭이 원천적으로 불가능하다. 리스트 정규화나 읽기 전용 의사 키처럼 처리가
// 특별한 것은 여전히 `get_value`/`set_value`의 명시적 arm으로 남는다.
//
// 종류:
//   num  — `FromStr` + `Display` (숫자·불리언). 파싱 실패는 오류로 전파
//   text — `String` 필드. 임의 문자열을 그대로 받는다
macro_rules! config_registry {
    ( $( $key:literal => $($field:ident).+ , $kind:ident ; )* ) => {
        impl Config {
            /// 레지스트리 키면 값을, 아니면 `None`.
            fn registry_get(&self, key: &str) -> Option<String> {
                match key {
                    $( $key => Some(registry_read!($kind, self.$($field).+)), )*
                    _ => None,
                }
            }

            /// 레지스트리 키면 설정하고 `Some(())`, 아니면 `None`.
            fn registry_set(&mut self, key: &str, value: &str) -> Result<Option<()>, ConfigError> {
                match key {
                    $(
                        $key => {
                            registry_write!($kind, self.$($field).+, key, value);
                            Ok(Some(()))
                        }
                    )*
                    _ => Ok(None),
                }
            }

            /// 레지스트리에 선언된 키 전체 (테스트·문서용).
            pub fn registry_keys() -> &'static [&'static str] {
                &[ $($key),* ]
            }
        }
    };
}

macro_rules! registry_read {
    (num, $f:expr) => {
        $f.to_string()
    };
    (text, $f:expr) => {
        $f.clone()
    };
}

macro_rules! registry_write {
    (num, $f:expr, $key:expr, $value:expr) => {
        $f = $value.parse().map_err(|_| ConfigError::InvalidValue {
            key: $key.to_string(),
            value: $value.to_string(),
            expected: std::any::type_name_of_val(&$f),
        })?
    };
    (text, $f:expr, $key:expr, $value:expr) => {
        $f = $value.to_string()
    };
}

config_registry! {
    "general.render_fps"               => general.render_fps, num;
    "general.show_preview"             => general.show_preview, num;
    "general.preview_width_percent"    => general.preview_width_percent, num;
    "general.theme"                    => general.theme, text;
    "general.emoji_icons"              => general.emoji_icons, num;
    "general.reset_ime_on_launch"      => general.reset_ime_on_launch, num;
    "launcher.file_search_provider"    => launcher.file_search_provider, text;
    "launcher.max_results"             => launcher.max_results, num;
    "launcher.search_depth"            => launcher.search_depth, num;
    "launcher.quit_on_launch"          => launcher.quit_on_launch, num;
    "launcher.index_directories"       => launcher.index_directories, num;
    "launcher.scan_drives"             => launcher.scan_drives, num;
    "launcher.drive_scan_depth"        => launcher.drive_scan_depth, num;
    "launcher.index_refresh_minutes"   => launcher.index_refresh_minutes, num;
    "launcher.kind_weights.directory"  => launcher.kind_weights.directory, num;
    "launcher.kind_weights.app"        => launcher.kind_weights.app, num;
    "launcher.kind_weights.file"       => launcher.kind_weights.file, num;
    "launcher.kind_weights.executable" => launcher.kind_weights.executable, num;
    "launcher.kind_weights.system_cmd" => launcher.kind_weights.system_cmd, num;
    "launcher.kind_weights.web_search" => launcher.kind_weights.web_search, num;
    "keybindings.global_hotkey"        => keybindings.global_hotkey, text;
    "keybindings.toggle_keymap"        => keybindings.toggle_keymap, text;
    "keybindings.quit"                 => keybindings.quit, text;
    "keybindings.next"                 => keybindings.next, text;
    "keybindings.prev"                 => keybindings.prev, text;
    "keybindings.select"               => keybindings.select, text;
    "keybindings.toggle_preview"       => keybindings.toggle_preview, text;
}

impl Config {
    /// Load config from a directory (reads config.toml)
    pub fn load(config_dir: &Path) -> Result<Self, ConfigError> {
        let config_path = config_dir.join(crate::CONFIG_FILENAME);
        let mut config = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .map_err(|e| ConfigError::Io(config_path.clone(), e))?;
            toml::from_str(&content).map_err(ConfigError::Parse)?
        } else {
            Config::default()
        };
        config.config_path = Some(config_path);
        Ok(config)
    }

    /// Save config to its TOML file.
    ///
    /// 두 가지를 지킨다.
    ///
    /// 1. **사용자 주석·키 순서 보존** — 기존 파일이 있으면 `toml_edit`로 문서를
    ///    열어 값만 갈아끼운다. 예전에는 구조체를 통째로 직렬화해 덮어써서
    ///    `kmd config set` 한 번에 손으로 단 주석이 전부 사라졌다.
    /// 2. **원자적 교체** — 임시 파일에 쓰고 flush/sync 뒤 rename 한다. 예전의
    ///    직접 `fs::write`는 디스크가 차거나 프로세스가 중간에 죽으면 config.toml이
    ///    반쪽 파일로 남을 수 있었다.
    pub fn save(&self) -> Result<(), ConfigError> {
        let path = self.config_path.as_ref().ok_or(ConfigError::NoPath)?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ConfigError::Io(path.clone(), e))?;
        }

        let serialized =
            toml::to_string_pretty(self).map_err(|e| ConfigError::Serialize(e.to_string()))?;
        let content = match std::fs::read_to_string(path) {
            Ok(existing) => merge_into_document(&existing, &serialized)?,
            // 파일이 없거나 못 읽으면 새로 쓴다 (보존할 주석도 없다)
            Err(_) => serialized,
        };

        write_atomic(path, &content)
    }

    /// 디스크의 최신 설정을 읽어 **변경분만 적용**하고 저장한다.
    ///
    /// `save()`는 메모리에 있는 설정 전체를 파일에 반영한다. 그래서 오래 열어 둔
    /// 창이 저장하면 **그 사이 다른 곳(CLI·데몬·다른 창)에서 바뀐 값까지 옛 값으로
    /// 되돌린다**:
    ///
    /// ```text
    /// TUI 열기(테마=A) → CLI에서 테마=B로 변경 → TUI에서 검색 경로만 수정·저장
    ///   → save()는 테마=A까지 다시 쓴다 (CLI 변경 유실)
    /// ```
    ///
    /// 이 함수는 저장 직전에 파일을 다시 읽어 그 위에 `edit`이 바꾸는 항목만
    /// 얹는다. 호출자는 **자기가 바꾸는 필드만** 손대면 된다.
    ///
    /// 파일 잠금은 아직 없다 — 두 프로세스가 같은 순간에 읽고 쓰면 나중 것이
    /// 이긴다. 다만 창을 열어 둔 시간 전체가 아니라 저장 순간의 짧은 창만
    /// 남으므로, 실사용에서 유실이 사실상 사라진다.
    ///
    /// 반환값은 저장된 설정 — 호출자가 메모리 캐시를 이것으로 교체하면
    /// 디스크와 어긋나지 않는다.
    pub fn update_and_save(
        path: &Path,
        edit: impl FnOnce(&mut Config),
    ) -> Result<Config, ConfigError> {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let mut latest = match Config::load(dir) {
            Ok(c) => c,
            // 파일이 없거나 깨졌으면 기본값 위에 편집을 얹는다 —
            // 저장 자체를 포기하면 사용자 조작이 조용히 사라진다.
            Err(e) => {
                tracing::warn!("설정을 다시 읽지 못해 기본값에 적용한다: {e}");
                Config::default()
            }
        };
        latest.config_path = Some(path.to_path_buf());
        edit(&mut latest);
        latest.save()?;
        Ok(latest)
    }

    /// Get a config value by dot-separated key path
    pub fn get_value(&self, key: &str) -> Option<String> {
        // Macro to reduce duplication between get/set match arms
        macro_rules! get {
            ($field:expr) => {
                Some($field.to_string())
            };
            (str $field:expr) => {
                Some($field.clone())
            };
            (opt $field:expr) => {
                Some($field.clone().unwrap_or_default())
            };
        }
        // 레지스트리 키(get/set이 한 선언에서 생성됨)를 먼저 본다
        if let Some(v) = self.registry_get(key) {
            return Some(v);
        }

        match key {
            // general
            "general.editor" => get!(opt self.general.editor),
            // launcher
            "launcher.everything_path" => Some(
                self.launcher
                    .everything_path
                    .clone()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            ),
            // kind_weights
            "launcher.multi_llm_providers" => Some(self.launcher.multi_llm_providers.join(",")),
            "launcher.multi_llm_prefixes" => Some(self.launcher.multi_llm_prefixes.join(",")),
            "launcher.llm_autopilot" => Some(self.launcher.llm_autopilot.to_string()),
            "launcher.multi_web_providers" => Some(self.launcher.multi_web_providers.join(",")),
            "launcher.multi_web_prefixes" => Some(self.launcher.multi_web_prefixes.join(",")),
            "launcher.spell_providers" => Some(self.launcher.spell_providers.join(",")),
            "launcher.spell_prefixes" => Some(self.launcher.spell_prefixes.join(",")),
            "launcher.translate_providers" => Some(self.launcher.translate_providers.join(",")),
            "launcher.translate_prefixes" => Some(self.launcher.translate_prefixes.join(",")),
            "launcher.prompt_templates" => {
                let list: Vec<String> = self
                    .launcher
                    .prompt_templates
                    .iter()
                    .map(|t| format!("{}:{}", t.name, t.body))
                    .collect();
                Some(list.join("|"))
            }
            "launcher.keymap.backend" => Some(self.launcher.keymap.backend.clone()),
            "launcher.keymap.kanata_path" => Some(
                self.launcher
                    .keymap
                    .kanata_path
                    .clone()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            ),
            "launcher.keymap.profile_dir" => Some(
                self.launcher
                    .keymap
                    .profile_dir
                    .clone()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            ),
            "launcher.keymap.active_profile" => Some(self.launcher.keymap.active_profile.clone()),
            // keybindings
            // virtual keys (read-only, computed at runtime)
            "_portable_mode" => Some(
                if crate::portable::is_portable() {
                    "Portable"
                } else {
                    "System"
                }
                .into(),
            ),
            "_data_path" => Some(Self::default_data_dir().to_string_lossy().to_string()),
            _ => None,
        }
    }

    /// Set a config value by dot-separated key path
    pub fn set_value(&mut self, key: &str, value: &str) -> Result<(), ConfigError> {
        // 레지스트리 키는 여기서 처리된다 — get/set 비대칭이 불가능한 쪽
        if self.registry_set(key, value)?.is_some() {
            return Ok(());
        }

        match key {
            // general
            "general.editor" => {
                self.general.editor = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                }
            }
            // launcher
            "launcher.everything_path" => {
                self.launcher.everything_path = if value.trim().is_empty() {
                    None
                } else {
                    Some(PathBuf::from(value))
                }
            }
            // kind_weights
            "launcher.multi_llm_providers" => {
                self.launcher.multi_llm_providers = value
                    .split(',')
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "launcher.multi_llm_prefixes" => {
                self.launcher.multi_llm_prefixes = value
                    .split(',')
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        if s.starts_with('@') {
                            s
                        } else {
                            format!("@{s}")
                        }
                    })
                    .collect();
            }
            "launcher.llm_autopilot" => {
                self.launcher.llm_autopilot = matches!(
                    value.trim().to_lowercase().as_str(),
                    "true" | "1" | "on" | "yes"
                );
            }
            "launcher.multi_web_providers" => {
                self.launcher.multi_web_providers = value
                    .split(',')
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "launcher.multi_web_prefixes" => {
                self.launcher.multi_web_prefixes = value
                    .split(',')
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        if s.starts_with('@') {
                            s
                        } else {
                            format!("@{s}")
                        }
                    })
                    .collect();
            }
            "launcher.spell_providers" => {
                self.launcher.spell_providers = value
                    .split(',')
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "launcher.spell_prefixes" => {
                self.launcher.spell_prefixes = value
                    .split(',')
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        if s.starts_with('@') {
                            s
                        } else {
                            format!("@{s}")
                        }
                    })
                    .collect();
            }
            "launcher.translate_providers" => {
                self.launcher.translate_providers = value
                    .split(',')
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "launcher.translate_prefixes" => {
                self.launcher.translate_prefixes = value
                    .split(',')
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        if s.starts_with('@') {
                            s
                        } else {
                            format!("@{s}")
                        }
                    })
                    .collect();
            }
            "launcher.keymap.backend" => self.launcher.keymap.backend = value.to_string(),
            "launcher.keymap.kanata_path" => {
                self.launcher.keymap.kanata_path = if value.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(value))
                }
            }
            "launcher.keymap.profile_dir" => {
                self.launcher.keymap.profile_dir = if value.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(value))
                }
            }
            "launcher.keymap.active_profile" => {
                self.launcher.keymap.active_profile = value.to_string()
            }
            // keybindings
            _ => return Err(ConfigError::UnknownKey(key.to_string())),
        }
        Ok(())
    }

    /// Return the OS-standard config directory (ignoring portable mode).
    pub fn system_config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("kmd")
    }

    /// Return the OS-standard data directory (ignoring portable mode).
    pub fn system_data_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("kmd")
    }

    /// Return the config directory for kmd.
    ///
    /// In portable mode (`kmd-data/` next to exe) this returns the portable
    /// directory. Otherwise the OS-standard config location is used.
    pub fn default_config_dir() -> PathBuf {
        if let Some(dir) = crate::portable::portable_data_dir() {
            if dir.is_dir() {
                return dir;
            }
        }
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("kmd")
    }

    /// Return the data directory for kmd.
    ///
    /// In portable mode (`kmd-data/` next to exe) this returns the portable
    /// directory. Otherwise the OS-standard data location is used.
    pub fn default_data_dir() -> PathBuf {
        if let Some(dir) = crate::portable::portable_data_dir() {
            if dir.is_dir() {
                return dir;
            }
        }
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("kmd")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Config file I/O error at {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("Config parse error: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("Config serialize error: {0}")]
    Serialize(String),
    #[error("Config path not set")]
    NoPath,
    #[error("Unknown config key: {0}")]
    UnknownKey(String),
    #[error("설정 '{key}'에 '{value}'는 올바르지 않습니다 (필요: {expected})")]
    InvalidValue {
        key: String,
        value: String,
        expected: &'static str,
    },
}

/// 직렬화 결과(`fresh`)를 기존 문서(`existing`)에 병합해 주석·키 순서를 살린다.
///
/// 기존 문서를 기준으로 값만 갈아끼우므로 키에 붙은 주석(decor)이 그대로 남는다.
/// 기존 문서가 깨져서 파싱되지 않으면 병합을 포기하고 새 직렬화 결과를 쓴다 —
/// 저장 자체를 실패시키는 것보다 낫다.
fn merge_into_document(existing: &str, fresh: &str) -> Result<String, ConfigError> {
    use toml_edit::DocumentMut;

    let Ok(mut doc) = existing.parse::<DocumentMut>() else {
        return Ok(fresh.to_string());
    };
    let new_doc = fresh
        .parse::<DocumentMut>()
        .map_err(|e| ConfigError::Serialize(e.to_string()))?;

    merge_table(doc.as_table_mut(), new_doc.as_table());
    Ok(doc.to_string())
}

/// `new`의 내용을 `old`에 재귀적으로 반영한다.
///
/// - 양쪽에 다 있는 하위 테이블은 재귀 (그 안의 주석도 보존)
/// - 값이 바뀌면 값만 교체하고 기존 decor(주석·공백)를 되돌려 붙인다
/// - `new`에 없는 키는 제거한다 (설정에서 사라진 항목)
fn merge_table(old: &mut toml_edit::Table, new: &toml_edit::Table) {
    let stale: Vec<String> = old
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| !new.contains_key(k))
        .collect();
    for key in stale {
        old.remove(&key);
    }

    for (key, new_item) in new.iter() {
        match (old.get_mut(key), new_item) {
            // 양쪽 다 테이블 → 재귀해서 안쪽 주석까지 보존
            (Some(toml_edit::Item::Table(old_t)), toml_edit::Item::Table(new_t)) => {
                merge_table(old_t, new_t);
            }
            // 기존 키가 있으면 값만 교체하고 decor(주석/공백)를 복원
            (Some(old_item), _) => {
                let decor = old_item.as_value().map(|v| v.decor().clone());
                *old_item = new_item.clone();
                if let (Some(d), Some(v)) = (decor, old_item.as_value_mut()) {
                    *v.decor_mut() = d;
                }
            }
            // 새 키
            (None, _) => {
                old.insert(key, new_item.clone());
            }
        }
    }
}

/// 임시 파일 → flush/sync → rename 으로 원자적으로 쓴다.
///
/// 두 가지를 막는다.
/// - 저장 도중 프로세스가 죽거나 디스크가 차서 **반쪽 파일**이 남는 것
/// - 데몬의 주기적 config 재로드가 그 반쪽 파일을 읽고 기본값으로 폴백하는 창
///
/// 같은 디렉터리에 임시 파일을 만들어야 rename이 같은 파일시스템 안에서
/// 원자적으로 동작한다.
fn write_atomic(path: &Path, content: &str) -> Result<(), ConfigError> {
    use std::io::Write;

    // 임시 파일명은 프로세스·스레드별로 고유해야 한다. 고정 이름(config.toml.tmp)을
    // 쓰면 데몬·데스크톱·TUI가 동시에 저장할 때 서로의 임시 파일을 덮어써
    // 뒤섞인 내용이 rename될 수 있다.
    let unique = format!(
        "toml.tmp.{}.{:?}",
        std::process::id(),
        std::thread::current().id()
    );
    // ThreadId의 Debug는 "ThreadId(2)" 꼴이라 경로에 쓰기 나쁜 문자를 지운다.
    let unique: String = unique
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let tmp = path.with_extension(unique);
    let io_err = |e| ConfigError::Io(path.to_path_buf(), e);

    {
        let mut f = std::fs::File::create(&tmp).map_err(io_err)?;
        f.write_all(content.as_bytes()).map_err(io_err)?;
        f.flush().map_err(io_err)?;
        f.sync_all().map_err(io_err)?;
    }

    // Rust의 fs::rename은 Windows에서도 기존 파일을 덮어쓴다(MOVEFILE_REPLACE_EXISTING).
    // 예전에 넣었던 "교체 전 remove_file"은 불필요했고, 오히려 파일이 없는 순간을
    // 만들어 원자성을 깨뜨렸다 — 실측으로 확인하고 제거했다 (2026-08-28).
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(ConfigError::Io(path.to_path_buf(), e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CONFIG_FILENAME;

    // ── 잘못된 값 거부 (2026-08-27) ────────────────────────────────────
    //
    // 예전에는 파싱 실패 시 기존 값을 유지한 채 Ok(())를 반환해서,
    // `kmd config set general.render_fps abc`가 성공 메시지 + 종료코드 0을
    // 내면서 값은 그대로였다. TUI는 그 결과마저 버리고 "변경됨"으로 표시했다.

    // ── 설정 키 레지스트리 ─────────────────────────────────────────────

    #[test]
    fn 레지스트리_키는_전부_get과_set이_된다() {
        // 매크로가 양쪽을 함께 생성하므로 비대칭은 원천적으로 불가능하지만,
        // 선언 자체가 잘못돼도(오타난 필드 경로 등) 여기서 걸린다.
        let mut c = Config::default();
        for key in Config::registry_keys() {
            let v = c
                .get_value(key)
                .unwrap_or_else(|| panic!("레지스트리 키 '{key}' 를 get_value가 모른다"));
            c.set_value(key, &v)
                .unwrap_or_else(|e| panic!("레지스트리 키 '{key}' 를 set_value가 거부한다: {e}"));
            assert_eq!(
                c.get_value(key).as_deref(),
                Some(v.as_str()),
                "'{key}' 왕복 값이 달라졌다"
            );
        }
    }

    #[test]
    fn 레지스트리_숫자_키는_잘못된_값을_거부한다() {
        let mut c = Config::default();
        // num 종류만 골라낸다 — 값이 숫자/불리언으로 파싱되는 키
        let numeric: Vec<&str> = Config::registry_keys()
            .iter()
            .copied()
            .filter(|k| {
                let v = Config::default().get_value(k).unwrap_or_default();
                v.parse::<i64>().is_ok() || v.parse::<bool>().is_ok()
            })
            .collect();
        assert!(numeric.len() >= 10, "숫자/불리언 키가 충분히 잡혀야 한다");

        for key in numeric {
            assert!(
                c.set_value(key, "완전히 잘못된 값").is_err(),
                "'{key}' 가 잘못된 값을 조용히 받아들인다"
            );
        }
    }

    #[test]
    fn 레지스트리와_명시적_arm이_겹치지_않는다() {
        // 같은 키가 양쪽에 있으면 레지스트리가 먼저 이겨 명시적 arm이
        // 죽은 코드가 된다 — 읽는 사람이 잘못된 곳을 고치게 된다.
        let src = include_str!("config.rs");
        let (_, after) = src.split_once("pub fn get_value").expect("get_value");
        let (get_body, rest) = after.split_once("pub fn set_value").expect("set_value");
        for key in Config::registry_keys() {
            let needle = format!("\"{key}\" =>");
            assert!(
                !get_body.contains(&needle),
                "'{key}' 가 레지스트리와 get_value 양쪽에 있다"
            );
            assert!(
                !rest.contains(&needle),
                "'{key}' 가 레지스트리와 set_value 양쪽에 있다"
            );
        }
    }

    #[test]
    fn 숫자_필드에_숫자가_아닌_값은_거부한다() {
        let mut c = Config::default();
        let before = c.general.render_fps;

        let err = c
            .set_value("general.render_fps", "이건숫자가아님")
            .expect_err("파싱 실패는 오류여야 한다");

        assert!(matches!(err, ConfigError::InvalidValue { .. }));
        assert_eq!(c.general.render_fps, before, "실패 시 값이 바뀌면 안 된다");
        // 메시지에 무엇이 잘못됐는지 담긴다
        let msg = err.to_string();
        assert!(msg.contains("general.render_fps"), "키 표시: {msg}");
        assert!(msg.contains("이건숫자가아님"), "입력값 표시: {msg}");
    }

    #[test]
    fn 불리언_필드에_아무_문자열이나_받지_않는다() {
        let mut c = Config::default();
        assert!(c.set_value("general.show_preview", "yes").is_err());
        assert!(c.set_value("general.show_preview", "false").is_ok());
        assert!(!c.general.show_preview);
    }

    #[test]
    fn 유효한_값은_정상_반영된다() {
        let mut c = Config::default();
        c.set_value("general.render_fps", "45").unwrap();
        assert_eq!(c.general.render_fps, 45);
        assert_eq!(c.get_value("general.render_fps").as_deref(), Some("45"));
    }

    // ── 저장: 주석 보존 + 원자적 교체 ─────────────────────────────────

    fn temp_config_path(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("kmd_cfg_test_{tag}_{}.toml", std::process::id()));
        p
    }

    // ── 동시 편집: 남의 변경을 덮지 않는다 (REF-01) ────────────────────
    //
    // save()는 메모리 전체를 쓰므로, 오래 열어 둔 창이 저장하면 그 사이 다른
    // 곳에서 바뀐 값까지 되돌린다. update_and_save는 저장 직전에 파일을 다시
    // 읽어 변경분만 얹는다.

    #[test]
    fn 변경분만_적용해_다른_곳의_수정을_보존한다() {
        let dir = std::env::temp_dir().join(format!(
            "kmd_cfg_concurrent_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CONFIG_FILENAME);

        // 초기 상태 저장
        let mut base = Config {
            config_path: Some(path.clone()),
            ..Default::default()
        };
        base.general.theme = "default".into();
        base.general.render_fps = 30;
        base.save().unwrap();

        // A가 설정을 읽어 든다(오래 열어 둔 창)
        let stale = Config::load(&dir).unwrap();
        assert_eq!(stale.general.theme, "default");

        // 그 사이 B가 테마를 바꿔 저장
        Config::update_and_save(&path, |c| c.general.theme = "nord".into()).unwrap();

        // A가 자기 항목(fps)만 바꿔 저장 — B의 테마를 되돌리면 안 된다
        let saved = Config::update_and_save(&path, |c| {
            c.general.render_fps = stale.general.render_fps + 15;
        })
        .unwrap();

        assert_eq!(saved.general.render_fps, 45, "내 변경은 반영된다");
        assert_eq!(saved.general.theme, "nord", "남의 변경을 덮지 않는다");

        // 디스크에도 동일하게 남아야 한다
        let reloaded = Config::load(&dir).unwrap();
        assert_eq!(reloaded.general.theme, "nord");
        assert_eq!(reloaded.general.render_fps, 45);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 저장_실패는_오류로_전달된다() {
        // 일반 파일을 부모로 지정 — 그 아래에는 디렉터리를 만들 수 없다
        let dir = std::env::temp_dir().join(format!(
            "kmd_cfg_failpath_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("not-a-dir");
        std::fs::write(&blocker, b"x").unwrap();

        let err = Config::update_and_save(&blocker.join(CONFIG_FILENAME), |c| {
            c.general.render_fps = 99;
        });
        assert!(err.is_err(), "저장 불가 경로는 오류여야 한다");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 저장이_사용자_주석을_보존한다() {
        let path = temp_config_path("comments");
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            "# 내가 손으로 단 머리말
[general]
# FPS 설명 주석
render_fps = 30
",
        )
        .unwrap();

        let mut c = Config {
            config_path: Some(path.clone()),
            ..Default::default()
        };
        c.set_value("general.render_fps", "45").unwrap();
        c.save().unwrap();

        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.contains("# 내가 손으로 단 머리말"),
            "머리말 주석 보존
{saved}"
        );
        assert!(
            saved.contains("# FPS 설명 주석"),
            "키 주석 보존
{saved}"
        );
        assert!(
            saved.contains("render_fps = 45"),
            "값 반영
{saved}"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn 저장_후_임시파일이_남지_않는다() {
        let path = temp_config_path("atomic");
        let _ = std::fs::remove_file(&path);

        let c = Config {
            config_path: Some(path.clone()),
            ..Default::default()
        };
        c.save().unwrap();

        assert!(path.exists(), "저장된 파일 존재");
        // 임시 파일명은 프로세스·스레드별로 고유하므로 이름을 특정할 수 없다.
        // 대신 **이 테스트가 만든 파일**의 이름으로 시작하는 tmp만 센다
        // (공용 temp 디렉터리에는 무관한 .tmp 파일이 널려 있다).
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(&stem) && n.contains(".toml.tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "임시 파일은 rename으로 사라져야 한다: {leftovers:?}"
        );
        // 저장 결과가 다시 읽히는 온전한 TOML인지
        let reloaded: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reloaded.general.render_fps, c.general.render_fps);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn 깨진_기존_파일이어도_저장은_성공한다() {
        // 병합 대상이 파싱 안 되면 새 직렬화 결과로 대체한다 — 저장 실패보다 낫다
        let path = temp_config_path("broken");
        std::fs::write(&path, "이건 = = 올바른 TOML이 아니다 [[[").unwrap();

        let c = Config {
            config_path: Some(path.clone()),
            ..Default::default()
        };
        c.save().unwrap();

        let reloaded: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reloaded.general.render_fps, 30);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.general.render_fps, 30);
        assert_eq!(config.launcher.file_search_provider, "auto");
        assert!(config.keybindings.global_hotkey.is_empty());
        assert_eq!(config.launcher.kind_weights.directory, 80);
        assert!(config.launcher.index_directories);
        assert!(!config.launcher.scan_drives);
        assert_eq!(config.launcher.drive_scan_depth, 2);
        assert_eq!(
            config.launcher.multi_llm_providers,
            vec!["chatgpt", "gemini", "claude", "grok", "perplexity"]
        );
        assert_eq!(
            config.launcher.multi_llm_prefixes,
            vec!["@llm", "@ll", "@multi", "@cmp", "@compare"]
        );
        assert_eq!(
            config.launcher.multi_web_providers,
            vec!["google", "naver_search", "daum"]
        );
        assert_eq!(
            config.launcher.multi_web_prefixes,
            vec![
                "@m",
                "@mw",
                "@msearch",
                "@multisearch",
                "@searchall",
                "@krsearch"
            ]
        );
        assert_eq!(
            config.launcher.spell_providers,
            vec!["naver_spell", "pusan_spell"]
        );
        assert_eq!(config.launcher.spell_prefixes, vec!["@sp", "@spell"]);
        assert_eq!(
            config.launcher.translate_providers,
            vec!["google_translate", "papago", "deepl"]
        );
        assert_eq!(
            config.launcher.translate_prefixes,
            vec!["@tr", "@trko", "@tren"]
        );
        assert_eq!(config.launcher.keymap.backend, "kanata");
        assert_eq!(config.launcher.keymap.active_profile, "vim-nav");
    }

    #[test]
    fn test_get_set_value() {
        let mut config = Config::default();
        config.set_value("general.theme", "nord").unwrap();
        assert_eq!(config.get_value("general.theme"), Some("nord".to_string()));

        config
            .set_value("launcher.kind_weights.directory", "90")
            .unwrap();
        assert_eq!(
            config.get_value("launcher.kind_weights.directory"),
            Some("90".to_string())
        );

        config
            .set_value("launcher.multi_llm_providers", "chatgpt,claude,perplexity")
            .unwrap();
        assert_eq!(
            config.get_value("launcher.multi_llm_providers"),
            Some("chatgpt,claude,perplexity".to_string())
        );
        config
            .set_value("launcher.multi_llm_prefixes", "llm,ll,cmp")
            .unwrap();
        assert_eq!(
            config.get_value("launcher.multi_llm_prefixes"),
            Some("@llm,@ll,@cmp".to_string())
        );

        config
            .set_value("launcher.multi_web_providers", "google,daum")
            .unwrap();
        assert_eq!(
            config.get_value("launcher.multi_web_providers"),
            Some("google,daum".to_string())
        );
        config
            .set_value("launcher.multi_web_prefixes", "m,mw,msearch")
            .unwrap();
        assert_eq!(
            config.get_value("launcher.multi_web_prefixes"),
            Some("@m,@mw,@msearch".to_string())
        );
        config
            .set_value("launcher.spell_prefixes", "sp,spell")
            .unwrap();
        assert_eq!(
            config.get_value("launcher.spell_prefixes"),
            Some("@sp,@spell".to_string())
        );
        config
            .set_value("launcher.translate_prefixes", "tr,trko,tren")
            .unwrap();
        assert_eq!(
            config.get_value("launcher.translate_prefixes"),
            Some("@tr,@trko,@tren".to_string())
        );
        config
            .set_value("launcher.keymap.active_profile", "coding.kbd")
            .unwrap();
        assert_eq!(
            config.get_value("launcher.keymap.active_profile"),
            Some("coding.kbd".to_string())
        );
    }

    #[test]
    fn test_unknown_key() {
        let mut config = Config::default();
        assert!(config.set_value("nonexistent.key", "val").is_err());
    }
}
