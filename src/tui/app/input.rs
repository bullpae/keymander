//! 키 입력 처리 — 단축키, 한글 조합, 붙여넣기

use super::*;

// ── Key Handling ─────────────────────────────────────────────────────────────

/// Handle a key event
pub(super) fn handle_key(
    state: &mut AppState,
    key: crossterm::event::KeyEvent,
    engine: &mut SearchEngine,
    db: Option<&kmd_core::Database>,
) {
    state.status_message = None;

    match (key.code, key.modifiers) {
        // Open settings modal
        (KeyCode::F(2), _) => {
            flush_composer(state);
            // 캐시된 설정으로 연다 — 시작 시 로드한 값과 이후 변경이 모두 반영돼 있다
            state.settings = Some(SettingsState::new(state.config.clone()));
        }

        (KeyCode::Char(' '), KeyModifiers::CONTROL) => {
            flush_composer(state);
            state.hangul_mode = !state.hangul_mode;
            // Manual toggle overrides auto mode
            state.hangul_auto = false;
            update_search(state, engine, db);
        }
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            state.should_quit = true;
        }
        (KeyCode::Esc, _) => handle_escape(state, db),
        (KeyCode::Up, _) => {
            flush_composer(state);
            if state.selected_index > 0 {
                state.selected_index -= 1;
            }
        }
        (KeyCode::Down, _) => {
            flush_composer(state);
            if state.selected_index + 1 < state.results.len() {
                state.selected_index += 1;
            }
        }
        (KeyCode::Tab, _) | (KeyCode::Right, _) => {
            flush_composer(state);
            drill_into_folder(state);
        }
        (KeyCode::Left, _) => {
            flush_composer(state);
            if state.drill_path.is_some() {
                drill_back(state);
            }
        }
        (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
            flush_composer(state);
            state.show_preview = !state.show_preview;
        }
        (KeyCode::Enter, _) => {
            flush_composer(state);
            // Execute the item the user is currently looking at — do NOT
            // re-run search first, as that resets selected_index to 0.
            execute_selected(state, engine, db);
        }
        (KeyCode::Backspace, _) => {
            if state.hangul_mode && state.composer.is_composing() {
                state.composer.backspace();
                state.composing = state.composer.composing();
            } else {
                state.query.pop();
            }
            state.refresh_effective_query();
            update_search(state, engine, db);
        }
        (KeyCode::Char(c), mods) => {
            if mods.contains(KeyModifiers::CONTROL) || mods.contains(KeyModifiers::ALT) {
                return;
            }
            if state.hangul_mode {
                handle_hangul_char(state, c, engine, db);
            } else {
                state.query.push(c);
                state.refresh_effective_query();
                update_search(state, engine, db);
            }
        }
        _ => {}
    }
}

/// Handle Escape: exit drill-down → clear query → quit
fn handle_escape(state: &mut AppState, db: Option<&kmd_core::Database>) {
    flush_composer(state);
    if state.drill_path.is_some() {
        drill_back(state);
    } else if state.query.is_empty() {
        state.should_quit = true;
    } else {
        state.query.clear();
        state.refresh_effective_query();
        state.results.clear();
        state.selected_index = 0;
        // Deactivate auto-hangul when query is cleared
        if state.hangul_auto {
            state.hangul_mode = false;
            state.hangul_auto = false;
        }
        if let Some(db) = db {
            load_history_into_results(state, db);
        }
    }
}

// ── Korean Input ─────────────────────────────────────────────────────────────

/// Handle a character in Korean input mode
fn handle_hangul_char(
    state: &mut AppState,
    c: char,
    engine: &mut SearchEngine,
    db: Option<&kmd_core::Database>,
) {
    if hangul::is_korean_char(c) {
        flush_composer(state);
        state.query.push(c);
    } else if let Some(jamo) = hangul::key_to_jamo(c) {
        let result = state.composer.process(jamo);
        if let Some(committed) = result.committed {
            state.query.push(committed);
        }
        state.composing = result.composing;
    } else {
        flush_composer(state);
        state.query.push(c);
    }
    state.refresh_effective_query();
    update_search(state, engine, db);
}

/// Flush the Hangul composer — commit any composing character to the query
pub(super) fn flush_composer(state: &mut AppState) {
    if let Some(committed) = state.composer.flush() {
        state.query.push(committed);
    }
    state.composing = None;
    state.refresh_effective_query();
}

/// Handle pasted text (clipboard paste or IME-composed text)
pub(super) fn handle_paste(
    state: &mut AppState,
    text: &str,
    engine: &mut SearchEngine,
    db: Option<&kmd_core::Database>,
) {
    flush_composer(state);
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    if clean.is_empty() {
        return;
    }
    state.query.push_str(&clean);
    state.refresh_effective_query();
    update_search(state, engine, db);
}
