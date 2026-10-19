//! 검색 — 질의 프리픽스별 결과 구성과 실행 이력 표시

use super::*;

// ── Search ───────────────────────────────────────────────────────────────────

/// ":e"/"​:emoji" 별칭 뒤에 공백이 와서 이모지 **키워드 입력이 시작**됐는지.
/// 공백 전(":e")에는 아직 /exit, :emoji처럼 e로 시작하는 다른 입력일 수
/// 있으므로 내장 한글 조합을 켜면 안 된다.
fn emoji_keyword_started(query: &str) -> bool {
    query.contains(' ')
}

/// Update search results based on current query (including composing char)
pub(super) fn update_search(
    state: &mut AppState,
    engine: &mut SearchEngine,
    db: Option<&kmd_core::Database>,
) {
    // Borrow the cached effective query (no allocation)
    let query = state.effective_query().to_owned();

    if query.is_empty() {
        return handle_empty_query(state, db);
    }

    // Typing while in drill mode exits drill and does a full search
    if state.drill_path.is_some() {
        state.drill_path = None;
        state.drill_stack.clear();
    }

    // /help → :help 정규화 — 표시되는 쿼리는 그대로, 디스패치만 : 형태로
    let query = kmd_core::query_prefix::normalize_slash_command(&query).unwrap_or(query);

    let prefix = kmd_core::query_prefix::prefix_of(&query);

    // :e / :emoji 키워드 입력에서만 내장 한글 조합 자동 활성화.
    // 별칭 뒤에 공백이 온 뒤(":e fire")부터 켠다 — 쿼리가 ":e"인 순간 켜면
    // /exit, :emoji처럼 e로 시작하는 더 긴 명령을 치는 도중 다음 키가
    // 자모로 조합되는 오입력이 발생한다 (/exit → /eㅌit).
    if prefix == QueryPrefix::Emoji && emoji_keyword_started(&query) {
        if !state.hangul_mode && !state.hangul_auto {
            state.hangul_mode = true;
            state.hangul_auto = true;
        }
    } else if state.hangul_auto {
        // Left emoji keyword — auto-deactivate hangul mode
        flush_composer(state);
        state.hangul_mode = false;
        state.hangul_auto = false;
    }

    match prefix {
        QueryPrefix::Web => handle_web_query(&query, state),
        QueryPrefix::Transform => handle_transform_query(&query, state),
        QueryPrefix::Prompt => handle_prompt_query(&query, state),
        QueryPrefix::Calc => handle_calc_query(&query, state),
        QueryPrefix::Emoji => handle_emoji_query(&query, state),
        QueryPrefix::Settings => handle_settings_query(state),
        QueryPrefix::Help => handle_help_query(state),
        QueryPrefix::Version => handle_version_query(state),
        QueryPrefix::Shell => handle_shell_query(&query, state),
        QueryPrefix::Keymap => handle_keymap_query(&query, state),
        QueryPrefix::Keys => handle_keys_query(state),
        QueryPrefix::FolderSearch => handle_folder_search(&query, state),
        QueryPrefix::Scope => super::scope::handle_scope_query(&query, state, engine),
        QueryPrefix::ContentSearch => handle_content_search(&query, state, db),
        // 클립보드 히스토리는 데몬+데스크톱 기능이다 (TUI는 상주 감시가 없음).
        // TUI에서는 일반 검색으로 흘려보낸다.
        QueryPrefix::Clipboard => handle_main_search(&query, state, engine),
        QueryPrefix::General => {
            handle_main_search(&query, state, engine);
            // 범위 밖 폴더 경로를 그대로 쳤으면 맨 위에 "검색 범위에 추가" (docs/18 E1)
            if let Some(item) = kmd_core::scope::path_add_suggestion(
                &state.config.launcher,
                &query,
                state.use_emoji,
            ) {
                state.results.insert(0, SearchResult { item, score: 0 });
            }
            // 오타/미지원 : 명령 안내를 최상단에 표시 (검색 폴스루는 유지)
            if let Some(hint) =
                kmd_core::query_prefix::unknown_command_hint(&query, state.use_emoji)
            {
                state.results.insert(
                    0,
                    SearchResult {
                        item: hint,
                        score: 0,
                    },
                );
                state.selected_index = 0;
            }
        }
    }
}

