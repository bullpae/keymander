//! 설정 쿼리/액션 핸들러 — :set, :help, autostart, provider 토글

use super::*;
#[allow(unused_imports)]
use super::{items_to_results, save_config};

/// `:set` 목록의 한 행. 화면 표시(name/icon/desc)와 실행 키(action)를 함께 든다.
struct SettingsRow {
    name: String,
    action: String,
    icon: String,
    desc: String,
}

/// 프로바이더 토글 4군(LLM·멀티웹·맞춤법·번역)의 차이점만 모은 표.
///
/// 네 군은 "목록에서 켜고 끄되, 전부 끄면 기본값으로 되돌린다"는 동작이 같아
/// 목록 생성(`settings_rows`)과 토글 실행(`toggle_provider`)을 공유한다.
struct ProviderGroup {
    kind: ProviderKind,
    /// action 키의 가운데 조각 — `kmd:settings:{action_prefix}:toggle:{id}`
    action_prefix: &'static str,
    label_prefix: &'static str,
    emoji: &'static str,
    ascii: &'static str,
    desc: &'static str,
    /// (설정 id, 표시 이름). 순서가 곧 목록 순서다.
    members: &'static [(&'static str, &'static str)],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProviderKind {
    Llm,
    MultiWeb,
    Spell,
    Translate,
}

const PROVIDER_GROUPS: &[ProviderGroup] = &[
    ProviderGroup {
        kind: ProviderKind::Llm,
        action_prefix: "llm",
        label_prefix: "Multi LLM",
        emoji: "\u{1F9E0}",
        ascii: "[LLM]",
        desc: "Toggle provider for @llm compare",
        members: &[
            ("chatgpt", "ChatGPT"),
            ("gemini", "Gemini"),
            ("claude", "Claude"),
            ("grok", "Grok"),
            ("perplexity", "Perplexity"),
        ],
    },
    ProviderGroup {
        kind: ProviderKind::MultiWeb,
        action_prefix: "mweb",
        label_prefix: "Multi Web",
        emoji: "\u{1F50E}",
        ascii: "[WEB]",
        desc: "Toggle engine for @msearch multi search",
        members: &[
            ("google", "Google"),
            ("naver_search", "Naver"),
            ("daum", "Daum"),
        ],
    },
    ProviderGroup {
        kind: ProviderKind::Spell,
        action_prefix: "spell",
        label_prefix: "Spell",
        emoji: "\u{270D}\u{FE0F}",
        ascii: "[SPL]",
        desc: "Toggle provider for @sp spelling check",
        members: &[
            ("naver_spell", "Naver Spell"),
            ("pusan_spell", "Pusan Spell"),
        ],
    },
    ProviderGroup {
        kind: ProviderKind::Translate,
        action_prefix: "translate",
        label_prefix: "Translate",
        emoji: "\u{1F5E3}\u{FE0F}",
        ascii: "[TR]",
        desc: "Toggle provider for @tr translation",
        members: &[
            ("google_translate", "Google Translate"),
            ("papago", "Papago"),
            ("deepl", "DeepL"),
        ],
    },
];

impl App {
    /// 군별 현재 선택 목록(App 상태 쪽 정본)에 대한 읽기 접근.
    fn provider_selection(&self, kind: ProviderKind) -> &Vec<String> {
        match kind {
            ProviderKind::Llm => &self.selected_llm_providers,
            ProviderKind::MultiWeb => &self.selected_multi_web_providers,
            ProviderKind::Spell => &self.spell_providers,
            ProviderKind::Translate => &self.translate_providers,
        }
    }

