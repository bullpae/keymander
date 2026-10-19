//! 실행 — 선택 항목 실행, 변환/프롬프트/웹 질의, quick action·LLM 헬퍼

use super::*;

// ── Execute ──────────────────────────────────────────────────────────────────

/// quick action을 워커 스레드에서 시작한다. 결과는 `poll_pending_shell`이 받는다.
pub(super) fn start_quick_action(state: &mut AppState, item: kmd_core::IndexItem) {
    // 겹쳐 실행하지 않는다 — 두 결과가 클립보드를 번갈아 덮어쓰면 사용자는
    // 무엇이 복사됐는지 알 수 없다.
    if state.pending_shell.is_some() {
        state.status_message = Some("\u{23F3} 이전 실행이 아직 끝나지 않았습니다".into()); // ⏳
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    state.status_message = Some(format!("\u{23F3} 실행 중: {}", item.path)); // ⏳
    std::thread::spawn(move || {
        let action = builtin_shell::ShellExtension.execute(&item);
        // 받는 쪽이 사라졌으면(TUI 종료) 결과를 버린다.
        let _ = tx.send(action);
    });
    state.pending_shell = Some(rx);
}

/// 실행 중인 quick action이 끝났으면 결과를 반영한다 — Tick마다 호출.
pub(super) fn poll_pending_shell(state: &mut AppState) {
    let Some(rx) = &state.pending_shell else {
        return;
    };
    let action = match rx.try_recv() {
        Ok(action) => action,
        Err(std::sync::mpsc::TryRecvError::Empty) => return,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            // 워커가 결과 없이 끝났다(패닉) — 조용히 "실행 중"에 머물지 않는다.
            kmd_core::plugin::ExtensionAction::Display("실행이 비정상 종료됐습니다".into())
        }
    };
    state.pending_shell = None;
    apply_quick_action_result(state, action);
    state.mark_dirty();
}