/// Empty query: show drill directory contents or recent history
fn handle_empty_query(state: &mut AppState, db: Option<&kmd_core::Database>) {
    state.results.clear();
    state.selected_index = 0;

    if let Some(ref path) = state.drill_path {
        let emoji = state.use_emoji;
        state.results = list_directory_contents(path, emoji);
    } else if let Some(db) = db {
        load_history_into_results(state, db);
    }
}

/// Handle @prefix web service queries — classify_web_query 통합 분류기 사용
fn handle_web_query(query: &str, state: &mut AppState) {
    let (items, listing) = web::web_query_items(query, &state.config.launcher, state.use_emoji);
    let score = if listing {
        SCORE_WEB_LIST
    } else {
        SCORE_WEB_SEARCH
    };
    state.results = items_to_results(items, score);
    state.search_mode = SearchMode::Contains;
    state.selected_index = 0;
}

/// Handle :t / :transform prefix (클립보드 변환 명령)
pub(super) fn handle_transform_query(query: &str, state: &mut AppState) {
    use kmd_core::transform;

    match transform::parse_transform_query_or_clipboard(query) {
        Some(tq) => {
            // 검색 중에는 열지 않는다 — 실행 항목만 보여주고 Enter에서 연다.
            // (데스크톱과 같은 규칙: "검색은 순수하게, 실행은 Enter에서")
            let count = transform::build_transform_urls(
                &tq,
                &state.config.launcher.spell_providers,
                &state.config.launcher.translate_providers,
            )
            .len();
            let item = transform::run_item(&tq, count, state.use_emoji);
            state.results = items_to_results(std::iter::once(item), SCORE_CALC);
            state.search_mode = SearchMode::Contains;
            state.selected_index = 0;
        }
        None => {
            // `:t` 만 입력 → 도움말
            let items = transform::help_items(state.use_emoji);
            state.results = items_to_results(items, SCORE_CALC);
            state.search_mode = SearchMode::Contains;
            state.selected_index = 0;
        }
    }
}

/// Handle :prompt / :pt prefix (프롬프트 템플릿 관리)
/// `:prompt` — 입력 중에는 미리보기만 한다. add/rm 확정은 Enter
/// (`execute_selected` → `commit_prompt_command`)에서만 일어난다.
/// 캐시된 설정을 쓴다 — 매 키 입력마다 디스크를 읽지 않는다.
pub(super) fn handle_prompt_query(query: &str, state: &mut AppState) {
    let cmd = kmd_core::prompt::parse_command(query);
    let items = kmd_core::prompt::command_items(
        &cmd,
        &state.config.launcher.prompt_templates,
        state.use_emoji,
    );
    state.results = items_to_results(items, SCORE_CALC);
    state.search_mode = SearchMode::Contains;
    state.selected_index = 0;
}

/// Handle :calc prefix (explicit calculator mode)
fn handle_calc_query(query: &str, state: &mut AppState) {
    let items = kmd_core::query_prefix::calc_items(query, state.use_emoji);
    state.results = items_to_results(items, SCORE_CALC);
    state.selected_index = 0;
}

/// Handle :emoji or :e prefix (emoji search)
fn handle_emoji_query(query: &str, state: &mut AppState) {
    let items = kmd_core::query_prefix::emoji_items(query);
    state.results = items_to_results(items, SCORE_CALC);
    state.selected_index = 0;
}

/// Handle :help / :h prefix — 공용 도움말 항목 표시 (Enter로 시작 쿼리 전환)
pub(super) fn handle_help_query(state: &mut AppState) {
    let items = help_items_for_tui(state.use_emoji);
    state.results = items_to_results(items, SCORE_CALC);
    state.search_mode = SearchMode::Contains;
    state.selected_index = 0;
}

