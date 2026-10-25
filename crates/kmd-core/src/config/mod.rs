//! TOML-based configuration management
//!
//! Launcher-focused config with keybindings and provider settings.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

mod defaults;
mod keymap_toml;
mod persist;
mod registry;
mod types;

pub use defaults::DEFAULT_IGNORE_PATTERNS;
pub use keymap_toml::{
    ComboToml, DoubleTapToml, KeymapConfig, LayerDoubleTapToml, LayerToml, TapHoldToml,
};
pub use persist::SEED_CONFIG_FILENAME;
use persist::{merge_into_document, seed_config, write_atomic};

/// OS 기본 사용자 폴더(바탕 화면·문서·다운로드·OneDrive) — 존재하는 것만, 중복 없이.
/// `launcher.include_user_folders`가 켜져 있으면 검색 범위에 들어간다 (docs/18).
pub fn default_user_folders() -> Vec<PathBuf> {
    defaults::default_search_paths()
}
pub use types::{
    ClipboardConfig, ContentSearchConfig, CustomWebService, GeneralConfig, KeybindingsConfig,
    KindWeights, LauncherConfig, PromptTemplate,
};

/// Top-level configuration
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub launcher: LauncherConfig,
    pub keybindings: KeybindingsConfig,
    pub clipboard: ClipboardConfig,
    /// `:say` 입버릇 데스크톱 (docs/21)
    pub say: crate::say::SayConfig,

    /// Config file path (excluded from serialization)
    #[serde(skip)]
    pub config_path: Option<PathBuf>,
}

impl Config {
    /// TOML 문자열을 설정으로 읽는다 — 레거시 "대체" 의미 판정을 함께 한다 (docs/18 §5.3).
    ///
    /// 예전엔 `search_paths`·`ignore_patterns`를 적으면 기본값을 **대체**했다. 새
    /// 의미는 "기본값 + 추가"라서, 그 키를 적었는데 새 스위치
    /// (`include_user_folders`·`use_default_ignores`)가 없는 파일은 레거시로 보고
    /// 스위치를 `None`으로 둔다 — 적힌 목록만 쓰는 예전 동작 그대로다.
    /// 키가 아예 없으면 기본값(`Some(true)`)이 남아 기본 폴더·규칙이 그대로 들어간다.
    pub fn from_toml_str(content: &str) -> Result<Self, toml::de::Error> {
        let mut config: Config = toml::from_str(content)?;
        let raw: toml::Table = toml::from_str(content)?;
        let launcher = raw.get("launcher").and_then(|v| v.as_table());
        let has = |key: &str| launcher.is_some_and(|t| t.contains_key(key));
        if has("search_paths") && !has("include_user_folders") {
            config.launcher.include_user_folders = None;
        }
        if has("ignore_patterns") && !has("use_default_ignores") {
            config.launcher.use_default_ignores = None;
        }
        Ok(config)
    }

    /// Load config from a directory (reads config.toml)
    pub fn load(config_dir: &Path) -> Result<Self, ConfigError> {
        let config_path = config_dir.join(crate::CONFIG_FILENAME);
        if !config_path.exists() {
            if let Some(exe_dir) = crate::portable::exe_dir() {
                seed_config(&config_path, &exe_dir);
            }
        }
        let mut config = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .map_err(|e| ConfigError::Io(config_path.clone(), e))?;
            Config::from_toml_str(&content).map_err(ConfigError::Parse)?
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
        // 읽기 실패는 그대로 올린다. 기본값으로 진행하면 **파일에 문법 오류가
        // 있는 상태에서 설정 하나를 바꿨을 때 나머지가 전부 기본값으로 덮인다.**
        // (파일이 아예 없는 경우는 Config::load가 이미 기본값을 돌려주므로
        // 여기까지 오지 않는다.)
        let mut latest = Config::load(dir)?;
        latest.config_path = Some(path.to_path_buf());
        edit(&mut latest);
        latest.save()?;
        Ok(latest)
    }

    /// Return the OS-standard config directory (ignoring portable mode).
    pub fn system_config_dir() -> PathBuf {
        dirs::config_dir()
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
        Self::system_config_dir()
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
        Self::standard_data_dir()
    }

    /// 포터블 여부와 상관없는 OS 표준 데이터 디렉터리 (`%APPDATA%\kmd`,
    /// `~/.local/share/kmd` 등). winget 패키지 폴더처럼 업그레이드 때 정리될 수
    /// 있는 곳을 피해야 하는 작은 상태 파일용. (예전엔 본문이 같은
    /// `system_data_dir`가 따로 있었다 — 하나로 합쳤다.)
    pub fn standard_data_dir() -> PathBuf {
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

#[cfg(test)]
mod tests;
