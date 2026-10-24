//! 설정 섹션 구조체와 기본값

use super::defaults::*;
use super::*;

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
    /// 추가 검색 폴더. `include_user_folders`가 켜져 있으면 OS 기본 사용자 폴더에
    /// **더해진다** (docs/18). 실제 스캔 루트는 [`Self::effective_search_paths`].
    pub search_paths: Vec<PathBuf>,
    /// OS 기본 사용자 폴더(바탕 화면·문서·다운로드·OneDrive)를 범위에 포함.
    ///
    /// `None` = 레거시 설정 — `search_paths`만 적고 이 키가 없던 파일이다. 예전엔
    /// `search_paths`를 적으면 기본 폴더를 **대체**했으므로, 그 의미를 그대로 지켜
    /// 적힌 목록만 쓴다. 판정은 로드 시 [`super::Config::from_toml_str`]가 한다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_user_folders: Option<bool>,
    /// 범위에서 뺀 기본 사용자 폴더. 경로로 기억하므로 OS가 다른 기본 폴더를
    /// 알려 줘도(OneDrive 리디렉션 등) 뺀 것만 계속 빠진다.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden_user_folders: Vec<PathBuf>,
    /// Maximum search results from file providers
    pub max_results: usize,
    /// Maximum recursive directory depth for file scanning (default: 4)
    pub search_depth: usize,
    /// 추가 제외 폴더 이름(정확히 일치). `use_default_ignores`가 켜져 있으면
    /// [`DEFAULT_IGNORE_PATTERNS`]에 **더해진다**. 실제 규칙은 [`Self::effective_ignore_patterns`].
    pub ignore_patterns: Vec<String>,
    /// 기본 제외 폴더 목록을 함께 쓴다. `None` = 레거시(`ignore_patterns`만 적은
    /// 파일) — 예전 "대체" 의미대로 적힌 목록만 쓴다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_default_ignores: Option<bool>,
    /// 인덱싱에서 뺄 파일 확장자 (대소문자 무시). `cat`·`.cat`·`*.cat` 모두 허용.
    /// `ignore_patterns`는 폴더 이름만 맞추므로, 실행 파일 옆의 부속 파일
    /// (`veracrypt.cat`·`.inf` 등)은 폴더째 빼지 않고는 거를 수 없었다.
    /// 파일에만 적용(폴더 제외) — 본문 인덱스도 같은 규칙을 따른다.
    #[serde(default)]
    pub ignore_extensions: Vec<String>,
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
    /// `:길` 카카오맵 길찾기 — 내 장소·기본 출발지·카카오 키 파일 (docs/20).
    #[serde(default)]
    pub route: crate::route::RouteConfig,
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

impl LauncherConfig {
    /// 범위에 들어가는 OS 기본 사용자 폴더 — 꺼져 있거나 레거시면 비어 있다.
    pub fn user_folders(&self) -> Vec<PathBuf> {
        if self.include_user_folders != Some(true) {
            return Vec::new();
        }
        default_search_paths()
            .into_iter()
            .filter(|p| !self.hidden_user_folders.iter().any(|h| same_path(h, p)))
            .collect()
    }

    /// 실제 스캔 루트 — 기본 사용자 폴더 + `search_paths`, 중복 없이(순서 유지).
    ///
    /// `search_paths`의 항목은 지금 없는 폴더라도 남긴다 — 외장 드라이브가 잠시
    /// 빠졌을 때 본문 인덱스가 그 아래를 지우지 않게 하는 판단(content_index)이
    /// 루트 목록에 기대고 있다. 스캔하는 쪽이 존재 여부를 따로 확인한다.
    pub fn effective_search_paths(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for p in self
            .user_folders()
            .into_iter()
            .chain(self.search_paths.iter().cloned())
        {
            if !out.iter().any(|seen| same_path(seen, &p)) {
                out.push(p);
            }
        }
        out
    }

    /// 실제 제외 폴더 규칙 — 기본 목록(켜져 있으면) + `ignore_patterns`, 중복 없이.
    pub fn effective_ignore_patterns(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let defaults: &[&str] = if self.use_default_ignores == Some(true) {
            DEFAULT_IGNORE_PATTERNS
        } else {
            &[]
        };
        for p in defaults
            .iter()
            .map(|s| s.to_string())
            .chain(self.ignore_patterns.iter().cloned())
        {
            if !out.contains(&p) {
                out.push(p);
            }
        }
        out
    }

    /// `ignore_extensions`를 비교용으로 정규화한 집합 — 소문자, 앞의 `*`·`.` 제거.
    pub fn ignored_extension_set(&self) -> std::collections::HashSet<String> {
        self.ignore_extensions
            .iter()
            .map(|e| {
                e.trim()
                    .trim_start_matches('*')
                    .trim_start_matches('.')
                    .to_lowercase()
            })
            .filter(|e| !e.is_empty())
            .collect()
    }
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            file_search_provider: "auto".to_string(),
            everything_path: None,
            search_paths: Vec::new(),
            include_user_folders: Some(true),
            hidden_user_folders: Vec::new(),
            // Conservative defaults to reduce cold-start indexing cost.
            max_results: 5000,
            search_depth: 4,
            ignore_patterns: Vec::new(),
            use_default_ignores: Some(true),
            ignore_extensions: Vec::new(),
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
            route: Default::default(),
        }
    }
}

/// 재사용 가능한 프롬프트 템플릿. `@ll :name question` 형태로 LLM에 전달.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PromptTemplate {
    /// 고유 이름 (영문/숫자/하이픈, 예: code-review)
    pub name: String,
    /// 본문 — `{query}` 자리표시자가 있으면 실제 질문으로 치환.
    /// 없으면 `body + "\n\n" + query` 형태로 결합.
    pub body: String,
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

/// 같은 폴더인지 — Windows는 대소문자·구분자 무시 (`fsutil::path_within` 양방향).
pub(crate) fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    crate::fsutil::path_within(a, b) && crate::fsutil::path_within(b, a)
}
