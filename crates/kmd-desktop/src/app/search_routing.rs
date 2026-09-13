use super::*;

use crate::query_prefix::{prefix_of, Prefix};

impl App {
    pub(super) fn perform_search(&mut self) -> Task<Message> {
        // 모든 검색이 세대를 올린다 — 비동기 경로(클립보드)의 늦은 응답은
        // 세대가 달라져 update에서 폐기된다.
        self.search_generation = self.search_generation.wrapping_add(1);
        let generation = self.search_generation;
        let query = self.query.clone();
        let trimmed = query.trim();

        if trimmed.is_empty() {
            self.clear_results_state(kmd_core::SearchMode::Fuzzy);
            return Task::none();
        }

        // /help → :help 정규화 — 표시되는 쿼리는 그대로, 디스패치만 : 형태로
        let normalized = kmd_core::query_prefix::normalize_slash_command(trimmed);
        let trimmed = normalized.as_deref().unwrap_or(trimmed);

        let prev_signature = self.last_results_signature;
        let mut post_task: Task<Message> = Task::none();

        match prefix_of(trimmed) {
            Prefix::Web => self.handle_web_query(trimmed),
            Prefix::Transform => self.handle_transform_query(trimmed),
            Prefix::Prompt => self.handle_prompt_query(trimmed),
            Prefix::Calc => self.handle_calc_query(trimmed),
            Prefix::Emoji => self.handle_emoji_query(trimmed),
            Prefix::Settings => {
                self.handle_settings_query(trimmed);
                post_task = self.schedule_autostart_status_refresh(false);
            }
            Prefix::Help => self.handle_help_query(),
            Prefix::Version => self.handle_version_query(),
            Prefix::Shell => self.handle_shell_query(trimmed),
            Prefix::Keymap => self.handle_keymap_query(trimmed),
            Prefix::Keys => self.handle_keys_query(),
            Prefix::FolderSearch => self.handle_folder_search(trimmed),
            Prefix::ContentSearch => self.handle_content_search(trimmed),
            Prefix::Clipboard => {
                post_task = self.handle_clip_query(trimmed, generation);
            }
            Prefix::General => self.handle_main_search(trimmed),
        }

        // 결과가 실제로 바뀐 경우에만 아이콘 prefetch를 실행한다.
        let icon_task = if self.last_results_signature != prev_signature {
            self.spawn_icon_prefetch()
        } else {
            Task::none()
        };
        Task::batch([icon_task, post_task])
    }

    /// classify_web_query 통합 분류기 사용
    pub(super) fn handle_web_query(&mut self, query: &str) {
        let emoji = self.use_emoji;
        let cfg = web::WebQueryConfig {
            spell_prefixes: &self.spell_prefixes,
            translate_prefixes: &self.translate_prefixes,
            multi_llm_prefixes: &self.multi_llm_prefixes,
            multi_llm_ids: &self.selected_llm_providers,
            multi_web_prefixes: &self.multi_web_prefixes,
            multi_web_ids: &self.selected_multi_web_providers,
        };

        let results = match web::classify_web_query(query, &cfg) {
            web::WebQueryResult::Spell(q) => {
                items_to_results(web::spell_result_items(&q, &self.spell_providers, emoji))
            }
            web::WebQueryResult::Translate(dir, q) => items_to_results(
                web::translate_result_items(&q, dir, &self.translate_providers, emoji),
            ),
            web::WebQueryResult::MultiLlm(_svcs, q) => items_to_results(
                web::multi_llm_result_items(&q, &self.selected_llm_providers, emoji),
            ),
            web::WebQueryResult::MultiWeb(_svcs, q) => items_to_results(
                web::multi_web_result_items(&q, &self.selected_multi_web_providers, emoji),
            ),
            web::WebQueryResult::Single(service, q) => {
                if q.is_empty() {
                    let mut items = web::list_services_as_items("", emoji);
                    ensure_multi_llm_hint(&mut items, emoji);
                    ensure_multi_web_hint(&mut items, emoji);
                    items_to_results(items)
                } else {
                    let item = web::search_result_item(service, &q, emoji);
                    items_to_results(std::iter::once(item))
                }
            }
            web::WebQueryResult::Browse(filter) => {
                let mut items = web::list_services_as_items(&filter, emoji);
                ensure_multi_llm_hint(&mut items, emoji);
                ensure_multi_web_hint(&mut items, emoji);
                items_to_results(items)
            }
        };
        self.commit_results(results, kmd_core::SearchMode::Contains, true);
    }

