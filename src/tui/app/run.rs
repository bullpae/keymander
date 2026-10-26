//! 메인 루프 — 터미널 설정/복구와 이벤트 루프

use super::*;

// ── Main Loop ────────────────────────────────────────────────────────────────

/// Run the TUI application.
///
/// `instance_guard` is the single-instance RAII guard obtained in `main()`.
/// The event loop checks it on every tick for external quit signals.
///
/// `_show_on_ready` — Windows 콘솔 표시 시점 제어용으로 받지만 현재는 쓰지 않는다
/// (창 중앙 배치는 Windows Terminal의 `centerOnLaunch`가 맡는다).
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

    let total_items = index.items.len();
    let mut engine = SearchEngine::with_items(&config.launcher.kind_weights, index.items);

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
        scope_undo: None,
        settings: None,
        is_portable: kmd_core::portable::is_portable(),
        use_emoji: config.general.emoji_icons,
        llm_autopilot: false,
        config: config.clone(),
        frecency: db
            .as_ref()
            .map(kmd_core::history::load_boost_map)
            .unwrap_or_default(),
        pending_shell: None,
        cached_effective_query: String::new(),
        dirty: true,
    };
    state.sync_config_mirrors();

    // Setup terminal — 복구는 가드가 책임진다 (아래 TerminalSession 참조).
    // 가드는 raw mode를 켠 **직후** 만든다. execute!가 실패하면 raw mode만
    // 켜진 채 남는데, 가드가 그 뒤에 있으면 그 경우가 복구되지 않는다.
    enable_raw_mode()?;
    let _terminal_guard = TerminalSession;
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
                poll_pending_shell(&mut state);
                // 퍼지 검색이 시간 안에 안 끝났으면 같은 질의로 다시 물어 결과를 완성한다
                if engine.is_pending() && state.settings.is_none() {
                    update_search(&mut state, &mut engine, db.as_ref());
                    state.mark_dirty();
                }
                // Re-render on tick only when status_message is set (for auto-clear)
                if state.status_message.is_some() {
                    state.mark_dirty();
                }
            }
        }
    }

    // 터미널 복구는 _terminal_guard의 Drop이 처리한다 — 여기서 중복으로 하지
    // 않는다. 정상 종료든 `?`로 빠져나가든 패닉이든 같은 경로로 복구된다.
    Ok(())
}

/// 터미널을 원래 상태로 되돌리는 RAII 가드.
///
/// raw mode·대체 화면·bracketed paste를 켠 구간에서 `?`가 하나라도 조기 반환하면
/// 복구 코드가 실행되지 않아 **사용자 셸이 망가진 채 남는다**(에코 없음, 프롬프트
/// 안 보임). 실제로 이 함수에는 `terminal.draw()?`·`events.next()?` 등 조기 반환
/// 지점이 여럿 있었다. Drop에 맡기면 오류·패닉 어느 쪽으로 나가도 복구된다.
///
/// Drop에서는 실패해도 할 수 있는 게 없으므로 결과를 무시한다 — 나머지 복구
/// 단계는 계속 시도하는 편이 낫다.
struct TerminalSession;

impl Drop for TerminalSession {
    fn drop(&mut self) {
        // 각 복구 단계를 따로 실행한다 — execute!로 묶으면 앞 명령이 실패할 때
        // 뒤 명령이 통째로 생략된다. 되돌릴 수 있는 것은 최대한 되돌린다.
        let mut out = std::io::stdout();
        let _ = crossterm::execute!(out, DisableBracketedPaste);
        let _ = crossterm::execute!(out, LeaveAlternateScreen);
        let _ = crossterm::execute!(out, crossterm::cursor::Show);
        let _ = disable_raw_mode();
    }
}