fn apply_quick_action_result(state: &mut AppState, action: kmd_core::plugin::ExtensionAction) {
    match action {
        kmd_core::plugin::ExtensionAction::CopyToClipboard(output) => {
            let first_line = output.lines().next().unwrap_or("(no output)").to_string();
            state.status_message = Some(match kmd_core::clipboard::set_text(&output) {
                Ok(()) => format!("\u{2705} {first_line}"),
                Err(e) => format!("\u{274C} 실행은 됐으나 복사 실패: {e}"),
            });
        }
        kmd_core::plugin::ExtensionAction::Display(msg) => {
            state.status_message = Some(format!("\u{274C} {}", msg)); // ❌
        }
        _ => {}
    }
}

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
        &state.config.launcher.multi_llm_providers,
        &state.config.launcher.multi_llm_prefixes,
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
        // 실패를 숨기지 않는다 — 복사됐다고 믿고 붙여넣으면 빈 프롬프트가 나간다.
        state.status_message = Some(match kmd_core::clipboard::set_text(&final_prompt) {
            Ok(()) => "✅ 프롬프트를 클립보드에 복사했습니다 (일부 서비스는 붙여넣기/Enter 필요)"
                .to_string(),
            Err(e) => format!("❌ 프롬프트 복사 실패: {e} — 서비스 창에 직접 입력하세요"),
        });
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
pub(super) fn execute_selected(
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

    // 무엇을 할지는 core가 분류한다(마커 해석의 정본 — 데스크톱과 공유).
    use kmd_core::activation::Activation;
    match kmd_core::activation::classify(&result.item) {
        // 도움말 시드, !bang 힌트, 미지 명령 → :help, 폴더 검색 시드, @ 브라우징 힌트
        Activation::SetQuery(seed) => {
            state.query = seed;
            state.refresh_effective_query();
            state.selected_index = 0;
            update_search(state, engine, db);
        }
        // 폴더 제안 → `:scope` 추가와 같은 경로 (저장 + 바로 인덱싱, docs/18 E5)
        Activation::FolderSuggest => {
            if let Some(path) = kmd_core::folder_suggest::suggest_path(&result.item.keywords) {
                super::scope::scope_add(state, engine, &path);
            }
            state.selected_index = 0;
            let msg = state.status_message.take();
            update_search(state, engine, db);
            state.status_message = msg;
        }
        Activation::ScopeAdd(path) => {
            super::scope::scope_add(state, engine, Path::new(&path));
            let msg = state.status_message.take();
            update_search(state, engine, db);
            state.status_message = msg;
        }
        Activation::ScopeFolder(path) => match action::open_with_system(&path) {
            action::ActionResult::Error(e) => {
                state.status_message = Some(format!("\u{274C} 폴더 열기 실패: {e}"));
            }
            _ => {
                if state.quit_on_launch {
                    state.should_quit = true;
                }
            }
        },
        // 설정 모달 열기 (:set)
        Activation::OpenSettingsModal => {
            state.settings = Some(SettingsState::new(state.config.clone()));
        }
        // 키맵 액션 (start/stop/프로파일 전환)
        Activation::Keymap => {
            if let Some(msg) =
                kmd_core::keymap::execute_keymap_action(&mut state.config, &result.item.keywords)
            {
                state.status_message = Some(msg);
            }
            let query = kmd_core::query_prefix::normalize_slash_command(&state.query)
                .unwrap_or_else(|| state.query.clone());
            handle_keymap_query(&query, state);
        }
        Activation::PromptCommit => commit_prompt_command(state, engine, db),
        // :t 실행 — 검색이 아니라 여기서 연다 (타이핑 중 브라우저가 열리지 않게)
        Activation::TransformRun => run_transform(state),
        // 클립보드 히스토리·데스크톱 설정은 TUI에 없다 — 안내 항목과 같이 무시
        Activation::ClipPaste | Activation::Settings | Activation::Noop => {}
        // 계산기·이모지 결과 → 클립보드 복사.
        // 복사에 실패하면 실패를 알린다 — 예전에는 결과를 버리고 무조건 "Copied"라
        // 표시해서, 클립보드가 막힌 환경에서 복사된 줄 알고 붙여넣다 낭패를 봤다.
        Activation::Copy(text) if !text.is_empty() => {
            state.status_message = Some(match kmd_core::clipboard::set_text(&text) {
                Ok(()) => format!("\u{2705} Copied: {text}"),
                Err(e) => format!("\u{274C} 복사 실패: {e}"),
            });
        }
        Activation::Copy(_) => execute_default(state, &result, db),
        // Shell command — quick action은 백그라운드 실행 후 결과를 클립보드에 복사,
        // 사용자 명령은 새 터미널 창에서 실행한다 (데스크톱과 동일한 UX).
        // 인라인 실행의 10초 타임아웃은 즉답형 quick action에만 적합하다 —
        // `>winget upgrade --all` 같은 장시간 명령을 중간에 죽이지 않는다.
        Activation::ShellQuickAction => start_quick_action(state, result.item.clone()),
        Activation::ShellTerminal => match builtin_shell::launch_in_terminal(&result.item.path) {
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
        },
        Activation::Web => {
            // LLM 실행(@gpt/@llm) — 오토파일럿 또는 URL 폴백으로 라우팅
            if tui_try_llm_launch(state) {
                if state.quit_on_launch {
                    state.should_quit = true;
                }
                return;
            }
            // 웹 검색 결과 — extract_batch_urls 통합 (LLM 외 msearch/spell/translate)
            if let Some(urls) = web::extract_batch_urls(&result.item) {
                open_urls_and_quit(state, &urls);
                return;
            }
            if !open_raw_web_query(state) {
                execute_default(state, &result, db);
            }
        }
        Activation::Execute => {
            if !open_raw_web_query(state) {
                execute_default(state, &result, db);
            }
        }
    }
}