    pub(super) fn handle_version_query(&mut self) {
        let items = kmd_core::query_prefix::version_items(
            "kmd-desktop",
            env!("CARGO_PKG_VERSION"),
            self.use_emoji,
        );
        self.apply_contains_items(items);
    }

    /// :t / :transform 쿼리 처리 (클립보드 변환)
    pub(super) fn handle_transform_query(&mut self, query: &str) {
        use kmd_core::transform;

        match transform::parse_transform_query(query) {
            Some(mut tq) => {
                if tq.text.is_empty() {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        if let Ok(text) = clipboard.get_text() {
                            tq.text = text;
                        }
                    }
                }
                // 검색 중에는 열지 않는다 — 타이핑하는 동안 키마다 탭이 열린다.
                // 실행 항목 하나만 보여주고, 실제 열기는 Enter에서 한다.
                let count = transform::build_transform_urls(
                    &tq,
                    &self.spell_providers,
                    &self.translate_providers,
                )
                .len();
                let item = transform::run_item(&tq, count, self.use_emoji);
                self.apply_contains_results(items_to_results(std::iter::once(item)));
            }
            None => {
                let items = transform::help_items(self.use_emoji);
                self.apply_contains_items(items);
            }
        }
    }

    /// :prompt / :pt 쿼리 처리
    pub(super) fn handle_prompt_query(&mut self, query: &str) {
        let sub = query
            .strip_prefix(":prompt")
            .or_else(|| query.strip_prefix(":pt"))
            .unwrap_or("")
            .trim();

        let templates = self.runtime_config.launcher.prompt_templates.clone();

        if sub.starts_with("add ") {
            let rest = sub.strip_prefix("add ").unwrap_or("").trim();
            if let Some(pos) = rest.find(char::is_whitespace) {
                let name = &rest[..pos];
                let body = rest[pos..].trim();
                if !kmd_core::prompt::validate_template_name(name) {
                    self.apply_contains_results(items_to_results(std::iter::once(IndexItem {
                        name: format!("❌ 잘못된 이름: '{name}'"),
                        path: "영문/숫자/하이픈/언더스코어만, 최대 32자".to_string(),
                        kind: ItemKind::SystemCommand,
                        source: Source::Plugin,
                        icon: if self.use_emoji { "\u{274C}" } else { "[!]" }.to_string(),
                        keywords: "kmd:settings:noop".to_string(),
                        icon_path: None,
                    })));
                } else if body.is_empty() {
                    self.apply_contains_results(items_to_results(std::iter::once(IndexItem {
                        name: "❌ 본문이 비어 있습니다".to_string(),
                        path: ":prompt add <name> <body> 형태로 입력하세요".to_string(),
                        kind: ItemKind::SystemCommand,
                        source: Source::Plugin,
                        icon: if self.use_emoji { "\u{274C}" } else { "[!]" }.to_string(),
                        keywords: "kmd:settings:noop".to_string(),
                        icon_path: None,
                    })));
                } else {
                    self.runtime_config
                        .launcher
                        .prompt_templates
                        .retain(|t| !t.name.eq_ignore_ascii_case(name));
                    self.runtime_config.launcher.prompt_templates.push(
                        kmd_core::config::PromptTemplate {
                            name: name.to_string(),
                            body: body.to_string(),
                        },
                    );
                    let templates_to_save = self.runtime_config.launcher.prompt_templates.clone();
                    save_config(move |c| c.launcher.prompt_templates = templates_to_save);
                    self.apply_contains_results(items_to_results(std::iter::once(IndexItem {
                        name: format!("✅ 템플릿 '{name}' 저장됨"),
                        path: format!("@ll :{name} <query> 형태로 사용"),
                        kind: ItemKind::SystemCommand,
                        source: Source::Plugin,
                        icon: if self.use_emoji { "\u{2705}" } else { "[OK]" }.to_string(),
                        keywords: "kmd:settings:noop".to_string(),
                        icon_path: None,
                    })));
                }
            } else {
                self.apply_contains_results(items_to_results(std::iter::once(IndexItem {
                    name: "사용법: :prompt add <name> <body>".to_string(),
                    path: "예: :prompt add review 코드를 리뷰해주세요".to_string(),
                    kind: ItemKind::SystemCommand,
                    source: Source::Plugin,
                    icon: if self.use_emoji {
                        "\u{2139}\u{FE0F}"
                    } else {
                        "[?]"
                    }
                    .to_string(),
                    keywords: "kmd:settings:noop".to_string(),
                    icon_path: None,
                })));
            }
            return;
        }

        if sub.starts_with("remove ") || sub.starts_with("rm ") || sub.starts_with("del ") {
            let name = sub
                .strip_prefix("remove ")
                .or_else(|| sub.strip_prefix("rm "))
                .or_else(|| sub.strip_prefix("del "))
                .unwrap_or("")
                .trim();
            if name.is_empty() {
                self.apply_contains_results(items_to_results(std::iter::once(IndexItem {
                    name: "사용법: :prompt remove <name>".to_string(),
                    path: "삭제할 템플릿 이름을 입력하세요".to_string(),
                    kind: ItemKind::SystemCommand,
                    source: Source::Plugin,
                    icon: if self.use_emoji {
                        "\u{2139}\u{FE0F}"
                    } else {
                        "[?]"
                    }
                    .to_string(),
                    keywords: "kmd:settings:noop".to_string(),
                    icon_path: None,
                })));
            } else if templates.iter().any(|t| t.name.eq_ignore_ascii_case(name)) {
                let name_owned = name.to_string();
                let display_name = name.to_string();
                self.runtime_config
                    .launcher
                    .prompt_templates
                    .retain(|t| !t.name.eq_ignore_ascii_case(&name_owned));
                let templates_to_save = self.runtime_config.launcher.prompt_templates.clone();
                save_config(move |cfg| cfg.launcher.prompt_templates = templates_to_save);
                self.apply_contains_results(items_to_results(std::iter::once(IndexItem {
                    name: format!("✅ 템플릿 '{display_name}' 삭제됨"),
                    path: String::new(),
                    kind: ItemKind::SystemCommand,
                    source: Source::Plugin,
                    icon: if self.use_emoji { "\u{2705}" } else { "[OK]" }.to_string(),
                    keywords: "kmd:settings:noop".to_string(),
                    icon_path: None,
                })));
            } else {
                self.apply_contains_results(items_to_results(std::iter::once(IndexItem {
                    name: format!("❌ 템플릿 '{name}'을 찾을 수 없습니다"),
                    path: String::new(),
                    kind: ItemKind::SystemCommand,
                    source: Source::Plugin,
                    icon: if self.use_emoji { "\u{274C}" } else { "[!]" }.to_string(),
                    keywords: "kmd:settings:noop".to_string(),
                    icon_path: None,
                })));
            }
            return;
        }

        let filter = sub.strip_prefix("list").unwrap_or(sub).trim();
        let items = kmd_core::prompt::list_templates_as_items(&templates, filter, self.use_emoji);
        self.apply_contains_items(items);
    }

    pub(super) fn handle_calc_query(&mut self, query: &str) {
        let items = kmd_core::query_prefix::calc_items(query, self.use_emoji);
        self.apply_contains_items(items);
    }

    pub(super) fn handle_emoji_query(&mut self, query: &str) {
        let items = kmd_core::query_prefix::emoji_items(query);
        self.apply_contains_items(items);
    }

    pub(super) fn handle_shell_query(&mut self, query: &str) {
        let items = kmd_core::query_prefix::shell_items(query, self.use_emoji);
        self.apply_contains_items(items);
    }

    /// `;` / `:clip` — 클립보드 히스토리 검색 (docs/12 흐름 B).
    /// 히스토리는 데몬에 있으므로 IPC로 조회한다. 결과 항목은
    /// `kmd:clip:<id>:<slot>` 키워드 마커를 달아, Enter 시 handle_submit이 이전 앱
    /// 붙여넣기(또는 Cmd/Ctrl+Enter 복사)로 라우팅한다.
    ///
    /// GUI 스레드에서 IPC 타임아웃(수 초)을 기다리지 않도록 블로킹 요청을 워커로
    /// 보낸다 — 데몬이 느리거나 멈춰도 입력이 얼지 않는다. 응답이 도착하면
    /// [`Message::ClipSearchFinished`]로 돌아오고, 그 사이 쿼리가 바뀌어
    /// generation이 달라진 응답은 update에서 폐기된다.
    pub(super) fn handle_clip_query(&mut self, query: &str, generation: u64) -> Task<Message> {
        let q = query
            .strip_prefix(';')
            .or_else(|| query.strip_prefix(":clip"))
            .unwrap_or("")
            .trim();

        let req = kmd_core::ipc::Request::ClipHistory {
            query: q.to_string(),
            limit: 30,
        };
        self.clear_results_state(kmd_core::SearchMode::Fuzzy);
        Task::future(async move {
            let result = tokio::task::spawn_blocking(move || {
                match kmd_core::ipc::send_request_result(&req) {
                    Ok(kmd_core::ipc::Response::ClipHistory { hits }) => Ok(hits),
                    Ok(kmd_core::ipc::Response::Error { message }) => Err(message),
                    Ok(other) => Err(format!("예기치 않은 데몬 응답: {other:?}")),
                    Err(_) => Err(
                        "데몬이 실행 중인지, [clipboard] history_enabled=true 인지 확인하세요"
                            .to_string(),
                    ),
                }
            })
            .await
            .unwrap_or_else(|e| Err(format!("클립보드 검색 작업 실패: {e}")));
            Message::ClipSearchFinished { generation, result }
        })
    }

    /// :keys / :k — 키 맵핑 치트시트
    pub(super) fn handle_keys_query(&mut self) {
        let items = kmd_core::keymap::keybinding_cheatsheet(
            &self.runtime_config,
            self.use_emoji,
            kmd_core::keymap::CheatsheetApp::Desktop,
        );
        self.apply_contains_items(items);
    }

    /// :keymap / :km 쿼리 처리
    pub(super) fn handle_keymap_query(&mut self, query: &str) {
        let items =
            kmd_core::query_prefix::keymap_query_items(&self.runtime_config, query, self.use_emoji);
        self.apply_contains_items(items);
    }

    pub(super) fn handle_main_search(&mut self, query: &str) {
        let (mode, mut results) = self.engine.search(query, SEARCH_LIMIT);
        self.search_mode = mode;

        // 실행 이력 기반으로 자주 사용하는 항목의 점수를 높인다.
        // 키 입력마다 호출되는 핫패스 — DB 조회 대신 부팅 시 로드한 맵 사용.
        kmd_core::history::boost_results_with_map(&mut results, &self.frecency);

        if builtin_calc::looks_like_math(query) {
            let calc = builtin_calc::CalcExtension;
            let calc_items = calc.search_with_emoji(query, self.use_emoji);
            let calc_results: Vec<kmd_core::SearchResult> = calc_items
                .into_iter()
                .map(|item| kmd_core::SearchResult {
                    item,
                    score: SCORE_PLUGIN,
                })
                .collect();
            results.splice(0..0, calc_results);
        }

        if results.is_empty() && !query.is_empty() {
            if let Some(relaxed) = relaxed_hangul_query(query) {
                let mut relaxed_results = self.engine.search_with_mode(
                    kmd_core::SearchMode::Contains,
                    &relaxed,
                    SEARCH_LIMIT,
                );
                if !relaxed_results.is_empty() {
                    kmd_core::history::boost_results_with_map(&mut relaxed_results, &self.frecency);
                    results = relaxed_results;
                    self.search_mode = kmd_core::SearchMode::Contains;
                }
            }
        }

        if results.is_empty() && !query.is_empty() {
            results = self.build_fallback_suggestions(query);
            self.search_mode = kmd_core::SearchMode::Contains;
        }

        // 오타/미지원 : 명령 안내를 최상단에 표시 (검색 폴스루는 유지)
        if let Some(hint) = kmd_core::query_prefix::unknown_command_hint(query, self.use_emoji) {
            results.insert(
                0,
                kmd_core::SearchResult {
                    item: hint,
                    score: SCORE_PLUGIN,
                },
            );
        }

        self.commit_results(results, self.search_mode, true);
    }

    /// `:f /경로 쿼리` — 지정 폴더 안에서 파일/폴더를 즉석 검색 (kmd-core 공용 구현).
    pub(super) fn handle_folder_search(&mut self, raw: &str) {
        let results = kmd_core::folder_search::folder_search_results(raw, self.use_emoji);
        self.commit_results(results, kmd_core::SearchMode::Contains, true);
    }

    /// `?질의` — 문서 본문 검색 (FTS5, docs/15). 공유 kmd.db를 지연 오픈해 읽는다.
    pub(super) fn handle_content_search(&mut self, raw: &str) {
        if self.content_db.is_none() && self.runtime_config.launcher.content_search.enabled {
            let path = kmd_core::Config::default_data_dir().join(kmd_core::DB_FILENAME);
            match kmd_core::db::Database::open(&path) {
                Ok(db) => self.content_db = Some(db),
                Err(e) => tracing::warn!("본문 검색 DB 열기 실패: {e}"),
            }
        }
        let mut results = kmd_core::content_index::launcher_results(
            self.content_db.as_ref(),
            raw,
            self.use_emoji,
            SEARCH_LIMIT,
            self.runtime_config.launcher.content_search.enabled,
        );
        // 빈 질의(`?`만 입력) → 사용법 안내 아래에 "자주 변하는 폴더" 제안 (docs/15 P2)
        if kmd_core::content_index::strip_query(raw).is_empty() {
            results.extend(kmd_core::folder_suggest::suggestion_results(
                &self.runtime_config.launcher,
                self.use_emoji,
                3,
            ));
        }
        self.commit_results(results, kmd_core::SearchMode::Contains, true);
    }

    fn build_fallback_suggestions(&self, query: &str) -> Vec<kmd_core::SearchResult> {
        use kmd_core::web::{self, WEB_SERVICES};

        const FALLBACK_IDS: &[&str] = &[
            "google",
            "perplexity",
            "chatgpt",
            "claude",
            "gemini",
            "naver_search",
            "youtube",
            "github",
            "stackoverflow",
            "wikipedia",
        ];

        let emoji = self.use_emoji;
        let mut items: Vec<kmd_core::SearchResult> = FALLBACK_IDS
            .iter()
            .filter_map(|id| WEB_SERVICES.iter().find(|s| s.id == *id))
            .map(|service| {
                let item = web::search_result_item(service, query, emoji);
                kmd_core::SearchResult { item, score: 0 }
            })
            .collect();

        let multi_items = web::multi_llm_result_items(query, &self.selected_llm_providers, emoji);
        if let Some(multi_item) = multi_items.into_iter().next() {
            items.insert(
                5,
                kmd_core::SearchResult {
                    item: multi_item,
                    score: 0,
                },
            );
        }

        // 폴더 지정 검색 힌트 — 인덱스에 없는 파일을 찾을 때 유용함을 알림
        items.push(kmd_core::SearchResult {
            item: kmd_core::index::IndexItem {
                name: format!(":f /폴더경로 {query}  — 폴더 직접 지정 검색"),
                path: format!(":f  {query}"),
                kind: kmd_core::index::ItemKind::SystemCommand,
                source: kmd_core::index::Source::Plugin,
                icon: "F>".to_string(),
                keywords: "kmd:folder_search:suggest".to_string(),
                icon_path: None,
            },
            score: 0,
        });

        items
    }
}

