//! 키맵 설정 — `[launcher.keymap]` 과 TOML 레이어/콤보/더블탭/tap-hold 구조체

use super::*;

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
    ///
    /// 이 파일의 맵은 전부 BTreeMap이다 — HashMap이면 직렬화(저장)할 때마다 키
    /// 순서가 달라져 config.toml diff가 요동치고, 순회 순서에 기대는 코드가 실행마다
    /// 다르게 동작한다.
    #[serde(default)]
    pub remaps: BTreeMap<String, String>,
    /// 레이어 정의. 이름 = { trigger, mappings, ... }
    #[serde(default)]
    pub layers: BTreeMap<String, LayerToml>,
    /// 수정자+키 콤보 (예: Shift+Space → Hangul)
    #[serde(default)]
    pub combos: Vec<ComboToml>,
    /// 글로벌 더블탭
    #[serde(default)]
    pub double_taps: Vec<DoubleTapToml>,
    /// tap-hold(모드탭): 키이름 = { tap, hold, timeout_ms }
    /// 짧게 탭 = tap 키, 홀드 중 다른 키 = hold 수정자 조합 (HHKB 스타일)
    #[serde(default)]
    pub tap_holds: BTreeMap<String, TapHoldToml>,
    /// 손버릇 통계 (docs/19) — 데몬이 레이어 수 사용·맨 화살표·트리거 홀드 시간을
    /// **횟수로만** 센다. 글자·순서·시각은 남기지 않는다. 기본 꺼짐.
    #[serde(default)]
    pub stats: bool,
}

impl Default for KeymapConfig {
    fn default() -> Self {
        Self {
            backend: "kanata".to_string(),
            kanata_path: None,
            profile_dir: None,
            active_profile: "vim-nav".to_string(),
            remaps: BTreeMap::new(),
            layers: BTreeMap::new(),
            combos: Vec::new(),
            double_taps: Vec::new(),
            tap_holds: BTreeMap::new(),
            stats: false,
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
    pub mappings: BTreeMap<String, String>,
    /// 레이어 내 더블탭: 키이름 = { single, double, timeout_ms }
    pub double_taps: BTreeMap<String, LayerDoubleTapToml>,
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
