//! TUI application state and main event loop

use std::path::{Path, PathBuf};

use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste, KeyCode, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use kmd_core::action;
use kmd_core::hangul::{self, HangulComposer};
use kmd_core::index::{
    files::{dir_icon, icon_for_path},
    ItemKind, Source,
};
use kmd_core::plugin::{builtin_calc, builtin_shell, Extension};
use kmd_core::query_prefix::QueryPrefix;
use kmd_core::search::{SearchEngine, SearchMode, SearchResult};
use kmd_core::web;

use super::event::{AppEvent, EventHandler};
use super::settings::{self, SettingsAction, SettingsState};
use super::theme::Theme;
use super::ui;

// ── Constants ────────────────────────────────────────────────────────────────

/// Max results returned by fuzzy search
const SEARCH_RESULT_LIMIT: usize = 50;

/// Max history entries shown when query is empty
const HISTORY_DISPLAY_LIMIT: usize = 20;

/// Score for web service list items (@ prefix browsing)
const SCORE_WEB_LIST: u32 = 0;

/// Score for a specific web search result
const SCORE_WEB_SEARCH: u32 = 100;

/// Score for explicit calculator results (:calc prefix)
const SCORE_CALC: u32 = 1000;

/// Score for inline calculator results (always on top)
const SCORE_CALC_INLINE: u32 = u32::MAX;

/// Score for directory listing items
const SCORE_DIR_LISTING: u32 = 0;

/// Multiplier for history frequency → score
const HISTORY_SCORE_MULTIPLIER: u32 = 100;

// ── Application State ────────────────────────────────────────────────────────

/// Application state
pub struct AppState {
    /// Current search query (committed text only)
    pub query: String,
    /// Search results
    pub results: Vec<SearchResult>,
    /// Currently selected index
    pub selected_index: usize,
    /// Total indexed items
    pub total_items: usize,
    /// Show preview panel
    pub show_preview: bool,
    /// Preview panel width percentage (from config)
    pub preview_width_percent: u16,
    /// Current search mode
    search_mode: SearchMode,
    /// Whether to quit
    should_quit: bool,
    /// Whether to quit after launch
    quit_on_launch: bool,
    /// Korean (Hangul) input mode
    pub hangul_mode: bool,
    /// Whether hangul mode was auto-activated (e.g. by :emoji prefix)
    pub hangul_auto: bool,
    /// Currently composing character (during Korean input)
    pub composing: Option<char>,
    /// Hangul composition engine
    composer: HangulComposer,
    /// Folder drill-down stack
    drill_stack: Vec<DrillState>,
    /// Current drill-down directory path (None = normal search mode)
    pub drill_path: Option<PathBuf>,
    /// Temporary status message (e.g. "Copied to clipboard")
    pub status_message: Option<String>,
    /// Settings modal (None = hidden, Some = active)
    pub settings: Option<SettingsState>,
    /// Portable mode indicator
    pub is_portable: bool,
    /// Use emoji icons (mirrors config.general.emoji_icons)
    pub use_emoji: bool,
    /// Selected LLM providers used by @llm multi prompt.
    pub selected_llm_providers: Vec<String>,
    /// Command aliases used for multi LLM query.
    pub multi_llm_prefixes: Vec<String>,
    /// LLM 오토파일럿(데몬 키 주입 자동 제출) opt-in 여부.
    pub llm_autopilot: bool,
    /// Selected search engines used by @msearch multi web.
    pub selected_multi_web_providers: Vec<String>,
    /// Command aliases used for multi web query.
    pub multi_web_prefixes: Vec<String>,
    /// Providers used for spelling check.
    pub spell_providers: Vec<String>,
    /// Command aliases used for spelling check.
    pub spell_prefixes: Vec<String>,
    /// Providers used for translation.
    pub translate_providers: Vec<String>,
    /// Command aliases used for translation.
    pub translate_prefixes: Vec<String>,
    /// 로드된 설정 — **키 입력마다 디스크를 다시 읽지 않기 위한 캐시**.
    /// 예전에는 `:keys`/`:keymap`/`:prompt`/`?` 핸들러가 매 키 입력마다
    /// `load_config()`로 config.toml을 읽었다 (데스크톱은 캐시를 쓴다).
    /// 설정을 바꾸는 경로는 이 값을 함께 갱신해 캐시가 어긋나지 않게 한다.
    pub config: kmd_core::Config,
    /// Cached effective query (query + composing char), updated on every input change
    cached_effective_query: String,
    /// Whether the UI needs to be redrawn
    pub dirty: bool,
}

/// Saved state for returning from a folder drill-down
struct DrillState {
    query: String,
    results: Vec<SearchResult>,
    selected_index: usize,
    search_mode: SearchMode,
    parent_drill_path: Option<PathBuf>,
}

impl AppState {
    /// `config`에서 파생되는 필드들을 다시 채운다.
    ///
    /// 이 필드들은 렌더·핸들러 경로에서 자주 읽혀 캐시로 두고 있다. 정본은 항상
    /// `self.config`이고, 설정이 바뀌는 경로(설정 모달 저장 등)는 config를 고친 뒤
    /// 이 함수 하나만 부르면 된다 — 예전에는 저장 시 12줄을 손으로 재대입해서
    /// 필드가 늘 때마다 누락되기 쉬웠다. `emoji_icons`는 인덱스 재빌드와 함께
    /// 다뤄야 해서 호출자가 따로 처리한다.
    fn sync_config_mirrors(&mut self) {
        let general = &self.config.general;
        let launcher = &self.config.launcher;
        self.show_preview = general.show_preview;
        self.preview_width_percent = general.preview_width_percent;
        self.quit_on_launch = launcher.quit_on_launch;
        self.selected_llm_providers = launcher.multi_llm_providers.clone();
        self.multi_llm_prefixes = launcher.multi_llm_prefixes.clone();
        self.llm_autopilot = launcher.llm_autopilot;
        self.selected_multi_web_providers = launcher.multi_web_providers.clone();
        self.multi_web_prefixes = launcher.multi_web_prefixes.clone();
        self.spell_providers = launcher.spell_providers.clone();
        self.spell_prefixes = launcher.spell_prefixes.clone();
        self.translate_providers = launcher.translate_providers.clone();
        self.translate_prefixes = launcher.translate_prefixes.clone();
    }

    pub fn search_mode_label(&self) -> &str {
        self.search_mode.label()
    }

    /// Get the effective query for display and search (query + composing char).
    /// Returns a borrowed `&str` from the internal cache — no allocation.
    pub fn effective_query(&self) -> &str {
        &self.cached_effective_query
    }