/// 클립보드 히스토리 항목 → 런처 결과. 키워드에 `kmd:clip:<id>:<slot>` 마커를
/// 담아 Enter 시 launch_selected가 이전 앱 붙여넣기로 라우팅한다. 실행은 안정
/// ID를 쓴다 — 검색 뒤 새 복사로 슬롯이 밀려도 같은 항목을 붙여넣기 위함.
/// slot은 구버전 데몬(id=0) 폴백용으로만 함께 싣는다.
pub(super) fn clip_item(hit: &kmd_core::ipc::ClipHit, use_emoji: bool) -> IndexItem {
    IndexItem {
        name: if hit.preview.is_empty() {
            "(빈 줄)".to_string()
        } else {
            hit.preview.clone()
        },
        path: hit.preview.clone(),
        icon: if use_emoji { "\u{1F4CB}" } else { "[CLIP]" }.to_string(),
        kind: ItemKind::SystemCommand,
        source: Source::Plugin,
        keywords: format!("kmd:clip:{}:{}", hit.id, hit.slot),
        icon_path: None,
    }
}

/// 클립보드 안내 항목 (히스토리 비었거나 데몬 미사용, 붙여넣기 실패 표시 겸용)
/// — Enter해도 아무 일 없음. launch.rs의 붙여넣기 실패 표시에서도 쓴다.
pub(super) fn clip_notice(title: &str, usage: &str, use_emoji: bool) -> IndexItem {
    IndexItem {
        name: title.to_string(),
        path: usage.to_string(),
        icon: if use_emoji { "\u{1F4CB}" } else { "[CLIP]" }.to_string(),
        kind: ItemKind::SystemCommand,
        source: Source::Plugin,
        keywords: "kmd:clip:notice".to_string(),
        icon_path: None,
    }
}
