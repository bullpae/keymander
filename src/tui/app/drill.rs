//! 폴더 드릴다운 — 진입/복귀와 디렉터리 내용 나열

use super::*;

// ── Drill-Down ───────────────────────────────────────────────────────────────

/// Drill into the selected folder — list its contents
pub(super) fn drill_into_folder(state: &mut AppState) {
    let Some(selected) = state.results.get(state.selected_index) else {
        return;
    };

    if selected.item.kind != ItemKind::Directory {
        return;
    }

    let dir_path = PathBuf::from(&selected.item.path);
    if !dir_path.is_dir() {
        return;
    }

    // Save current state to the drill stack
    state.drill_stack.push(DrillState {
        query: state.query.clone(),
        results: state.results.clone(),
        selected_index: state.selected_index,
        search_mode: state.search_mode,
        parent_drill_path: state.drill_path.clone(),
    });

    state.results = list_directory_contents(&dir_path, state.use_emoji);
    state.selected_index = 0;
    state.drill_path = Some(dir_path);
    state.search_mode = SearchMode::Contains;
    state.query.clear();
    // 캐시된 유효 쿼리도 함께 비운다 — 안 그러면 입력창 플레이스홀더 판정이 낡은 값을 본다
    state.refresh_effective_query();
}

/// Go back from drill-down to the previous state
pub(super) fn drill_back(state: &mut AppState) {
    if let Some(prev) = state.drill_stack.pop() {
        state.query = prev.query;
        state.results = prev.results;
        state.selected_index = prev
            .selected_index
            .min(state.results.len().saturating_sub(1));
        state.search_mode = prev.search_mode;
        state.drill_path = prev.parent_drill_path;
        state.refresh_effective_query();
    }
}

/// List contents of a directory as SearchResults, sorted: directories first, then files
///
/// 폴더 읽기는 `:f`와 같은 `folder_search::list_dir`를 쓴다 — 상한(잘리면 안내),
/// OS별 숨김 규칙(Windows 숨김 속성), stat 절약이 함께 따라온다.
pub(super) fn list_directory_contents(dir: &Path, use_emoji: bool) -> Vec<SearchResult> {
    let mut directories = Vec::new();
    let mut files = Vec::new();

    let Ok(listing) = kmd_core::folder_search::list_dir(dir, kmd_core::folder_search::MAX_ENTRIES)
    else {
        return Vec::new();
    };

    for entry in listing.entries {
        let is_dir = entry.is_dir;
        let item = kmd_core::IndexItem {
            icon: if is_dir {
                dir_icon(use_emoji)
            } else {
                icon_for_path(&entry.path, use_emoji)
            },
            name: entry.name,
            path: entry.path.to_string_lossy().into_owned(),
            kind: if is_dir {
                ItemKind::Directory
            } else {
                ItemKind::File
            },
            source: Source::FileProvider,
            keywords: String::new(),
            icon_path: None,
        };

        let result = SearchResult {
            item,
            score: SCORE_DIR_LISTING,
        };

        if is_dir {
            directories.push(result);
        } else {
            files.push(result);
        }
    }

    let sort_by_name = |a: &SearchResult, b: &SearchResult| {
        a.item
            .name
            .as_bytes()
            .iter()
            .map(|b| b.to_ascii_lowercase())
            .cmp(
                b.item
                    .name
                    .as_bytes()
                    .iter()
                    .map(|b| b.to_ascii_lowercase()),
            )
    };
    directories.sort_by(sort_by_name);
    files.sort_by(sort_by_name);

    directories.extend(files);
    // 잘렸으면 알린다 — "이게 전부"와 "앞부분만 봤다"는 다르다
    if listing.truncated {
        directories.push(kmd_core::folder_search::truncated_notice(
            &dir.to_string_lossy(),
        ));
    }
    directories
}