    /// 프로바이더 하나를 켜고 끈다. 전부 꺼지면 해당 군의 기본값(모든 멤버)으로
    /// 되돌린다 — `@llm`/`@sp` 같은 프리픽스가 빈 목록으로 무력화되지 않게 한다.
    /// App 상태와 `runtime_config`를 함께 갱신하고 config에 저장한다.
    fn toggle_provider(&mut self, group: &ProviderGroup, target: &str) {
        if target.is_empty() {
            return;
        }
        let selected = match group.kind {
            ProviderKind::Llm => &mut self.selected_llm_providers,
            ProviderKind::MultiWeb => &mut self.selected_multi_web_providers,
            ProviderKind::Spell => &mut self.spell_providers,
            ProviderKind::Translate => &mut self.translate_providers,
        };

        if selected.iter().any(|v| v.eq_ignore_ascii_case(target)) {
            selected.retain(|v| !v.eq_ignore_ascii_case(target));
        } else {
            selected.push(target.to_string());
        }
        if selected.is_empty() {
            *selected = group
                .members
                .iter()
                .map(|(id, _)| (*id).to_string())
                .collect();
        }

        let selected = selected.clone();
        match group.kind {
            ProviderKind::Llm => {
                self.runtime_config.launcher.multi_llm_providers = selected.clone();
                save_config(move |cfg| cfg.launcher.multi_llm_providers = selected);
            }
            ProviderKind::MultiWeb => {
                self.runtime_config.launcher.multi_web_providers = selected.clone();
                save_config(move |cfg| cfg.launcher.multi_web_providers = selected);
            }
            ProviderKind::Spell => {
                self.runtime_config.launcher.spell_providers = selected.clone();
                save_config(move |cfg| cfg.launcher.spell_providers = selected);
            }
            ProviderKind::Translate => {
                self.runtime_config.launcher.translate_providers = selected.clone();
                save_config(move |cfg| cfg.launcher.translate_providers = selected);
            }
        }
    }

