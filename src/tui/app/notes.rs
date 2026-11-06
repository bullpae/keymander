//! `:m` 메모 (TUI). 판단은 `kmd_core::notes`, 여기는 화면 (docs/23)

use super::*;

use kmd_core::notes::{self, EditStart};

pub(super) fn handle_notes_query(query: &str, state: &mut AppState) {
    let items = state.notes.results(query, state.use_emoji);
    state.results = items_to_results(items, SCORE_CALC);
    state.search_mode = SearchMode::Contains;
    state.selected_index = 0;
}

/// Enter. TUI 는 창을 닫지 않는다 — 저장해도 목록으로 돌아가 다음 메모를 받는다
pub(super) fn commit_note_command(
    state: &mut AppState,
    engine: &mut SearchEngine,
    db: Option<&kmd_core::Database>,
    target: Option<String>,
) {
    let query = kmd_core::query_prefix::normalize_slash_command(state.query.trim())
        .unwrap_or_else(|| state.query.trim().to_string());
    let applied = match state.notes.commit(&query, target.as_deref()) {
        Ok(a) => a,
        Err(e) => {
            state.status_message = Some(format!("\u{274C} {e}"));
            return;
        }
    };
    if let Some(text) = &applied.copy {
        if let Err(e) = kmd_core::clipboard::set_text(text) {
            state.status_message = Some(format!("클립보드 복사 실패: {e}"));
            return;
        }
    }
    if let Some(next) = applied.next_query {
        state.query = next;
        state.refresh_effective_query();
    }
    update_search(state, engine, db);
    state.status_message = Some(applied.message);
}

fn in_notes(state: &AppState) -> bool {
    kmd_core::query_prefix::prefix_of(state.query.trim())
        == kmd_core::query_prefix::QueryPrefix::Notes
}

fn selected_id(state: &AppState) -> Option<String> {
    let item = &state.results.get(state.selected_index)?.item;
    notes::selected_id(item).map(str::to_string)
}

/// Ctrl+Del / Ctrl+D. 메모 행이 아니면 false
pub(super) fn delete_selected(
    state: &mut AppState,
    engine: &mut SearchEngine,
    db: Option<&kmd_core::Database>,
) -> bool {
    let Some(id) = selected_id(state) else {
        return false;
    };
    let result = state.notes.delete(&id);
    update_search(state, engine, db);
    state.status_message = Some(result.unwrap_or_else(|e| format!("\u{274C} {e}")));
    true
}

/// Ctrl+Z. `:m` 이 아니거나 되돌릴 것이 없으면 false
pub(super) fn undo_delete(
    state: &mut AppState,
    engine: &mut SearchEngine,
    db: Option<&kmd_core::Database>,
) -> bool {
    if !in_notes(state) {
        return false;
    }
    let Some(result) = state.notes.undo_delete() else {
        return false;
    };
    update_search(state, engine, db);
    state.status_message = Some(result.unwrap_or_else(|e| format!("\u{274C} {e}")));
    true
}

/// Ctrl+E. 메모 행이 아니면 false
pub(super) fn edit_selected(
    state: &mut AppState,
    engine: &mut SearchEngine,
    db: Option<&kmd_core::Database>,
) -> bool {
    let Some(id) = selected_id(state) else {
        return false;
    };
    match state.notes.begin_edit(&id) {
        Ok(EditStart::Inline(query)) => {
            state.query = query;
            state.refresh_effective_query();
            update_search(state, engine, db);
        }
        Ok(EditStart::External(path)) => {
            state.status_message = Some(
                match kmd_core::action::open_with_system(&path.to_string_lossy()) {
                    kmd_core::action::ActionResult::Error(e) => {
                        format!("\u{274C} {} 열기 실패: {e}", path.display())
                    }
                    _ => format!("편집기로 열었습니다: {}", path.display()),
                },
            );
        }
        Err(e) => state.status_message = Some(format!("\u{274C} {e}")),
    }
    true
}
