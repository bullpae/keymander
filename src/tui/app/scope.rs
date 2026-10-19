//! `:scope` — 검색 범위 관리 (TUI). 판단은 `kmd_core::scope`, 여기는 화면·인덱스 갱신 (docs/18)

use super::*;

/// 리빌드 IPC 대기 상한 — 데스크톱과 같은 값
const REINDEX_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// `:scope [경로]` — 범위 목록 또는 경로 자동완성
pub(super) fn handle_scope_query(query: &str, state: &mut AppState, engine: &SearchEngine) {
    let rest = kmd_core::scope::strip_command(query);
    state.results = kmd_core::scope::results(
        &state.config.launcher,
        rest,
        &|p| engine.count_within(p),
        state.use_emoji,
    )
    .into_iter()
    .map(|item| SearchResult { item, score: 0 })
    .collect();
    state.search_mode = SearchMode::Contains;
    state.selected_index = 0;
}

/// 폴더를 범위에 추가하고 바로 인덱싱한다.
pub(super) fn scope_add(state: &mut AppState, engine: &mut SearchEngine, path: &Path) {
    match kmd_core::scope::add(&mut state.config, path) {
        Ok((kmd_core::scope::AddOutcome::AlreadyCovered(_), msg)) => {
            state.status_message = Some(msg);
        }
        Ok((_, _)) => {
            let report = reindex(state, engine).map(|()| {
                format!(
                    "\u{2705} {}",
                    kmd_core::scope::added_message(path, engine.count_within(path))
                )
            });
            state.status_message =
                Some(report.unwrap_or_else(|e| format!("\u{274C} 인덱싱 실패: {e}")));
        }
        Err(e) => state.status_message = Some(format!("\u{274C} {e}")),
    }
}

/// 선택한 범위 폴더를 뺀다 (Ctrl+Del, Ctrl+D — 맥 키보드엔 앞쪽 Delete가 없다).
/// `:scope` 목록 행이 아니면 false.
pub(super) fn scope_remove_selected(state: &mut AppState, engine: &mut SearchEngine) -> bool {
    let Some(path) = state
        .results
        .get(state.selected_index)
        .and_then(|r| {
            r.item
                .keywords
                .strip_prefix(kmd_core::scope::SCOPE_FOLDER_MARKER)
        })
        .map(PathBuf::from)
    else {
        return false;
    };
    match kmd_core::scope::remove(&mut state.config, &path) {
        Ok(msg) => {
            state.scope_undo = Some(path.clone());
            state.status_message = Some(match reindex(state, engine) {
                Ok(()) => format!("{msg} · Ctrl+Z 되돌리기"),
                Err(e) => format!("\u{274C} 인덱싱 실패: {e}"),
            });
        }
        Err(e) => state.status_message = Some(format!("\u{274C} {e}")),
    }
    true
}

/// 방금 뺀 폴더 되돌리기 (Ctrl+Z, `:scope` 화면에서만)
pub(super) fn scope_undo(state: &mut AppState, engine: &mut SearchEngine) -> bool {
    if kmd_core::query_prefix::prefix_of(state.query.trim()) != QueryPrefix::Scope {
        return false;
    }
    let Some(path) = state.scope_undo.take() else {
        return false;
    };
    scope_add(state, engine, &path);
    true
}

/// Tab — 선택한 하위 폴더/추가 후보로 입력을 채운다. 해당 행이 아니면 None.
pub(super) fn scope_tab_seed(state: &AppState) -> Option<String> {
    let kw = &state.results.get(state.selected_index)?.item.keywords;
    let path = kw
        .strip_prefix(kmd_core::scope::SCOPE_BROWSE_MARKER)
        .or_else(|| kw.strip_prefix(kmd_core::scope::SCOPE_ADD_MARKER))?;
    Some(kmd_core::scope::browse_seed(path))
}

/// 데몬(없으면 직접)으로 다시 인덱싱하고, 새 캐시로 TUI 엔진을 교체한다.
fn reindex(state: &mut AppState, engine: &mut SearchEngine) -> Result<(), String> {
    kmd_core::scope::rebuild_index_now(REINDEX_TIMEOUT)?;
    let (bin_path, json_path) = crate::cmd::index_cache_paths();
    let index = kmd_core::index::store::try_load_cached(
        &bin_path,
        &json_path,
        kmd_core::Index::current_version(),
    )
    .ok_or_else(|| "새 인덱스 캐시를 읽지 못했습니다".to_string())?;
    state.total_items = index.items.len();
    engine.load(index.items);
    Ok(())
}