/// TUI에 노출할 도움말 항목 — 클립보드 히스토리는 데몬+데스크톱 전용 흐름이라
/// 숨긴다. 공용 COMMANDS 레지스트리의 항목을 그대로 노출하면 선택 시 시작
/// 쿼리 `;`가 TUI에서는 일반 파일 검색으로 흘러가(위 QueryPrefix::Clipboard
/// 라우팅 참고) 도움말이 안내한 것과 다른 동작이 된다.
fn help_items_for_tui(use_emoji: bool) -> Vec<kmd_core::IndexItem> {
    kmd_core::query_prefix::help_items(use_emoji)
        .into_iter()
        .filter(|item| {
            !matches!(
                kmd_core::query_prefix::help_query_seed(&item.name),
                Some(seed) if kmd_core::query_prefix::prefix_of(seed) == QueryPrefix::Clipboard
            )
        })
        .collect()
}

/// Handle :version prefix — 버전 정보 표시
fn handle_version_query(state: &mut AppState) {
    let items =
        kmd_core::query_prefix::version_items("kmd", env!("CARGO_PKG_VERSION"), state.use_emoji);
    state.results = items_to_results(items, SCORE_CALC);
    state.search_mode = SearchMode::Contains;
    state.selected_index = 0;
}

/// Handle :set / :settings prefix — Enter로 설정 모달(F2)을 여는 항목 표시
fn handle_settings_query(state: &mut AppState) {
    let item = kmd_core::IndexItem {
        name: "Settings 열기".to_string(),
        path: "Enter로 설정 모달을 엽니다 (단축키 F2)".to_string(),
        kind: ItemKind::SystemCommand,
        source: Source::Plugin,
        icon: if state.use_emoji {
            "\u{2699}\u{FE0F}"
        } else {
            "[SET]"
        }
        .to_string(),
        keywords: kmd_core::activation::TUI_OPEN_SETTINGS_MARKER.to_string(),
        icon_path: None,
    };
    state.results = items_to_results(std::iter::once(item), SCORE_CALC);
    state.search_mode = SearchMode::Contains;
    state.selected_index = 0;
}

/// Handle :keymap / :km prefix — kanata 키맵 제어 항목 표시
pub(super) fn handle_keymap_query(query: &str, state: &mut AppState) {
    let items = kmd_core::query_prefix::keymap_query_items(&state.config, query, state.use_emoji);
    state.results = items_to_results(items, SCORE_CALC);
    state.search_mode = SearchMode::Contains;
    state.selected_index = 0;
}

/// Handle :keys / :k prefix — 키 바인딩 치트시트 표시
fn handle_keys_query(state: &mut AppState) {
    let items = kmd_core::keymap::keybinding_cheatsheet(
        &state.config,
        state.use_emoji,
        kmd_core::keymap::CheatsheetApp::Tui,
    );
    state.results = items_to_results(items, SCORE_CALC);
    state.search_mode = SearchMode::Contains;
    state.selected_index = 0;
}

/// Handle :f prefix — 폴더 지정 즉석 검색 (kmd-core 공용 구현)
fn handle_folder_search(query: &str, state: &mut AppState) {
    state.results = kmd_core::folder_search::folder_search_results(query, state.use_emoji);
    state.search_mode = SearchMode::Contains;
    state.selected_index = 0;
}

/// Handle `?` prefix — 문서 본문 검색 (FTS5, docs/15, kmd-core 공용 구현)
fn handle_content_search(query: &str, state: &mut AppState, db: Option<&kmd_core::Database>) {
    // 캐시된 설정을 쓴다 — `?` 는 매 키 입력마다 도는 경로다 (R2-7)
    let launcher = state.config.launcher.clone();
    state.results = kmd_core::content_index::launcher_results(
        db,
        query,
        state.use_emoji,
        SEARCH_RESULT_LIMIT,
        launcher.content_search.enabled,
    );
    // 빈 질의(`?`만 입력) → 사용법 안내 아래에 "자주 변하는 폴더" 제안 (docs/15 P2)
    if kmd_core::content_index::strip_query(query).is_empty() {
        state
            .results
            .extend(kmd_core::folder_suggest::suggestion_results(
                &launcher,
                state.use_emoji,
                3,
            ));
    }
    state.search_mode = SearchMode::Contains;
    state.selected_index = 0;
}