    fn set_clipboard(text: &str) {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            if let Err(e) = clipboard.set_text(text.to_string()) {
                tracing::warn!("클립보드 쓰기 실패: {e}");
            }
        }
    }

    /// LLM 실행(@gpt/@llm) 라우팅. LLM 쿼리가 아니면 None을 반환해 일반 경로에 맡긴다.
    ///
    /// - 오토파일럿 켜짐 + 데몬에 잡 전송 성공: 자동화 서비스는 데몬이 키 주입,
    ///   나머지(perplexity/grok)는 여기서 URL로 연다.
    /// - 아니면 폴백: 전 서비스 URL을 열고, 붙여넣기형(gemini)이 있으면 프롬프트를
    ///   클립보드에 담아 수동 붙여넣기를 돕는다 (현행 동작).
    pub(super) fn try_llm_launch(&self) -> Option<Task<Message>> {
        let (services, prompt) = web::parse_any_llm_query(
            &self.query,
            &self.selected_llm_providers,
            &self.multi_llm_prefixes,
        )?;
        if services.is_empty() {
            return None;
        }

        let final_prompt = kmd_core::prompt::apply_template(
            &self.runtime_config.launcher.prompt_templates,
            &prompt,
        );
        let plan = web::build_llm_launch_plan(&services, &final_prompt);

        let has_paste = plan
            .jobs
            .iter()
            .any(|j| matches!(j.method, kmd_core::ipc::LlmInject::PasteEnter));

        // 오토파일럿 시도 (opt-in + 데몬 실행 필요)
        if self.runtime_config.launcher.llm_autopilot && !plan.jobs.is_empty() {
            let req = kmd_core::ipc::Request::LlmAutopilot {
                jobs: plan.jobs.clone(),
            };
            match kmd_core::ipc::send_request_result(&req) {
                Ok(_) => {
                    // 자동화 불필요 서비스만 여기서 직접 연다
                    for url in &plan.plain_urls {
                        let _ = kmd_core::action::open_url(url);
                    }
                    tracing::info!("LLM 오토파일럿 위임: {}개 잡", plan.jobs.len());
                    return Some(iced::exit());
                }
                Err(e) => {
                    tracing::warn!("오토파일럿 IPC 실패 — URL 폴백: {e}");
                }
            }
        }

        // 폴백: 전 서비스 URL 열기 + (붙여넣기형 있으면) 클립보드
        if has_paste && !final_prompt.is_empty() {
            Self::set_clipboard(&final_prompt);
        }
        for url in web::llm_plan_all_urls(&plan) {
            let _ = kmd_core::action::open_url(&url);
        }
        Some(iced::exit())
    }

    /// `@@ <프롬프트>` 이어서 질문 — 데몬에 위임. 열 URL이 없으므로 데몬
    /// 미실행/세션 없음 시엔 안내 로그만 남기고 종료(폴백 불가).
    pub(super) fn send_llm_followup(&self, prompt: &str) -> Task<Message> {
        let final_prompt = kmd_core::prompt::apply_template(
            &self.runtime_config.launcher.prompt_templates,
            prompt,
        );
        let req = kmd_core::ipc::Request::LlmFollowup {
            prompt: final_prompt,
        };
        match kmd_core::ipc::send_request_result(&req) {
            Ok(kmd_core::ipc::Response::Ok { message }) => tracing::info!("{message}"),
            Ok(kmd_core::ipc::Response::Error { message }) => tracing::warn!("{message}"),
            Ok(_) => {}
            Err(e) => tracing::warn!("이어서 질문 실패(데몬 미실행?): {e}"),
        }
        iced::exit()
    }

    pub(super) fn handle_keymap_action(
        &mut self,
        result: &kmd_core::SearchResult,
    ) -> Task<Message> {
        let keywords = &result.item.keywords;
        if keywords.ends_with(":noop") || keywords.contains(":noop:") {
            return Task::none();
        }
        if let Some(msg) =
            kmd_core::keymap::execute_keymap_action(&mut self.runtime_config, keywords)
        {
            tracing::info!("keymap action: {msg}");
        }
        let current_query = kmd_core::query_prefix::normalize_slash_command(self.query.trim())
            .unwrap_or_else(|| self.query.clone());
        self.handle_keymap_query(&current_query);
        Task::none()
    }

    pub(super) fn handle_settings_query(&mut self, query: &str) {
        let filter = match query.find(' ') {
            Some(pos) => query[pos + 1..].trim().to_lowercase(),
            None => String::new(),
        };

        let items: Vec<IndexItem> = self
            .settings_rows()
            .into_iter()
            .filter(|row| filter.is_empty() || row.name.to_lowercase().contains(&filter))
            .map(|row| IndexItem {
                name: row.name,
                path: row.desc,
                icon: row.icon,
                kind: ItemKind::SystemCommand,
                source: Source::Plugin,
                keywords: row.action,
                icon_path: None,
            })
            .collect();

        self.apply_contains_items(items);
    }

    /// `:set` 목록의 행 전체를 현재 상태 기준으로 만든다 (필터 이전).
    ///
    /// 순서가 곧 화면 순서다 — 고정 항목 → 테마 → 프로바이더 4군 → 실행 항목 →
    /// 정보 행(noop, 실행 불가라 맨 끝).
    fn settings_rows(&self) -> Vec<SettingsRow> {
        let emoji = self.use_emoji;
        let current_theme = self.theme.name;

        let ime_label = if self.reset_ime_on_launch {
            "IME: Reset to English on Launch [ON]"
        } else {
            "IME: Reset to English on Launch [OFF]"
        };
        let daemon_autostart_label = match self.daemon_autostart_enabled {
            Some(true) => "Daemon Auto Start [ON]",
            Some(false) => "Daemon Auto Start [OFF]",
            None => "Daemon Auto Start [UNKNOWN]",
        };
        let brand_icons_label = if self
            .runtime_config
            .general
            .brand_icons
            .eq_ignore_ascii_case("mono")
        {
            "Brand Icons: Mono (theme tint) [ON]"
        } else {
            "Brand Icons: Mono (theme tint) [OFF]"
        };

        let label = |base: &str, theme_name: &str| -> String {
            if current_theme.eq_ignore_ascii_case(theme_name) {
                format!("{base} [Current]")
            } else {
                base.to_string()
            }
        };

        let mut settings_entries: Vec<(String, String, String, String)> = vec![
            (
                "Edit Config File".to_string(),
                "kmd:settings:config".to_string(),
                if emoji { "\u{2699}\u{FE0F}" } else { "[CFG]" }.to_string(),
                "Open config.toml".to_string(),
            ),
            (
                "Open Config Directory".to_string(),
                "kmd:settings:dir".to_string(),
                if emoji { "\u{1F4C2}" } else { "[DIR]" }.to_string(),
                "Open configuration folder".to_string(),
            ),
            (
                format!(
                    "Version: desktop {} / core {}",
                    env!("CARGO_PKG_VERSION"),
                    kmd_core::Index::current_version()
                ),
                "kmd:settings:noop".to_string(),
                if emoji { "\u{1F4E6}" } else { "[VER]" }.to_string(),
                "Use :version or kmd-desktop --version".to_string(),
            ),
            (
                "Reset Window Position".to_string(),
                "kmd:settings:reset_position".to_string(),
                if emoji { "\u{1F4CD}" } else { "[POS]" }.to_string(),
                "Move window to default position".to_string(),
            ),
            (
                ime_label.to_string(),
                "kmd:settings:toggle_ime_reset".to_string(),
                if emoji { "\u{1F310}" } else { "[IME]" }.to_string(),
                "Toggle English input on launch".to_string(),
            ),
            (
                daemon_autostart_label.to_string(),
                "kmd:settings:toggle_autostart".to_string(),
                if emoji { "\u{23FB}\u{FE0F}" } else { "[BOOT]" }.to_string(),
                "Toggle daemon start at login".to_string(),
            ),
            (
                brand_icons_label.to_string(),
                "kmd:settings:toggle_brand_icons".to_string(),
                if emoji { "\u{1F5BC}" } else { "[ICO]" }.to_string(),
                "Mono glyphs vs full-color logos".to_string(),
            ),
            (
                label("Theme: Keymander (default)", "Keymander"),
                "kmd:settings:theme:keymander".to_string(),
                if emoji { "\u{1F319}" } else { "[THM]" }.to_string(),
                "Switch desktop theme".to_string(),
            ),
            (
                label("Theme: Obsidian", "Obsidian"),
                "kmd:settings:theme:obsidian".to_string(),
                if emoji { "\u{2B1B}" } else { "[THM]" }.to_string(),
                "Switch desktop theme".to_string(),
            ),
            (
                label("Theme: Snow", "Snow"),
                "kmd:settings:theme:snow".to_string(),
                if emoji { "\u{2600}\u{FE0F}" } else { "[THM]" }.to_string(),
                "Switch desktop theme".to_string(),
            ),
            (
                label("Theme: Rose Pine", "Rose Pine"),
                "kmd:settings:theme:rose_pine".to_string(),
                if emoji { "\u{1F339}" } else { "[THM]" }.to_string(),
                "Switch desktop theme".to_string(),
            ),
            (
                label("Theme: Nord", "Nord"),
                "kmd:settings:theme:nord".to_string(),
                if emoji { "\u{2744}\u{FE0F}" } else { "[THM]" }.to_string(),
                "Switch desktop theme".to_string(),
            ),
        ];

        // 프로바이더 4군은 라벨 접두·action 접두·아이콘·설명만 다르고 구조가 같다.
        for group in PROVIDER_GROUPS {
            let selected = self.provider_selection(group.kind);
            for (id, provider_name) in group.members {
                let enabled = selected.iter().any(|v| v.eq_ignore_ascii_case(id));
                settings_entries.push((
                    format!(
                        "{}: {} [{}]",
                        group.label_prefix,
                        provider_name,
                        if enabled { "ON" } else { "OFF" }
                    ),
                    format!("kmd:settings:{}:toggle:{id}", group.action_prefix),
                    if emoji { group.emoji } else { group.ascii }.to_string(),
                    group.desc.to_string(),
                ));
            }
        }

        settings_entries.extend_from_slice(&[
            (
                "Rebuild Index".to_string(),
                "kmd:settings:rebuild".to_string(),
                if emoji { "\u{1F504}" } else { "[IDX]" }.to_string(),
                "Rebuild and reload index data".to_string(),
            ),
            // Non-actionable info entries are intentionally at the bottom.
            (
                "Info: Move Window (drag top strip)".to_string(),
                "kmd:settings:noop".to_string(),
                if emoji { "\u{2139}\u{FE0F}" } else { "[TIP]" }.to_string(),
                "Info only - not executable".to_string(),
            ),
            (
                "Info: Resize Window (drag left/right edges)".to_string(),
                "kmd:settings:noop".to_string(),
                if emoji { "\u{2139}\u{FE0F}" } else { "[TIP]" }.to_string(),
                "Info only - not executable".to_string(),
            ),
        ]);

        settings_entries
            .into_iter()
            .map(|(name, action, icon, desc)| SettingsRow {
                name,
                action,
                icon,
                desc,
            })
            .collect()
    }

    pub(super) fn handle_help_query(&mut self) {
        let items = kmd_core::query_prefix::help_items(self.use_emoji);
        self.apply_contains_items(items);
    }

    pub(super) fn handle_settings_action(
        &mut self,
        result: &kmd_core::SearchResult,
    ) -> Task<Message> {
        let action_src = if result.item.keywords.starts_with("kmd:settings:") {
            result.item.keywords.as_str()
        } else {
            result.item.path.as_str()
        };
        let action = action_src.strip_prefix("kmd:settings:").unwrap_or("");

        match action {
            "noop" => {
                return Task::none();
            }
            "config" => {
                let config_dir = kmd_core::Config::default_config_dir();
                let config_path = config_dir.join(kmd_core::CONFIG_FILENAME);
                if !config_path.exists() {
                    let mut cfg = crate::engine::load_config();
                    cfg.config_path = Some(config_path.clone());
                    if let Err(e) = cfg.save() {
                        tracing::warn!("Failed to create config file: {e}");
                    }
                }
                if let Err(e) = open::that(&config_path) {
                    tracing::warn!("설정 파일 열기 실패: {e}");
                }
            }
            "dir" => {
                let config_dir = kmd_core::Config::default_config_dir();
                if let Err(e) = open::that(&config_dir) {
                    tracing::warn!("설정 디렉토리 열기 실패: {e}");
                }
            }
            "reset_position" => {
                WindowState::reset();
                self.window_state = WindowState::default();
                self.window_width = DEFAULT_WIDTH;
                self.state_dirty = false;
                // 높이 고정 플랫폼에서는 높이를 건드리지 않는다 — 여기서 접으면
                // 리사이즈 잔상이 그대로 드러난다 (app.rs FIXED_WINDOW_HEIGHT).
                // (쿼리/결과는 이 아래에서 비우므로 목표 높이는 명시적으로 정한다)
                let reset_height = if self.fixed_window_height {
                    self.ui.full_window_height
                } else {
                    self.ui.collapsed_window_height
                };
                self.window_height = reset_height;
                let run_reset = move |id: window::Id| {
                    let resize = window::resize(id, Size::new(DEFAULT_WIDTH, reset_height));
                    let move_task = window::monitor_size(id).then(move |maybe_size| {
                        if let Some(mon) = maybe_size {
                            let x = (mon.width - DEFAULT_WIDTH) / 2.0;
                            let y = (mon.height / 3.0).max(0.0);
                            window::move_to(id, Point::new(x, y))
                        } else {
                            Task::none()
                        }
                    });
                    Task::batch([resize, move_task])
                };

                self.query.clear();
                self.clear_results_state(kmd_core::SearchMode::Fuzzy);

                return match self.window_id {
                    Some(id) => run_reset(id),
                    None => window::oldest().then(move |maybe_id| match maybe_id {
                        Some(id) => run_reset(id),
                        None => Task::none(),
                    }),
                };
            }
            "rebuild" => {
                self.full_warmup_started = true;
                self.loading = true;
                let task = self.spawn_full_engine_load_task();
                self.query.clear();
                self.clear_results_state(kmd_core::SearchMode::Fuzzy);
                return task;
            }
            "toggle_brand_icons" => {
                let mono = self
                    .runtime_config
                    .general
                    .brand_icons
                    .eq_ignore_ascii_case("mono");
                let new_val = if mono { "color" } else { "mono" };
                self.runtime_config.general.brand_icons = new_val.to_string();
                tracing::info!("brand_icons = {new_val}");
                save_config(|cfg| cfg.general.brand_icons = new_val.to_string());

                self.query = ":set".to_string();
                self.handle_settings_query(":set");
                return self.request_focus();
            }
            "toggle_ime_reset" => {
                self.reset_ime_on_launch = !self.reset_ime_on_launch;
                let new_val = self.reset_ime_on_launch;
                self.runtime_config.general.reset_ime_on_launch = new_val;
                tracing::info!("reset_ime_on_launch = {new_val}");
                save_config(|cfg| cfg.general.reset_ime_on_launch = new_val);

                self.query = ":set".to_string();
                self.handle_settings_query(":set");
                return self.request_focus();
            }
            "toggle_autostart" => {
                if self.daemon_autostart_toggle_in_flight {
                    return Task::none();
                }
                self.daemon_autostart_toggle_in_flight = true;
                let request = if self.daemon_autostart_enabled.unwrap_or(false) {
                    kmd_core::ipc::Request::AutostartDisable
                } else {
                    kmd_core::ipc::Request::AutostartEnable
                };
                self.query = ":set".to_string();
                self.handle_settings_query(":set");
                return Task::future(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        kmd_core::ipc::send_request_result(&request)
                    })
                    .await;
                    let mapped = match result {
                        Ok(Ok(kmd_core::ipc::Response::Ok { message })) => Ok(message),
                        Ok(Ok(kmd_core::ipc::Response::Error { message })) => Err(message),
                        Ok(Ok(other)) => Err(format!("예기치 않은 응답: {other:?}")),
                        Ok(Err(e)) => Err(format!("IPC 실패: {e}")),
                        Err(e) => Err(format!("작업 실패: {e}")),
                    };
                    Message::AutostartToggleFinished(mapped)
                });
            }
            // 프로바이더 4군은 접두만 다르고 동작이 같다 — 표에서 찾아 공통 처리.
            provider_action
                if PROVIDER_GROUPS.iter().any(|g| {
                    provider_action.starts_with(&format!("{}:toggle:", g.action_prefix))
                }) =>
            {
                let group = PROVIDER_GROUPS
                    .iter()
                    .find(|g| provider_action.starts_with(&format!("{}:toggle:", g.action_prefix)))
                    .expect("가드에서 확인한 군");
                let target = provider_action
                    .strip_prefix(&format!("{}:toggle:", group.action_prefix))
                    .unwrap_or("");
                self.toggle_provider(group, target);

                self.query = ":set".to_string();
                self.handle_settings_query(":set");
                return self.request_focus();
            }
            theme_action if theme_action.starts_with("theme:") => {
                let theme_name = theme_action.strip_prefix("theme:").unwrap_or("midnight");
                self.theme = crate::theme::from_name(theme_name);
                self.runtime_config.general.theme = theme_name.to_string();
                tracing::info!("Theme changed to: {}", self.theme.name);

                // Persist theme selection to config file.
                let name_owned = theme_name.to_string();
                save_config(|cfg| cfg.general.theme = name_owned);
            }
            _ => {
                tracing::warn!("Unknown settings action: {action}");
            }
        }
        self.query.clear();
        self.clear_results_state(kmd_core::SearchMode::Fuzzy);
        self.request_focus()
    }

    pub(super) fn schedule_autostart_status_refresh(&mut self, force: bool) -> Task<Message> {
        if self.daemon_autostart_check_in_flight {
            return Task::none();
        }
        if !force
            && self
                .daemon_autostart_last_checked_at
                .is_some_and(|ts| ts.elapsed() < Duration::from_millis(AUTOSTART_STATUS_REFRESH_MS))
        {
            return Task::none();
        }
        self.daemon_autostart_check_in_flight = true;
        Task::future(async move {
            let result = tokio::task::spawn_blocking(|| {
                kmd_core::ipc::send_request_result(&kmd_core::ipc::Request::AutostartStatus)
            })
            .await;
            let mapped = match result {
                Ok(Ok(kmd_core::ipc::Response::AutostartStatus { installed })) => Ok(installed),
                Ok(Ok(other)) => Err(format!("예기치 않은 응답: {other:?}")),
                Ok(Err(e)) => Err(format!("IPC 실패: {e}")),
                Err(e) => Err(format!("작업 실패: {e}")),
            };
            Message::AutostartStatusLoaded(mapped)
        })
    }
}
