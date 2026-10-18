//! 설정 모달 연동 — 모달 액션 적용과 설정 저장

use super::*;

// ── Settings Integration ─────────────────────────────────────────────────────

/// Handle a key event when the settings modal is open
pub(super) fn handle_settings_key_event(
    state: &mut AppState,
    key: crossterm::event::KeyEvent,
    engine: &mut SearchEngine,
) {
    // Take ownership of settings state temporarily
    let Some(mut settings_state) = state.settings.take() else {
        return;
    };

    let action = settings::handle_settings_key(&mut settings_state, key);
    apply_settings_action(state, settings_state, action, engine);
}

/// 설정 모달의 액션을 적용한다 (키 입력 해석과 분리 — 테스트에서 직접 호출).
pub(super) fn apply_settings_action(
    state: &mut AppState,
    mut settings_state: SettingsState,
    action: SettingsAction,
    engine: &mut SearchEngine,
) {
    match action {
        SettingsAction::None => {
            // Put it back
            state.settings = Some(settings_state);
        }
        SettingsAction::Close => {
            // Discard unsaved changes, close modal
            state.settings = None;
        }
        SettingsAction::Save { needs_rebuild } => {
            // 저장이 **성공한 뒤에만** 메모리 상태를 바꾼다.
            // 예전에는 state.config를 먼저 교체하고 dirty를 내린 다음 저장해서,
            // 저장이 실패하면 파일은 옛 값인데 메모리는 새 값이고 편집창은
            // "저장됨"으로 보이는 불일치가 남았다. 파생 필드 동기화도 건너뛰어
            // 메모리 안에서까지 어긋났다.
            let edited = settings_state.config.clone();
            let Some(path) = state
                .config
                .config_path
                .clone()
                .or_else(|| edited.config_path.clone())
            else {
                state.status_message = Some("[!] Save failed: 설정 경로를 알 수 없습니다".into());
                state.settings = Some(settings_state);
                return;
            };

            // 모달이 열려 있는 동안 다른 곳(CLI·데몬·데스크톱)에서 바뀐 값을
            // 되돌리지 않도록, 디스크의 최신 설정 위에 **이 모달에서 실제로
            // 바뀐 항목만** 얹는다. 예전에는 편집본 전체를 저장해서, 창을 오래
            // 열어 둘수록 남의 변경을 덮어쓸 창이 넓어졌다.
            let before = state.config.clone();
            let saved = kmd_core::Config::update_and_save(&path, |latest| {
                // **모든** 편집 가능 키를 훑는다. registry_keys()만 보면 명시적
                // arm으로 처리되는 키(everything_path·keymap.*·각종 providers/
                // prefixes 11개)가 빠져, 화면에서 바꿔도 저장되지 않는다.
                // kind_weights도 여기서 키 단위로 처리되므로 구조체를 통째로
                // 대입하지 않는다 — 대입하면 남이 바꾼 다른 가중치를 되돌린다.
                for key in kmd_core::Config::all_editable_keys() {
                    let (old, new) = (before.get_value(key), edited.get_value(key));
                    if old != new {
                        if let Some(v) = new {
                            if let Err(e) = latest.set_value(key, &v) {
                                tracing::warn!("설정 '{key}' 적용 실패: {e}");
                            }
                        }
                    }
                }
                // 리스트 편집 UI가 직접 다루는 항목 (키 문자열로 왕복하지 않는다)
                if before.launcher.search_paths != edited.launcher.search_paths {
                    latest.launcher.search_paths = edited.launcher.search_paths.clone();
                }
                if before.launcher.ignore_patterns != edited.launcher.ignore_patterns {
                    latest.launcher.ignore_patterns = edited.launcher.ignore_patterns.clone();
                }
            });

            let saved = match saved {
                Ok(c) => c,
                Err(e) => {
                    state.status_message = Some(format!("[!] Save failed: {}", e));
                    state.settings = Some(settings_state); // dirty 유지 — 아직 저장 전이다
                    return;
                }
            };

            // 여기부터는 파일이 확실히 새 값이다. 메모리도 저장된 것으로 맞춘다.
            settings_state.dirty = false;

            // Apply immediate settings — config 파생 필드는 한 곳에서 다시 채운다.
            state.replace_config(saved);
            engine.set_kind_weights(state.config.launcher.kind_weights.clone());

            if needs_rebuild {
                let use_emoji = state.config.general.emoji_icons;
                state.use_emoji = use_emoji;
                let (bin_path, json_path) = crate::cmd::index_cache_paths();
                let _ = std::fs::remove_file(&json_path);
                let _ = std::fs::remove_file(&bin_path);
                let index = kmd_core::Index::build(&state.config.launcher, use_emoji);
                kmd_core::index::store::save_both(&index, &bin_path, &json_path);
                state.total_items = index.items.len();
                engine.load(index.items);

                state.status_message = Some(format!(
                    "\u{2705} Settings saved. Index rebuilt ({} items)",
                    state.total_items
                ));
            } else {
                state.status_message = Some("\u{2705} Settings saved".to_string());
            }

            // Close modal after save
            state.settings = None;
        }
        SettingsAction::Reset => {
            // Reset to defaults
            let default_config = kmd_core::Config::default();
            settings_state.config.launcher.kind_weights = default_config.launcher.kind_weights;
            settings_state.config.launcher.search_depth = default_config.launcher.search_depth;
            settings_state.config.launcher.max_results = default_config.launcher.max_results;
            settings_state.config.launcher.ignore_patterns =
                default_config.launcher.ignore_patterns;
            settings_state.config.launcher.index_directories =
                default_config.launcher.index_directories;
            settings_state.config.launcher.file_search_provider =
                default_config.launcher.file_search_provider;
            settings_state.config.launcher.multi_llm_providers =
                default_config.launcher.multi_llm_providers;
            settings_state.config.launcher.multi_llm_prefixes =
                default_config.launcher.multi_llm_prefixes;
            settings_state.config.launcher.multi_web_providers =
                default_config.launcher.multi_web_providers;
            settings_state.config.launcher.multi_web_prefixes =
                default_config.launcher.multi_web_prefixes;
            settings_state.config.launcher.spell_providers =
                default_config.launcher.spell_providers;
            settings_state.config.launcher.spell_prefixes = default_config.launcher.spell_prefixes;
            settings_state.config.launcher.translate_providers =
                default_config.launcher.translate_providers;
            settings_state.config.launcher.translate_prefixes =
                default_config.launcher.translate_prefixes;
            settings_state.config.launcher.keymap = default_config.launcher.keymap;
            settings_state.config.general = default_config.general;
            settings_state.config.keybindings = default_config.keybindings;
            settings_state.dirty = true;
            state.settings = Some(settings_state);
        }
    }
}

/// 디스크의 최신 설정에 변경분만 얹어 저장하고, 성공하면 메모리 캐시를
/// 저장된 설정으로 교체한다.
///
/// 직접 `state.config`를 고치고 `save()`하면 두 가지가 깨진다 —
/// 그 사이 다른 곳에서 바뀐 값을 되돌리고, 저장이 실패해도 메모리에는
/// 변경이 남는다. 설정을 바꾸는 모든 TUI 경로가 이 함수를 쓴다.
pub(super) fn save_config_change(
    state: &mut AppState,
    edit: impl FnOnce(&mut kmd_core::Config),
) -> Result<(), String> {
    let Some(path) = state.config.config_path.clone() else {
        return Err("설정 경로를 알 수 없습니다".to_string());
    };
    match kmd_core::Config::update_and_save(&path, edit) {
        Ok(saved) => {
            state.replace_config(saved);
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    }
}