/// Handle ! prefix (shell commands and quick actions)
fn handle_shell_query(query: &str, state: &mut AppState) {
    // 웹 검색 전환 힌트(`!g rust` → `@g rust`) 삽입까지 core가 담당한다
    let items = kmd_core::query_prefix::shell_items(query, state.use_emoji);
    state.results = items_to_results(items, SCORE_CALC);
    state.search_mode = SearchMode::Contains;
    state.selected_index = 0;
}

/// Main fuzzy search with optional inline calculator and history boost
fn handle_main_search(query: &str, state: &mut AppState, engine: &mut SearchEngine) {
    let (mode, mut results) = engine.search(query, SEARCH_RESULT_LIMIT);
    state.search_mode = mode;

    // URL로 판정된 쿼리: 원본 쿼리로 일반(contains) 검색을 수행해 파일도
    // 계속 찾을 수 있게 하고, "URL 열기" 가상 항목을 맨 위에 추가한다.
    // (Enter는 항상 선택된 항목을 실행 — URL은 이 가상 항목으로 연다)
    if mode == SearchMode::Url {
        let (_, normalized_url) = SearchMode::detect(query);
        results = engine.search_with_mode(SearchMode::Contains, query.trim(), SEARCH_RESULT_LIMIT);
        let url_item = kmd_core::query_prefix::url_open_item(&normalized_url, state.use_emoji);
        results.insert(
            0,
            SearchResult {
                item: url_item,
                score: SCORE_WEB_SEARCH,
            },
        );
    }

    // Inline calculator: prepend result if query looks like math
    if builtin_calc::looks_like_math(query) {
        let calc = builtin_calc::CalcExtension;
        let calc_items = calc.search_with_emoji(query, state.use_emoji);
        let calc_results = items_to_results(calc_items, SCORE_CALC_INLINE);
        results.splice(0..0, calc_results);
    }

    // Apply history boost — 캐시된 맵으로 (키 입력마다 DB 조회 없음)
    kmd_core::history::boost_results_with_map(&mut results, &state.frecency);

    // 결과가 없으면 "검색 범위 밖 폴더" 제안을 보여준다. 빈 화면만 내놓으면
    // 사용자는 왜 안 나오는지 알 수 없다 — 가장 흔한 원인이 범위 누락이다.
    // (데스크톱도 같은 지점에서 같은 제안을 쓴다)
    if results.is_empty() && !query.trim().is_empty() {
        results.extend(kmd_core::folder_suggest::suggestion_results(
            &state.config.launcher,
            state.use_emoji,
            2,
        ));
    }

    state.results = results;
    state.selected_index = 0;
}

// ── History ──────────────────────────────────────────────────────────────────

/// Load recent history into results (for empty query display)
pub(super) fn load_history_into_results(state: &mut AppState, db: &kmd_core::Database) {
    let history = db.query_history(HISTORY_DISPLAY_LIMIT);
    state.results = history
        .into_iter()
        .map(|h| {
            let kind = ItemKind::from_history_tag(&h.item_type).unwrap_or(ItemKind::App);
            let path_buf = PathBuf::from(&h.value);
            let base_icon = match kind {
                ItemKind::Directory => dir_icon(state.use_emoji),
                _ => icon_for_path(&path_buf, state.use_emoji),
            };
            // History prefix: * + first char of base icon
            let first_char = base_icon.chars().next().unwrap_or('?');
            let icon = format!("*{}", first_char);
            SearchResult {
                item: kmd_core::IndexItem {
                    name: h.display,
                    path: h.value,
                    kind,
                    source: Source::FileProvider,
                    icon,
                    keywords: String::new(),
                    icon_path: None,
                },
                score: h.frequency * HISTORY_SCORE_MULTIPLIER,
            }
        })
        .collect();
}
