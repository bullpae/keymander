//! 설정 키 레지스트리 — get/set 을 한 선언에서 생성하는 매크로와 명시적 키 처리

use super::*;

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
    /// `get_value`/`set_value`로 다룰 수 있는 **모든** 설정 키.
    ///
    /// `registry_keys()`는 매크로가 생성한 단순 키만 담는다. 리스트 정규화나
    /// 중첩 구조가 필요한 키는 여전히 명시적 match arm으로 처리되는데, 이 둘을
    /// 헷갈리면 조용한 유실이 난다 — 실제로 TUI 저장이 `registry_keys()`만
    /// 훑어 **화면에서 편집한 11개 항목이 저장되지 않았다**(v0.16.6 회귀).
    ///
    /// 아래 `EXPLICIT_KEYS`와 레지스트리를 합친 것이 정본이다. 새 키를 명시적
    /// arm으로 추가하면 여기에도 넣어야 하며, `모든_편집키가_왕복한다` 테스트가
    /// 누락을 잡는다.
    pub fn all_editable_keys() -> Vec<&'static str> {
        /// 매크로 밖에서 match arm으로 처리되는 키들 (리스트·중첩 구조).
        // 주의: search_paths·ignore_patterns는 여기 없다 — get_value를 지원하지
        // 않고 전용 리스트 편집 UI가 직접 다룬다. 저장 경로도 그 둘만 따로
        // 비교해 반영한다.
        const EXPLICIT_KEYS: &[&str] = &[
            "general.editor",
            "launcher.everything_path",
            "launcher.multi_llm_providers",
            "launcher.multi_llm_prefixes",
            "launcher.multi_web_providers",
            "launcher.multi_web_prefixes",
            "launcher.spell_providers",
            "launcher.spell_prefixes",
            "launcher.translate_providers",
            "launcher.translate_prefixes",
            "launcher.llm_autopilot",
            "launcher.keymap.backend",
            "launcher.keymap.kanata_path",
            "launcher.keymap.profile_dir",
            "launcher.keymap.active_profile",
        ];
        let mut keys: Vec<&'static str> = Self::registry_keys().to_vec();
        keys.extend_from_slice(EXPLICIT_KEYS);
        keys
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
                self.launcher.multi_llm_providers = parse_csv_lower(value)
            }
            "launcher.multi_llm_prefixes" => {
                self.launcher.multi_llm_prefixes = parse_at_prefixes(value)
            }
            "launcher.llm_autopilot" => {
                self.launcher.llm_autopilot = matches!(
                    value.trim().to_lowercase().as_str(),
                    "true" | "1" | "on" | "yes"
                );
            }
            "launcher.multi_web_providers" => {
                self.launcher.multi_web_providers = parse_csv_lower(value)
            }
            "launcher.multi_web_prefixes" => {
                self.launcher.multi_web_prefixes = parse_at_prefixes(value)
            }
            "launcher.spell_providers" => self.launcher.spell_providers = parse_csv_lower(value),
            "launcher.spell_prefixes" => self.launcher.spell_prefixes = parse_at_prefixes(value),
            "launcher.translate_providers" => {
                self.launcher.translate_providers = parse_csv_lower(value)
            }
            "launcher.translate_prefixes" => {
                self.launcher.translate_prefixes = parse_at_prefixes(value)
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
}

/// `"a, B ,c"` → `["a","b","c"]` (소문자·공백 제거·빈 항목 제외) — 공급자 ID 목록용.
fn parse_csv_lower(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// [`parse_csv_lower`] + 각 항목에 `@`를 보장 — 명령 별칭 목록용 (`sp` → `@sp`).
fn parse_at_prefixes(value: &str) -> Vec<String> {
    parse_csv_lower(value)
        .into_iter()
        .map(|s| {
            if s.starts_with('@') {
                s
            } else {
                format!("@{s}")
            }
        })
        .collect()
}