    /// Rebuild the cached effective query from `query` + optional `composing` char.
    /// Must be called whenever `query` or `composing` changes.
    fn refresh_effective_query(&mut self) {
        self.cached_effective_query.clear();
        self.cached_effective_query.push_str(&self.query);
        if let Some(c) = self.composing {
            self.cached_effective_query.push(c);
        }
    }

    /// Mark the UI as needing a redraw.
    fn mark_dirty(&mut self) {
        self.dirty = true;
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Wrap items into SearchResults with a uniform score
fn items_to_results(
    items: impl IntoIterator<Item = kmd_core::IndexItem>,
    score: u32,
) -> Vec<SearchResult> {
    items
        .into_iter()
        .map(|item| SearchResult { item, score })
        .collect()
}

// ── Main Loop ────────────────────────────────────────────────────────────────

/// Run the TUI application.
///
/// `instance_guard` is the single-instance RAII guard obtained in `main()`.
/// The event loop checks it on every tick for external quit signals.
///
/// `center_window` — if true, re-centre the console after entering the
/// alternate screen buffer, ensuring a stable position on hotkey launch.
pub fn run_app(
    instance_guard: Option<kmd_core::single_instance::Guard>,
    _show_on_ready: bool,
) -> color_eyre::Result<()> {
    // Load config and build index
    // mut는 아래 #[cfg(windows)] 이모지 폴백 분기에서만 필요하다.
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut config = crate::cmd::load_config()?;

    // conhost(레거시 콘솔)에서는 이모지 렌더링 불가 → ASCII 폴백 자동 적용
    #[cfg(windows)]
    if config.general.emoji_icons && !crate::win_console::is_modern_terminal() {
        tracing::info!("Legacy console detected — emoji_icons auto-disabled");
        config.general.emoji_icons = false;
    }

    let index = crate::cmd::load_or_build_index(&config.launcher, config.general.emoji_icons);
    let db = crate::cmd::open_db().ok();

    // Initialize search engine with kind weights
    let mut engine = SearchEngine::new();
    engine.set_kind_weights(config.launcher.kind_weights.clone());
    let total_items = index.items.len();
    engine.load(index.items);

    // Initialize state — config 파생 필드는 sync_config_mirrors가 채운다.
    let mut state = AppState {
        query: String::new(),
        results: Vec::new(),
        selected_index: 0,
        total_items,
        show_preview: false,
        preview_width_percent: 0,
        search_mode: SearchMode::Fuzzy,
        should_quit: false,
        quit_on_launch: false,
        hangul_mode: false,
        hangul_auto: false,
        composing: None,
        composer: HangulComposer::new(),
        drill_stack: Vec::new(),
        drill_path: None,
        status_message: None,
        settings: None,
        is_portable: kmd_core::portable::is_portable(),
        use_emoji: config.general.emoji_icons,
        selected_llm_providers: Vec::new(),
        multi_llm_prefixes: Vec::new(),
        llm_autopilot: false,
        selected_multi_web_providers: Vec::new(),
        multi_web_prefixes: Vec::new(),
        spell_providers: Vec::new(),
        spell_prefixes: Vec::new(),
        translate_providers: Vec::new(),
        translate_prefixes: Vec::new(),
        config: config.clone(),
        cached_effective_query: String::new(),
        dirty: true,
    };
    state.sync_config_mirrors();

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let theme = Theme::default();
    let events = EventHandler::new(config.general.render_fps);

    // Initial empty results: show history
    if let Some(ref db) = db {
        load_history_into_results(&mut state, db);
    }

    // Draw the first frame while the window is still hidden.
    // This fills the console buffer with TUI content so the user
    // sees a fully rendered screen the moment the window appears.
    terminal.draw(|frame| {
        ui::render(frame, &state, &theme);
    })?;

    // Show the terminal window (it was hidden in main() for hotkey launch).
    // Centering is handled by Windows Terminal's `centerOnLaunch` setting.
    #[cfg(windows)]
    if _show_on_ready {
        crate::win_console::show();
    }

    // Main loop
    loop {
        if state.dirty {
            terminal.draw(|frame| {
                ui::render(frame, &state, &theme);
                // Render settings modal overlay on top
                if let Some(ref settings_state) = state.settings {
                    settings::render::render_modal(frame, frame.area(), settings_state, &theme);
                }
            })?;
            state.dirty = false;
        }

        if state.should_quit {
            break;
        }

        match events.next()? {
            AppEvent::Key(key) => {
                state.mark_dirty();
                // Route keys to settings if modal is open
                if state.settings.is_some() {
                    handle_settings_key_event(&mut state, key, &mut engine);
                } else {
                    handle_key(&mut state, key, &mut engine, db.as_ref());
                }
            }
            AppEvent::Paste(text) => {
                state.mark_dirty();
                if state.settings.is_none() {
                    handle_paste(&mut state, &text, &mut engine, db.as_ref());
                }
            }
            AppEvent::Resize => {
                state.mark_dirty();
            }
            AppEvent::Tick => {
                // Check if another instance requested us to quit
                if let Some(ref guard) = instance_guard {
                    if guard.should_quit() {
                        guard.consume_quit_signal();
                        state.should_quit = true;
                        state.mark_dirty();
                    }
                }
                // Re-render on tick only when status_message is set (for auto-clear)
                if state.status_message.is_some() {
                    state.mark_dirty();
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    Ok(())
}

// ── Settings Integration ─────────────────────────────────────────────────────

/// Handle a key event when the settings modal is open
fn handle_settings_key_event(
    state: &mut AppState,
    key: crossterm::event::KeyEvent,
    engine: &mut SearchEngine,
) {
    // Take ownership of settings state temporarily
    let Some(mut settings_state) = state.settings.take() else {
        return;
    };

    let action = settings::handle_settings_key(&mut settings_state, key);

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
            // 편집 결과를 정본(`state.config`)에 반영한다.
            // 예전에는 메인 루프가 소유한 `config`와 이 캐시 두 벌이 공존해
            // 한쪽만 고치면 `:keys`/`:keymap`이 옛 설정을 보게 됐다 — 이제 한 벌이다.
            state.config = settings_state.config.clone();
            settings_state.dirty = false;

            // Save to file
            if let Err(e) = state.config.save() {
                state.status_message = Some(format!("[!] Save failed: {}", e));
                state.settings = Some(settings_state);
                return;
            }

            // Apply immediate settings — config 파생 필드는 한 곳에서 다시 채운다.
            state.sync_config_mirrors();
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

// ── Key Handling ─────────────────────────────────────────────────────────────

/// Handle a key event
fn handle_key(
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
fn flush_composer(state: &mut AppState) {
    if let Some(committed) = state.composer.flush() {
        state.query.push(committed);
    }
    state.composing = None;
    state.refresh_effective_query();
}

/// Handle pasted text (clipboard paste or IME-composed text)
fn handle_paste(
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

// ── Execute ──────────────────────────────────────────────────────────────────

/// URL 목록을 브라우저로 열고, quit_on_launch 설정 시 종료 플래그 설정
fn open_urls_and_quit(state: &mut AppState, urls: &[String]) {
    for url in urls {
        let _ = action::open_url(url);
    }
    if state.quit_on_launch {
        state.should_quit = true;
    }
}

/// LLM 실행(@gpt/@llm) 라우팅. LLM 쿼리를 처리했으면 true.
/// 오토파일럿 켜짐 + 데몬 위임 성공 시 자동 제출, 아니면 URL/클립보드 폴백.
fn tui_try_llm_launch(state: &mut AppState) -> bool {
    let Some((services, prompt)) = web::parse_any_llm_query(
        &state.query,
        &state.selected_llm_providers,
        &state.multi_llm_prefixes,
    ) else {
        return false;
    };
    if services.is_empty() {
        return false;
    }

    let final_prompt =
        kmd_core::prompt::apply_template(&state.config.launcher.prompt_templates, &prompt);
    let plan = web::build_llm_launch_plan(&services, &final_prompt);
    let has_paste = plan
        .jobs
        .iter()
        .any(|j| matches!(j.method, kmd_core::ipc::LlmInject::PasteEnter));

    if state.llm_autopilot && !plan.jobs.is_empty() {
        let req = kmd_core::ipc::Request::LlmAutopilot {
            jobs: plan.jobs.clone(),
        };
        if kmd_core::ipc::send_request_result(&req).is_ok() {
            for url in &plan.plain_urls {
                let _ = action::open_url(url);
            }
            state.status_message =
                Some("🤖 LLM 오토파일럿에 위임했습니다 (데몬이 자동 제출)".to_string());
            return true;
        }
    }

    // 폴백: 전 서비스 URL + (붙여넣기형 있으면) 클립보드
    if has_paste && !final_prompt.is_empty() {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(&final_prompt);
        }
        state.status_message = Some(
            "✅ 프롬프트를 클립보드에 복사했습니다 (일부 서비스는 붙여넣기/Enter 필요)".to_string(),
        );
    }
    for url in web::llm_plan_all_urls(&plan) {
        let _ = action::open_url(&url);
    }
    true
}

/// `@@ <프롬프트>` 이어서 질문 — 데몬에 위임 (열 URL 없음).
fn tui_send_llm_followup(state: &mut AppState, prompt: &str) {
    let final_prompt =
        kmd_core::prompt::apply_template(&state.config.launcher.prompt_templates, prompt);
    let req = kmd_core::ipc::Request::LlmFollowup {
        prompt: final_prompt,
    };
    match kmd_core::ipc::send_request_result(&req) {
        Ok(kmd_core::ipc::Response::Ok { .. }) => {
            state.status_message = Some("🤖 이어서 질문을 전달했습니다".to_string());
        }
        Ok(kmd_core::ipc::Response::Error { message }) => {
            state.status_message = Some(format!("⚠ {message}"));
        }
        Ok(_) => {}
        Err(_) => {
            state.status_message =
                Some("⚠ 데몬이 실행 중이 아니어서 이어서 질문할 수 없습니다".to_string());
        }
    }
}

/// Execute the currently selected item
fn execute_selected(
    state: &mut AppState,
    engine: &mut SearchEngine,
    db: Option<&kmd_core::Database>,
) {
    // 이어서 질문: `@@ <프롬프트>` → 데몬이 기억한 LLM 창들에 전달 (열 결과 없음)
    if let Some(followup) = web::parse_llm_followup(&state.query) {
        tui_send_llm_followup(state, &followup);
        if state.quit_on_launch {
            state.should_quit = true;
        }
        return;
    }

    // result를 소유값으로 복제 — 이후 state를 가변 대여해도 대여 충돌이 없다
    let Some(result) = state.results.get(state.selected_index).cloned() else {
        return;
    };

    // ── kmd 가상 항목 (도움말/설정/키맵 등 UI 내부 명령) ──
    if result.item.kind == ItemKind::SystemCommand && result.item.keywords.starts_with("kmd:") {
        let keywords = result.item.keywords.clone();
        let item_name = result.item.name.clone();
        let item_path = result.item.path.clone();

        // 도움말 항목 → 시작 쿼리(퀵 템플릿)로 전환
        if keywords.starts_with("kmd:help:") {
            if let Some(seed) = kmd_core::query_prefix::help_query_seed(&item_name) {
                state.query = seed.to_string();
                state.refresh_effective_query();
                state.selected_index = 0;
                update_search(state, engine, db);
            }
            return;
        }
        // 셸 모드의 웹 검색 전환 힌트 (!g → @g)
        if keywords.starts_with("kmd:bang_hint:") {
            state.query = item_path;
            state.refresh_effective_query();
            state.selected_index = 0;
            update_search(state, engine, db);
            return;
        }
        // 미지 명령 안내 → :help 로 이동
        if keywords == "kmd:unknown_cmd" {
            state.query = ":help".to_string();
            state.refresh_effective_query();
            state.selected_index = 0;
            update_search(state, engine, db);
            return;
        }
        // 폴더 제안 → search_paths에 추가 + config 저장 (docs/15 P2)
        if keywords.starts_with(kmd_core::folder_suggest::SUGGEST_MARKER) {
            if let Some(msg) =
                kmd_core::folder_suggest::execute_suggest_action(&mut state.config, &keywords)
            {
                state.status_message = Some(msg);
            }
            state.selected_index = 0;
            update_search(state, engine, db);
            return;
        }
        // 설정 모달 열기 (:set)
        if keywords == "kmd:tui:open_settings" {
            state.settings = Some(SettingsState::new(state.config.clone()));
            return;
        }
        // 키맵 액션 (start/stop/프로파일 전환)
        if keywords.starts_with("kmd:keymap:") && !keywords.ends_with(":noop") {
            if let Some(msg) = kmd_core::keymap::execute_keymap_action(&mut state.config, &keywords)
            {
                state.status_message = Some(msg);
            }
            let query = kmd_core::query_prefix::normalize_slash_command(&state.query)
                .unwrap_or_else(|| state.query.clone());
            handle_keymap_query(&query, state);
            return;
        }
        // 나머지 kmd:* 항목은 안내용(noop) — 실행할 대상이 없다
        return;
    }

    // Calculator result → copy to clipboard
    if result.item.kind == ItemKind::Calculator && !result.item.path.is_empty() {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(&result.item.path);
            state.status_message = Some(format!("\u{2705} Copied: {}", result.item.path));
            // ✅
        }
        return;
    }

    // Emoji result → copy to clipboard
    if result.item.kind == ItemKind::Emoji && !result.item.path.is_empty() {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(&result.item.path);
            state.status_message = Some(format!("\u{2705} Copied: {}", result.item.path));
            // ✅
        }
        return;
    }

    // Shell command — quick action은 백그라운드 실행 후 결과를 클립보드에 복사,
    // 사용자 명령은 새 터미널 창에서 실행한다 (데스크톱과 동일한 UX).
    // 인라인 실행의 10초 타임아웃은 즉답형 quick action에만 적합하다 —
    // `>winget upgrade --all` 같은 장시간 명령을 중간에 죽이지 않는다.
    if result.item.kind == ItemKind::Shell {
        if builtin_shell::ShellExtension::is_quick_action(&result.item.path) {
            let shell_ext = builtin_shell::ShellExtension;
            match shell_ext.execute(&result.item) {
                kmd_core::plugin::ExtensionAction::CopyToClipboard(output) => {
                    // Copy to clipboard and show first line as status
                    let first_line = output.lines().next().unwrap_or("(no output)");
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        let _ = clipboard.set_text(&output);
                    }
                    state.status_message = Some(format!("\u{2705} {}", first_line));
                }
                kmd_core::plugin::ExtensionAction::Display(msg) => {
                    state.status_message = Some(format!("\u{274C} {}", msg)); // ❌
                }
                _ => {}
            }
        } else {
            match builtin_shell::launch_in_terminal(&result.item.path) {
                Ok(()) => {
                    state.status_message =
                        Some(format!("\u{1F4DF} 터미널에서 실행: {}", result.item.path)); // 📟
                    if state.quit_on_launch {
                        state.should_quit = true;
                    }
                }
                Err(e) => {
                    state.status_message = Some(format!("\u{274C} {}", e)); // ❌
                }
            }
        }
        return;
    }

    // LLM 실행(@gpt/@llm) — 오토파일럿 또는 URL 폴백으로 라우팅
    if result.item.kind == ItemKind::WebSearch && tui_try_llm_launch(state) {
        if state.quit_on_launch {
            state.should_quit = true;
        }
        return;
    }

    // 웹 검색 결과 — extract_batch_urls 통합 (LLM 외 msearch/spell/translate)
    if result.item.kind == ItemKind::WebSearch {
        if let Some(urls) = web::extract_batch_urls(&result.item) {
            open_urls_and_quit(state, &urls);
            return;
        }
    }

    // 원시 입력 웹 쿼리 폴백 — classify_web_query 통합 분류기 사용
    {
        let cfg = web::WebQueryConfig {
            spell_prefixes: &state.spell_prefixes,
            translate_prefixes: &state.translate_prefixes,
            multi_llm_prefixes: &state.multi_llm_prefixes,
            multi_llm_ids: &state.selected_llm_providers,
            multi_web_prefixes: &state.multi_web_prefixes,
            multi_web_ids: &state.selected_multi_web_providers,
        };
        match web::classify_web_query(&state.query, &cfg) {
            web::WebQueryResult::MultiLlm(services, q) if !q.is_empty() => {
                let urls: Vec<String> = services
                    .iter()
                    .map(|s| web::build_search_url(s, &q))
                    .collect();
                open_urls_and_quit(state, &urls);
                return;
            }
            web::WebQueryResult::MultiWeb(services, q) if !q.is_empty() => {
                let urls: Vec<String> = services
                    .iter()
                    .map(|s| web::build_search_url(s, &q))
                    .collect();
                open_urls_and_quit(state, &urls);
                return;
            }
            web::WebQueryResult::Spell(q) if !q.is_empty() => {
                let items = web::spell_result_items(&q, &state.spell_providers, state.use_emoji);
                if let Some(first) = items.first() {
                    if let Some(urls) = web::extract_batch_urls(first) {
                        open_urls_and_quit(state, &urls);
                    }
                }
                return;
            }
            web::WebQueryResult::Translate(dir, q) if !q.is_empty() => {
                let items = web::translate_result_items(
                    &q,
                    dir,
                    &state.translate_providers,
                    state.use_emoji,
                );
                if let Some(first) = items.first() {
                    if let Some(urls) = web::extract_batch_urls(first) {
                        open_urls_and_quit(state, &urls);
                    }
                }
                return;
            }
            web::WebQueryResult::Single(service, q) if !q.is_empty() => {
                let url = web::build_search_url(service, &q);
                open_urls_and_quit(state, &[url]);
                return;
            }
            _ => {}
        }
    }

    // Normal execution
    match action::execute(&result) {
        action::ActionResult::Launched => {
            if let Some(db) = db {
                kmd_core::history::record_launch(
                    db,
                    &result.item.kind.to_string(),
                    &result.item.path,
                    Some(&result.item.name),
                );
            }
            if state.quit_on_launch {
                state.should_quit = true;
            }
        }
        action::ActionResult::OpenedUrl(_) => {
            if state.quit_on_launch {
                state.should_quit = true;
            }
        }
        action::ActionResult::NeedsConfirmation(name) => {
            state.status_message = Some(format!("\u{26A0}\u{FE0F} Confirmation needed: {}", name));
            // ⚠️
        }
        action::ActionResult::Error(e) => {
            state.status_message = Some(format!("\u{274C} {}", e)); // ❌
        }
    }
}

// ── Search ───────────────────────────────────────────────────────────────────

/// ":e"/"​:emoji" 별칭 뒤에 공백이 와서 이모지 **키워드 입력이 시작**됐는지.
/// 공백 전(":e")에는 아직 /exit, :emoji처럼 e로 시작하는 다른 입력일 수
/// 있으므로 내장 한글 조합을 켜면 안 된다.
fn emoji_keyword_started(query: &str) -> bool {
    query.contains(' ')
}

/// Update search results based on current query (including composing char)
fn update_search(state: &mut AppState, engine: &mut SearchEngine, db: Option<&kmd_core::Database>) {
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
        QueryPrefix::ContentSearch => handle_content_search(&query, state, db),
        // 클립보드 히스토리는 데몬+데스크톱 기능이다 (TUI는 상주 감시가 없음).
        // TUI에서는 일반 검색으로 흘려보낸다.
        QueryPrefix::Clipboard => handle_main_search(&query, state, engine, db),
        QueryPrefix::General => {
            handle_main_search(&query, state, engine, db);
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
    let emoji = state.use_emoji;
    let cfg = web::WebQueryConfig {
        spell_prefixes: &state.spell_prefixes,
        translate_prefixes: &state.translate_prefixes,
        multi_llm_prefixes: &state.multi_llm_prefixes,
        multi_llm_ids: &state.selected_llm_providers,
        multi_web_prefixes: &state.multi_web_prefixes,
        multi_web_ids: &state.selected_multi_web_providers,
    };

    match web::classify_web_query(query, &cfg) {
        web::WebQueryResult::Spell(q) => {
            state.results = items_to_results(
                web::spell_result_items(&q, &state.spell_providers, emoji),
                SCORE_WEB_SEARCH,
            );
        }
        web::WebQueryResult::Translate(dir, q) => {
            state.results = items_to_results(
                web::translate_result_items(&q, dir, &state.translate_providers, emoji),
                SCORE_WEB_SEARCH,
            );
        }
        web::WebQueryResult::MultiLlm(_svcs, q) => {
            state.results = items_to_results(
                web::multi_llm_result_items(&q, &state.selected_llm_providers, emoji),
                SCORE_WEB_SEARCH,
            );
        }
        web::WebQueryResult::MultiWeb(_svcs, q) => {
            state.results = items_to_results(
                web::multi_web_result_items(&q, &state.selected_multi_web_providers, emoji),
                SCORE_WEB_SEARCH,
            );
        }
        web::WebQueryResult::Single(service, q) => {
            if q.is_empty() {
                state.results =
                    items_to_results(web::list_services_as_items("", emoji), SCORE_WEB_LIST);
            } else {
                let item = web::search_result_item(service, &q, emoji);
                state.results = items_to_results(std::iter::once(item), SCORE_WEB_SEARCH);
            }
        }
        web::WebQueryResult::Browse(filter) => {
            state.results =
                items_to_results(web::list_services_as_items(&filter, emoji), SCORE_WEB_LIST);
        }
    }
    state.search_mode = SearchMode::Contains;
    state.selected_index = 0;
}

/// Handle :t / :transform prefix (클립보드 변환 명령)
fn handle_transform_query(query: &str, state: &mut AppState) {
    use kmd_core::transform;

    match transform::parse_transform_query(query) {
        Some(mut tq) => {
            // 텍스트가 비어있으면 클립보드에서 가져오기
            if tq.text.is_empty() {
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    if let Ok(text) = clipboard.get_text() {
                        tq.text = text;
                    }
                }
                if tq.text.is_empty() {
                    state.status_message = Some("❌ 클립보드가 비어 있습니다".to_string());
                    state.results.clear();
                    state.selected_index = 0;
                    return;
                }
            }

            let urls = transform::build_transform_urls(
                &tq,
                &state.spell_providers,
                &state.translate_providers,
            );
            let kind_label = match &tq.kind {
                transform::TransformKind::Spell => "맞춤법 검사",
                transform::TransformKind::Translate(_) => "번역",
            };
            state.status_message = Some(format!("✅ {} 실행 ({} 서비스)", kind_label, urls.len()));
            open_urls_and_quit(state, &urls);
            state.results.clear();
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
fn handle_prompt_query(query: &str, state: &mut AppState) {
    let sub = query
        .strip_prefix(":prompt")
        .or_else(|| query.strip_prefix(":pt"))
        .unwrap_or("")
        .trim();

    // 캐시된 설정을 쓴다 — 매 키 입력마다 디스크를 읽지 않는다.
    // 아래 add/remove는 캐시를 직접 고치고 저장하므로 캐시가 어긋나지 않는다.
    let templates = state.config.launcher.prompt_templates.clone();

    // :prompt add <name> <body>
    if sub.starts_with("add ") {
        let rest = sub.strip_prefix("add ").unwrap_or("").trim();
        if let Some(pos) = rest.find(char::is_whitespace) {
            let name = &rest[..pos];
            let body = rest[pos..].trim();
            if !kmd_core::prompt::validate_template_name(name) {
                state.status_message =
                    Some("❌ 이름은 영문/숫자/하이픈만 사용 가능 (최대 32자)".to_string());
            } else if body.is_empty() {
                state.status_message = Some("❌ 본문이 비어 있습니다".to_string());
            } else {
                state
                    .config
                    .launcher
                    .prompt_templates
                    .retain(|t| !t.name.eq_ignore_ascii_case(name));
                state
                    .config
                    .launcher
                    .prompt_templates
                    .push(kmd_core::config::PromptTemplate {
                        name: name.to_string(),
                        body: body.to_string(),
                    });
                if let Err(e) = state.config.save() {
                    state.status_message = Some(format!("❌ 저장 실패: {e}"));
                } else {
                    state.status_message = Some(format!("✅ 템플릿 '{name}' 저장됨"));
                }
            }
        } else {
            state.status_message = Some("사용법: :prompt add <name> <body>".to_string());
        }
        state.results.clear();
        state.selected_index = 0;
        return;
    }

    // :prompt remove <name>
    if sub.starts_with("remove ") || sub.starts_with("rm ") || sub.starts_with("del ") {
        let name = sub
            .strip_prefix("remove ")
            .or_else(|| sub.strip_prefix("rm "))
            .or_else(|| sub.strip_prefix("del "))
            .unwrap_or("")
            .trim();
        if name.is_empty() {
            state.status_message = Some("사용법: :prompt remove <name>".to_string());
        } else {
            let before = state.config.launcher.prompt_templates.len();
            state
                .config
                .launcher
                .prompt_templates
                .retain(|t| !t.name.eq_ignore_ascii_case(name));
            if state.config.launcher.prompt_templates.len() < before {
                if let Err(e) = state.config.save() {
                    state.status_message = Some(format!("❌ 저장 실패: {e}"));
                } else {
                    state.status_message = Some(format!("✅ 템플릿 '{name}' 삭제됨"));
                }
            } else {
                state.status_message = Some(format!("❌ 템플릿 '{name}'을 찾을 수 없습니다"));
            }
        }
        state.results.clear();
        state.selected_index = 0;
        return;
    }

    // :prompt list 또는 :prompt (필터링)
    let filter = sub.strip_prefix("list").unwrap_or(sub).trim();
    let items = kmd_core::prompt::list_templates_as_items(&templates, filter, state.use_emoji);
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
fn handle_help_query(state: &mut AppState) {
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
        keywords: "kmd:tui:open_settings".to_string(),
        icon_path: None,
    };
    state.results = items_to_results(std::iter::once(item), SCORE_CALC);
    state.search_mode = SearchMode::Contains;
    state.selected_index = 0;
}

/// Handle :keymap / :km prefix — kanata 키맵 제어 항목 표시
fn handle_keymap_query(query: &str, state: &mut AppState) {
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
fn handle_main_search(
    query: &str,
    state: &mut AppState,
    engine: &mut SearchEngine,
    db: Option<&kmd_core::Database>,
) {
    let (mode, mut results) = engine.search(query, SEARCH_RESULT_LIMIT);
    state.search_mode = mode;

    // URL로 판정된 쿼리: 원본 쿼리로 일반(contains) 검색을 수행해 파일도
    // 계속 찾을 수 있게 하고, "URL 열기" 가상 항목을 맨 위에 추가한다.
    // (Enter는 항상 선택된 항목을 실행 — URL은 이 가상 항목으로 연다)
    if mode == SearchMode::Url {
        let (_, normalized_url) = SearchMode::detect(query);
        results = engine.search_with_mode(SearchMode::Contains, query.trim(), SEARCH_RESULT_LIMIT);
        let url_item = kmd_core::IndexItem {
            name: format!("Open {}", normalized_url),
            path: normalized_url.clone(),
            kind: ItemKind::WebSearch,
            source: Source::Plugin,
            icon: if state.use_emoji {
                "\u{1F310}".to_string() // 🌐
            } else {
                "Ww".to_string()
            },
            keywords: normalized_url,
            icon_path: None,
        };
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

    // Apply history boost
    if let Some(db) = db {
        kmd_core::history::boost_results(&mut results, db);
    }

    state.results = results;
    state.selected_index = 0;
}

// ── Drill-Down ───────────────────────────────────────────────────────────────

/// Drill into the selected folder — list its contents
fn drill_into_folder(state: &mut AppState) {
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
}

/// Go back from drill-down to the previous state
fn drill_back(state: &mut AppState) {
    if let Some(prev) = state.drill_stack.pop() {
        state.query = prev.query;
        state.results = prev.results;
        state.selected_index = prev
            .selected_index
            .min(state.results.len().saturating_sub(1));
        state.search_mode = prev.search_mode;
        state.drill_path = prev.parent_drill_path;
    }
}

/// List contents of a directory as SearchResults, sorted: directories first, then files
fn list_directory_contents(dir: &Path, use_emoji: bool) -> Vec<SearchResult> {
    let mut directories = Vec::new();
    let mut files = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files/dirs
        if name.starts_with('.') {
            continue;
        }

        let is_dir = path.is_dir();
        let path_str = path.to_string_lossy().to_string();

        let item = kmd_core::IndexItem {
            name,
            path: path_str,
            kind: if is_dir {
                ItemKind::Directory
            } else {
                ItemKind::File
            },
            source: Source::FileProvider,
            icon: if is_dir {
                dir_icon(use_emoji)
            } else {
                icon_for_path(&path, use_emoji)
            },
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
    directories
}

// ── History ──────────────────────────────────────────────────────────────────

/// Load recent history into results (for empty query display)
fn load_history_into_results(state: &mut AppState, db: &kmd_core::Database) {
    let history = db.query_history(HISTORY_DISPLAY_LIMIT);
    state.results = history
        .into_iter()
        .map(|h| {
            let kind = match h.item_type.as_str() {
                "App" => ItemKind::App,
                "File" => ItemKind::File,
                "Dir" => ItemKind::Directory,
                "Exe" => ItemKind::Executable,
                "System" => ItemKind::SystemCommand,
                "Web" => ItemKind::WebSearch,
                _ => ItemKind::App,
            };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn test_state() -> AppState {
        AppState {
            query: String::new(),
            results: Vec::new(),
            selected_index: 0,
            total_items: 0,
            show_preview: false,
            preview_width_percent: 40,
            search_mode: SearchMode::Fuzzy,
            should_quit: false,
            quit_on_launch: false,
            hangul_mode: false,
            hangul_auto: false,
            composing: None,
            composer: HangulComposer::new(),
            drill_stack: Vec::new(),
            drill_path: None,
            status_message: None,
            settings: None,
            is_portable: false,
            use_emoji: true,
            selected_llm_providers: Vec::new(),
            multi_llm_prefixes: Vec::new(),
            llm_autopilot: false,
            selected_multi_web_providers: Vec::new(),
            multi_web_prefixes: Vec::new(),
            spell_providers: Vec::new(),
            spell_prefixes: Vec::new(),
            translate_providers: Vec::new(),
            translate_prefixes: Vec::new(),
            config: kmd_core::Config::default(),
            cached_effective_query: String::new(),
            dirty: true,
        }
    }

    // ── config 파생 필드 동기화 ─────────────────────────────────────────
    //
    // 이 필드들은 config의 캐시다. 예전에는 저장 경로에서 12줄을 손으로
    // 재대입해 필드가 늘 때 누락되기 쉬웠다 — 이제 sync_config_mirrors 한
    // 곳이며, 이 테스트가 "config를 고치면 전부 따라온다"를 지킨다.
    #[test]
    fn config_변경은_파생_필드에_모두_반영된다() {
        let mut state = test_state();

        state.config.general.show_preview = true;
        state.config.general.preview_width_percent = 55;
        state.config.launcher.quit_on_launch = true;
        state.config.launcher.llm_autopilot = true;
        state.config.launcher.multi_llm_providers = vec!["claude".into()];
        state.config.launcher.multi_llm_prefixes = vec!["@llm".into()];
        state.config.launcher.multi_web_providers = vec!["google".into()];
        state.config.launcher.multi_web_prefixes = vec!["@ms".into()];
        state.config.launcher.spell_providers = vec!["naver_spell".into()];
        state.config.launcher.spell_prefixes = vec!["@sp".into()];
        state.config.launcher.translate_providers = vec!["papago".into()];
        state.config.launcher.translate_prefixes = vec!["@tr".into()];

        state.sync_config_mirrors();

        assert!(state.show_preview);
        assert_eq!(state.preview_width_percent, 55);
        assert!(state.quit_on_launch);
        assert!(state.llm_autopilot);
        assert_eq!(state.selected_llm_providers, vec!["claude".to_string()]);
        assert_eq!(state.multi_llm_prefixes, vec!["@llm".to_string()]);
        assert_eq!(
            state.selected_multi_web_providers,
            vec!["google".to_string()]
        );
        assert_eq!(state.multi_web_prefixes, vec!["@ms".to_string()]);
        assert_eq!(state.spell_providers, vec!["naver_spell".to_string()]);
        assert_eq!(state.spell_prefixes, vec!["@sp".to_string()]);
        assert_eq!(state.translate_providers, vec!["papago".to_string()]);
        assert_eq!(state.translate_prefixes, vec!["@tr".to_string()]);
    }

    #[test]
    fn 동기화는_기본_설정과도_일치한다() {
        let mut state = test_state();
        state.sync_config_mirrors();
        let cfg = kmd_core::Config::default();
        assert_eq!(state.show_preview, cfg.general.show_preview);
        assert_eq!(
            state.preview_width_percent,
            cfg.general.preview_width_percent
        );
        assert_eq!(
            state.selected_llm_providers,
            cfg.launcher.multi_llm_providers
        );
    }

    fn type_str(state: &mut AppState, engine: &mut SearchEngine, s: &str) {
        for c in s.chars() {
            handle_key(
                state,
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
                engine,
                None,
            );
        }
    }

    // ── 실행 분기 (execute_selected) ────────────────────────────────────
    //
    // 실행 경로는 상태를 크게 흔드는데(쿼리 교체, 모달 열기, 드릴 진입)
    // 지금까지 검증이 없었다. **부작용이 상태 안에서 끝나는 분기만** 다룬다 —
    // 셸 실행·클립보드·브라우저 열기·config 저장은 테스트에서 건드리지 않는다.

    fn sysitem(name: &str, path: &str, keywords: &str) -> SearchResult {
        SearchResult {
            item: kmd_core::IndexItem {
                name: name.to_string(),
                path: path.to_string(),
                kind: ItemKind::SystemCommand,
                source: kmd_core::index::Source::Plugin,
                icon: String::new(),
                keywords: keywords.to_string(),
                icon_path: None,
            },
            score: 0,
        }
    }

    /// 선택 항목 하나를 놓고 실행한다.
    fn run_selected(state: &mut AppState, engine: &mut SearchEngine, item: SearchResult) {
        state.results = vec![item];
        state.selected_index = 0;
        execute_selected(state, engine, None);
    }

    #[test]
    fn 미지_명령_안내를_실행하면_help로_이동한다() {
        let mut engine = SearchEngine::new();
        let mut state = test_state();
        run_selected(
            &mut state,
            &mut engine,
            sysitem("안내", "", "kmd:unknown_cmd"),
        );

        assert_eq!(state.query, ":help");
        assert!(!state.results.is_empty(), "도움말 결과가 채워져야 한다");
    }

    #[test]
    fn bang_힌트를_실행하면_웹검색_쿼리로_전환된다() {
        let mut engine = SearchEngine::new();
        let mut state = test_state();
        run_selected(
            &mut state,
            &mut engine,
            sysitem("웹으로 검색", "@g rust", "kmd:bang_hint:g"),
        );

        assert_eq!(state.query, "@g rust", "항목의 path가 새 쿼리가 된다");
    }

    #[test]
    fn 설정_항목을_실행하면_모달이_열린다() {
        let mut engine = SearchEngine::new();
        let mut state = test_state();
        assert!(state.settings.is_none());

        run_selected(
            &mut state,
            &mut engine,
            sysitem("설정", "", "kmd:tui:open_settings"),
        );
        assert!(state.settings.is_some(), "설정 모달이 열려야 한다");
    }

    #[test]
    fn 도움말_항목을_실행하면_시작_쿼리로_전환된다() {
        let mut engine = SearchEngine::new();
        let mut state = test_state();

        // 공용 도움말 목록에서 시작 쿼리가 있는 항목 하나를 고른다
        let help = kmd_core::query_prefix::help_items(false);
        let item = help
            .into_iter()
            .find(|i| kmd_core::query_prefix::help_query_seed(&i.name).is_some())
            .expect("시작 쿼리가 있는 도움말 항목");
        let seed = kmd_core::query_prefix::help_query_seed(&item.name)
            .unwrap()
            .to_string();

        run_selected(&mut state, &mut engine, SearchResult { item, score: 0 });
        assert_eq!(state.query, seed);
    }

    // ── 드릴다운 진입/복귀 ──────────────────────────────────────────────

    #[test]
    fn 드릴다운_진입후_복귀하면_이전_상태가_그대로_돌아온다() {
        let mut state = test_state();

        // 이 저장소의 docs/ 를 대상으로 실제 디렉터리 나열을 태운다
        let dir = std::path::PathBuf::from("docs");
        assert!(dir.is_dir(), "테스트 전제: docs/ 존재");

        state.query = "원래쿼리".to_string();
        state.results = vec![SearchResult {
            item: kmd_core::IndexItem {
                name: "docs".to_string(),
                path: dir.to_string_lossy().to_string(),
                kind: ItemKind::Directory,
                source: kmd_core::index::Source::FileProvider,
                icon: String::new(),
                keywords: String::new(),
                icon_path: None,
            },
            score: 0,
        }];
        state.selected_index = 0;

        drill_into_folder(&mut state);
        assert_eq!(
            state.drill_path.as_deref(),
            Some(dir.as_path()),
            "드릴 경로 설정"
        );
        assert!(state.query.is_empty(), "드릴 진입 시 쿼리는 비운다");
        assert!(!state.results.is_empty(), "디렉터리 내용이 채워져야 한다");

        drill_back(&mut state);
        assert!(state.drill_path.is_none(), "드릴에서 빠져나왔다");
        assert_eq!(state.query, "원래쿼리", "이전 쿼리 복원");
        assert_eq!(state.results.len(), 1, "이전 결과 복원");
    }

    #[test]
    fn 디렉터리가_아니면_드릴다운하지_않는다() {
        let mut state = test_state();
        state.results = vec![sysitem("파일아님", "docs", "")];
        state.selected_index = 0;

        drill_into_folder(&mut state);
        assert!(
            state.drill_path.is_none(),
            "SystemCommand 항목은 드릴 대상이 아니다"
        );
        assert!(state.drill_stack.is_empty(), "스택도 쌓이면 안 된다");
    }

    // ── 프리픽스 디스패치 테이블 ────────────────────────────────────────
    //
    // update_search의 14-arm match가 각 프리픽스를 **자기 핸들러로** 보내는지
    // 고정한다. arm 하나를 잘못 연결하거나 프리픽스 판정이 바뀌면 사용자는
    // "엉뚱한 결과가 나온다"로만 겪고 컴파일러는 아무 말도 안 한다.

    fn results_for(query: &str) -> AppState {
        let mut engine = SearchEngine::new();
        let mut state = test_state();
        type_str(&mut state, &mut engine, query);
        state
    }

    #[test]
    fn calc_프리픽스는_계산_결과로_간다() {
        let st = results_for(":calc 2+3");
        assert!(
            st.results.iter().any(|r| r.item.name.contains('5')),
            "2+3 → 5 가 없다: {:?}",
            st.results.iter().map(|r| &r.item.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn emoji_프리픽스는_이모지_결과로_간다() {
        // 주의: 별칭 뒤 공백부터 내장 한글 조합이 자동으로 켜지므로(§emoji_keyword_started)
        // 여기서 "heart"를 타이핑하면 자모로 조합된다. 라우팅만 확인하려면
        // 공백 없는 상태를 본다.
        let st = results_for(":emoji");
        assert!(
            !st.results.is_empty(),
            "이모지 핸들러로 라우팅되어 목록이 나와야 한다"
        );
    }

    #[test]
    fn emoji_키워드는_한글_조합으로_입력된다() {
        // `:e ` 이후 자동 한글 조합이 켜지므로 로마자 타이핑은 자모가 된다.
        // 한국어 키워드로 이모지를 찾는 것이 이 모드의 의도다.
        let mut engine = SearchEngine::new();
        let mut state = test_state();
        type_str(&mut state, &mut engine, "/e ");
        assert!(state.hangul_mode, "공백 후 한글 조합 활성");

        type_str(&mut state, &mut engine, "gkdns"); // → 하늘
        assert!(
            state.effective_query().contains('하') || state.effective_query().contains('느'),
            "한글로 조합돼야 한다: {}",
            state.effective_query()
        );
    }

    #[test]
    fn shell_프리픽스는_contains_모드로_전환된다() {
        let st = results_for("!echo");
        assert!(!st.results.is_empty());
        assert_eq!(st.search_mode_label(), SearchMode::Contains.label());
    }

    #[test]
    fn version_프리픽스는_버전을_보여준다() {
        let st = results_for(":version");
        let names: Vec<_> = st.results.iter().map(|r| r.item.name.clone()).collect();
        assert!(
            names.iter().any(|n| n.contains(env!("CARGO_PKG_VERSION"))),
            "버전 문자열이 없다: {names:?}"
        );
    }

    #[test]
    fn help_keys_keymap_프리픽스가_각각_결과를_낸다() {
        for q in [":help", ":keys", ":keymap"] {
            assert!(
                !results_for(q).results.is_empty(),
                "'{q}' 가 빈 결과 — 디스패치가 끊겼을 수 있다"
            );
        }
    }

    #[test]
    fn 미지원_콜론_명령은_안내를_최상단에_붙인다() {
        let st = results_for(":이런명령은없다");
        assert!(
            !st.results.is_empty(),
            "일반 검색 폴스루 + 안내가 있어야 한다"
        );
    }

    #[test]
    fn 프리픽스만_입력한_중간_상태에서_패닉하지_않는다() {
        // 사용자는 ":calc"를 치기까지 ":", ":c", ":ca" ... 를 모두 거친다
        for q in [":", ":c", ":ca", ":cal", ":calc", ":e", "!", ">", "@"] {
            let _ = results_for(q);
        }
    }

    // ── 선택 상태 전이 ──────────────────────────────────────────────────

    #[test]
    fn 선택_이동은_결과_범위를_벗어나지_않는다() {
        let mut engine = SearchEngine::new();
        let mut state = test_state();
        type_str(&mut state, &mut engine, ":help");
        let n = state.results.len();
        assert!(n >= 2, "이동을 검증하려면 결과가 2개 이상이어야 한다");

        let down = |st: &mut AppState, e: &mut SearchEngine| {
            handle_key(
                st,
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                e,
                None,
            )
        };
        let up = |st: &mut AppState, e: &mut SearchEngine| {
            handle_key(st, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), e, None)
        };

        // 위로: 0에서 더 못 올라간다
        up(&mut state, &mut engine);
        assert_eq!(state.selected_index, 0, "맨 위에서 Up은 무동작");

        down(&mut state, &mut engine);
        assert_eq!(state.selected_index, 1);

        // 아래로: 마지막을 넘지 않는다
        for _ in 0..(n + 5) {
            down(&mut state, &mut engine);
        }
        assert_eq!(state.selected_index, n - 1, "맨 아래에서 Down은 무동작");
    }

    #[test]
    fn 쿼리가_바뀌면_선택이_처음으로_돌아간다() {
        let mut engine = SearchEngine::new();
        let mut state = test_state();
        type_str(&mut state, &mut engine, ":help");
        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &mut engine,
            None,
        );
        assert_eq!(state.selected_index, 1);

        // 한 글자 더 입력 → 결과가 다시 만들어지므로 선택은 0으로
        type_str(&mut state, &mut engine, "x");
        assert_eq!(state.selected_index, 0, "새 결과에서는 첫 항목 선택");
    }

    #[test]
    fn 입력하면_드릴다운에서_빠져나온다() {
        let mut engine = SearchEngine::new();
        let mut state = test_state();
        state.drill_path = Some(std::path::PathBuf::from("."));

        type_str(&mut state, &mut engine, "a");
        assert!(state.drill_path.is_none(), "타이핑은 드릴 모드를 종료한다");
    }

    #[test]
    fn slash_exit_입력중_한글_조합_안됨() {
        // 회귀: "/e"까지 입력된 순간 이모지 프리픽스로 오인해 한글 조합이
        // 켜지면서 x가 'ㅌ'로 바뀌던 버그 (/exit → /eㅌit)
        let mut engine = SearchEngine::new();
        let mut state = test_state();
        type_str(&mut state, &mut engine, "/exit");
        assert_eq!(state.query, "/exit");
        assert!(!state.hangul_mode);
        assert!(!state.hangul_auto);
    }

    #[test]
    fn emoji_별칭만으로는_한글_조합_비활성() {
        let mut engine = SearchEngine::new();
        let mut state = test_state();
        type_str(&mut state, &mut engine, "/e");
        assert!(!state.hangul_mode, "공백 전에는 한글 조합이 켜지면 안 됨");
    }

    #[test]
    fn emoji_키워드_공백_후_자동_한글_조합() {
        let mut engine = SearchEngine::new();
        let mut state = test_state();
        type_str(&mut state, &mut engine, "/e ");
        assert!(state.hangul_mode && state.hangul_auto);

        // "rk" → ㄱ+ㅏ 조합 중 '가'
        type_str(&mut state, &mut engine, "rk");
        assert!(state.effective_query().ends_with('가'));
    }

    #[test]
    fn emoji_키워드_이탈시_자동_한글_해제() {
        let mut engine = SearchEngine::new();
        let mut state = test_state();
        type_str(&mut state, &mut engine, "/e ");
        assert!(state.hangul_auto);

        // 백스페이스로 공백 제거 → ":e"로 복귀 → 자동 모드 해제
        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
            &mut engine,
            None,
        );
        assert!(!state.hangul_mode);
        assert!(!state.hangul_auto);
    }

    #[test]
    fn tui_도움말은_클립보드_히스토리_항목을_숨긴다() {
        // 회귀 방지: 공용 COMMANDS의 클립보드 항목을 TUI 도움말에 노출하면
        // 선택 시 시작 쿼리 `;`가 일반 파일 검색으로 흘러가 안내와 다르게 동작한다.
        let mut state = test_state();
        handle_help_query(&mut state);
        assert!(
            !state.results.is_empty(),
            "다른 도움말 항목은 그대로 보여야 함"
        );
        assert!(
            state.results.iter().all(|r| {
                !matches!(
                    kmd_core::query_prefix::help_query_seed(&r.item.name),
                    Some(seed)
                        if kmd_core::query_prefix::prefix_of(seed) == QueryPrefix::Clipboard
                )
            }),
            "클립보드 도움말 항목이 TUI에 노출됨"
        );
    }
}