/// `:t` 실행 — 질의로 변환 URL을 만들어 연다. 텍스트가 비면 클립보드를 쓴다.
fn run_transform(state: &mut AppState) {
    use kmd_core::transform;
    let trimmed = state.query.trim();
    let normalized = kmd_core::query_prefix::normalize_slash_command(trimmed);
    let dispatch = normalized.as_deref().unwrap_or(trimmed);
    let Some(tq) = transform::parse_transform_query_or_clipboard(dispatch) else {
        return;
    };
    let urls = transform::build_transform_urls(
        &tq,
        &state.config.launcher.spell_providers,
        &state.config.launcher.translate_providers,
    );
    let kind_label = match &tq.kind {
        transform::TransformKind::Spell => "맞춤법 검사",
        transform::TransformKind::Translate(_) => "번역",
    };
    state.status_message = Some(format!(
        "\u{2705} {} 실행 ({} 서비스)",
        kind_label,
        urls.len()
    ));
    open_urls_and_quit(state, &urls);
}

/// `:prompt add/rm` Enter 확정 — 현재 질의를 다시 해석해 적용·저장한다.
/// 성공하면 `:prompt` 목록으로 돌아가 결과를 바로 보여 준다.
fn commit_prompt_command(
    state: &mut AppState,
    engine: &mut SearchEngine,
    db: Option<&kmd_core::Database>,
) {
    let cmd = kmd_core::prompt::parse_command(state.query.trim());
    // 디스크 최신 설정 위에 적용한다 (save_config_change 규칙) — 검증 실패면 저장하지 않는다.
    let mut preview = state.config.launcher.prompt_templates.clone();
    let msg = match kmd_core::prompt::apply_command(&cmd, &mut preview) {
        Err(e) => {
            state.status_message = Some(e);
            return;
        }
        Ok(msg) => msg,
    };
    let saved = save_config_change(state, move |latest| {
        let _ = kmd_core::prompt::apply_command(&cmd, &mut latest.launcher.prompt_templates);
    });
    match saved {
        Ok(()) => {
            state.query = ":prompt".to_string();
            state.refresh_effective_query();
            state.selected_index = 0;
            update_search(state, engine, db);
            state.status_message = Some(msg);
        }
        Err(e) => state.status_message = Some(format!("\u{274C} 저장 실패: {e}")),
    }
}

/// 원시 입력 웹 쿼리 폴백 — classify_web_query 통합 분류기로 URL을 연다.
/// 열었으면 true.
fn open_raw_web_query(state: &mut AppState) -> bool {
    let launcher = &state.config.launcher;
    let cfg = web::WebQueryConfig::from_launcher(launcher);
    let urls: Vec<String> = match web::classify_web_query(&state.query, &cfg) {
        web::WebQueryResult::MultiLlm(services, q) | web::WebQueryResult::MultiWeb(services, q)
            if !q.is_empty() =>
        {
            services
                .iter()
                .map(|s| web::build_search_url(s, &q))
                .collect()
        }
        web::WebQueryResult::Spell(q) if !q.is_empty() => {
            web::spell_result_items(&q, &launcher.spell_providers, state.use_emoji)
                .first()
                .and_then(web::extract_batch_urls)
                .unwrap_or_default()
        }
        web::WebQueryResult::Translate(dir, q) if !q.is_empty() => {
            web::translate_result_items(&q, dir, &launcher.translate_providers, state.use_emoji)
                .first()
                .and_then(web::extract_batch_urls)
                .unwrap_or_default()
        }
        web::WebQueryResult::Single(service, q) if !q.is_empty() => {
            vec![web::build_search_url(service, &q)]
        }
        _ => return false,
    };
    // Spell/Translate는 URL이 없어도 처리한 것으로 본다 (예전 동작)
    if !urls.is_empty() {
        open_urls_and_quit(state, &urls);
    }
    true
}

/// 기본 실행 — `action::execute` 후 성공하면 실행 이력을 남긴다.
fn execute_default(state: &mut AppState, result: &SearchResult, db: Option<&kmd_core::Database>) {
    match action::execute(result) {
        action::ActionResult::Launched => {
            if let Some(db) = db {
                kmd_core::history::record_launch(
                    db,
                    result.item.kind,
                    &result.item.path,
                    Some(&result.item.name),
                );
                // 창이 계속 떠 있으면 방금 연 항목이 다음 검색에 반영돼야 한다.
                // 실행은 드물어 여기서 다시 읽는 비용은 무시할 만하다.
                state.frecency = kmd_core::history::load_boost_map(db);
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
