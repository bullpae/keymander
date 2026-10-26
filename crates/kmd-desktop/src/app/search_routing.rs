use super::*;

use crate::query_prefix::{prefix_of, Prefix};

impl App {
    pub(super) fn perform_search(&mut self) -> Task<Message> {
        // 모든 검색이 세대를 올린다 — 비동기 경로(클립보드)의 늦은 응답은
        // 세대가 달라져 update에서 폐기된다.
        self.search_generation = self.search_generation.wrapping_add(1);
        let generation = self.search_generation;
        // 일반 검색이 아니면 엔진을 안 거치므로 이전 미완성 표시를 지운다
        self.engine.clear_pending();
        let query = self.query.clone();
        let trimmed = query.trim();
        // 알림은 짧은 수명 — 다음 검색이 시작되면 지운다 (TUI와 같은 규칙).
        self.status_message = None;

        if trimmed.is_empty() {
            self.clear_results_state(kmd_core::SearchMode::Fuzzy);
            // 빈 화면 대신 최근 실행 항목 — 처음 연 사람에게 유일한 학습 단서다.
            let mut recent = kmd_core::query_prefix::recent_items(
                &self.db,
                RECENT_DISPLAY_LIMIT,
                self.use_emoji,
            );
            self.say_empty_state(&mut recent);
            if !recent.is_empty() {
                self.apply_contains_items(recent);
            }
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
            Prefix::Dojo => {
                self.apply_contains_items(kmd_core::query_prefix::dojo_items(self.use_emoji))
            }
            Prefix::FolderSearch => self.handle_folder_search(trimmed),
            Prefix::Scope => self.handle_scope_query(trimmed),
            Prefix::ContentSearch => self.handle_content_search(trimmed),
            Prefix::Clipboard => {
                post_task = self.handle_clip_query(trimmed, generation);
            }
            Prefix::Route => {
                post_task = self.handle_route_query(trimmed, generation);
            }
            Prefix::Say => self.handle_say_query(trimmed),
            Prefix::General => {
                self.handle_main_search(trimmed);
                // 항목이 많아 퍼지 검색이 시간 안에 안 끝났다 — 잠시 뒤 같은 질의로 다시 묻는다.
                // 그 사이 입력이 바뀌면 세대가 달라져 버려진다.
                if self.engine.is_pending() {
                    post_task = Task::future(async move {
                        tokio::time::sleep(Duration::from_millis(SEARCH_RETRY_MS)).await;
                        Message::SearchRetry(generation)
                    });
                }
            }
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
        let (items, _listing) =
            web::web_query_items(query, &self.runtime_config.launcher, self.use_emoji);
        self.commit_results(
            items_to_results(items),
            kmd_core::SearchMode::Contains,
            true,
        );
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

        match transform::parse_transform_query_or_clipboard(query) {
            Some(tq) => {
                // 검색 중에는 열지 않는다 — 타이핑하는 동안 키마다 탭이 열린다.
                // 실행 항목 하나만 보여주고, 실제 열기는 Enter에서 한다.
                let count = transform::build_transform_urls(
                    &tq,
                    &self.runtime_config.launcher.spell_providers,
                    &self.runtime_config.launcher.translate_providers,
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
    /// `:prompt` — 입력 중에는 미리보기만 한다. add/rm 확정은 Enter
    /// (`launch_selected` → `commit_prompt_command`)에서만 일어난다.
    pub(super) fn handle_prompt_query(&mut self, query: &str) {
        let cmd = kmd_core::prompt::parse_command(query);
        let items = kmd_core::prompt::command_items(
            &cmd,
            &self.runtime_config.launcher.prompt_templates,
            self.use_emoji,
        );
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

    /// `:길` / `:route` — 카카오맵 길찾기 (docs/20).
    ///
    /// 첫 화면은 네트워크 없이 낸다(현재 위치 자리는 `origin`, 처음 가는 곳은 장소
    /// 검색 링크). 그다음 워커에서 데몬에게 현재 위치를 묻고, 카카오 키가 있으면
    /// 장소 검색 API 로 후보를 찾아 [`Message::RouteSearchFinished`] 로 바꿔 끼운다.
    /// 키 입력마다 호출하지 않도록 잠깐 기다렸다가 그 사이 새 검색이 시작됐으면 건너뛴다.
    pub(super) fn handle_route_query(&mut self, query: &str, generation: u64) -> Task<Message> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static LATEST: AtomicU64 = AtomicU64::new(0);

        let cfg = self.runtime_config.launcher.route.clone();
        let use_emoji = self.use_emoji;
        let plan = kmd_core::route::parse(&cfg, query);
        let key = cfg.kakao_key().filter(|_| plan.needs_lookup());
        let mut first = kmd_core::route::items(&cfg, query, use_emoji);
        if plan.needs_here() {
            first.insert(
                0,
                kmd_core::route::notice("현재 위치를 확인하는 중…", "", use_emoji),
            );
        }
        self.apply_contains_items(first);
        if !plan.needs_here() && key.is_none() {
            return Task::none();
        }
        LATEST.store(generation, Ordering::Relaxed);
        Task::future(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            if LATEST.load(Ordering::Relaxed) != generation {
                return Message::RouteSearchFinished {
                    generation,
                    result: Err(String::new()),
                };
            }
            let result = tokio::task::spawn_blocking(move || {
                use kmd_core::route;
                let (here, note) = if plan.needs_here() {
                    current_location()
                } else {
                    (None, None)
                };
                let plan = plan.with_here(&cfg, here);
                let mut items = match &key {
                    Some(k) if plan.needs_lookup() => route::lookup_items(k, &plan, use_emoji)?,
                    _ => route::items_for(&cfg, &plan, use_emoji),
                };
                if let Some(note) = note {
                    items.insert(
                        0,
                        route::notice("현재 위치를 쓰지 못했습니다", &note, use_emoji),
                    );
                }
                Ok(items)
            })
            .await
            .unwrap_or_else(|e| Err(format!("장소 검색 작업 실패: {e}")));
            Message::RouteSearchFinished { generation, result }
        })
    }

    /// 장소 검색 결과 반영 — 늦은 응답(쿼리가 바뀜)은 버린다
    pub(super) fn handle_route_search_finished(
        &mut self,
        generation: u64,
        result: Result<Vec<kmd_core::IndexItem>, String>,
    ) -> Task<Message> {
        if generation != self.search_generation || prefix_of(self.query.trim()) != Prefix::Route {
            return Task::none();
        }
        match result {
            Ok(items) => self.apply_contains_items(items),
            Err(message) => {
                tracing::warn!("장소 검색 실패: {message}");
                let mut items = vec![kmd_core::route::notice(
                    "장소 검색에 실패했습니다",
                    &message,
                    self.use_emoji,
                )];
                items.extend(kmd_core::route::items(
                    &self.runtime_config.launcher.route,
                    self.query.trim(),
                    self.use_emoji,
                ));
                self.apply_contains_items(items);
            }
        }
        Task::none()
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

        // URL로 판정된 입력: 파일 검색은 그대로 두고 "열기" 항목을 맨 위에 세운다.
        // 이게 없으면 example.com 을 쳐도 열 방법이 없다.
        if mode == kmd_core::SearchMode::Url {
            let (_, normalized_url) = kmd_core::SearchMode::detect(query);
            results = self.engine.search_with_mode(
                kmd_core::SearchMode::Contains,
                query.trim(),
                SEARCH_LIMIT,
            );
            results.insert(
                0,
                kmd_core::SearchResult {
                    item: kmd_core::query_prefix::url_open_item(&normalized_url, self.use_emoji),
                    score: SCORE_PLUGIN,
                },
            );
        }

        // 새 파일 검색 엔진(docs/22): 할당량에 잘린 옛 파일 결과를 전체 색인 결과로 바꾸고,
        // 앱·실행 파일까지 한 척도(rank::merge)로 다시 매긴다 — 실행 이력도 그 안에서 한 번.
        // 실행 이력은 키 입력마다 DB 조회 대신 부팅 시 로드한 맵을 쓴다.
        if let Some(index) = &self.file_index {
            let weights = &self.runtime_config.launcher.kind_weights;
            let files = index.search(
                query,
                SEARCH_LIMIT,
                &self.file_boost,
                weights,
                self.use_emoji,
            );
            results = kmd_core::rank::merge(
                std::mem::take(&mut results),
                files,
                query,
                weights,
                &self.frecency,
            );
            results.truncate(SEARCH_LIMIT);
        } else {
            kmd_core::history::boost_results_with_map(&mut results, &self.frecency);
        }

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

        // 미완성이면 빈 결과가 진짜가 아니다 — 완화 검색·웹 제안을 띄우지 않고 재검색을 기다린다
        let pending = self.engine.is_pending();
        if results.is_empty() && !query.is_empty() && !pending {
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

        if pending {
            // 계산기·경로 추가 행처럼 엔진 밖 결과만 그대로 둔다
        } else if results.is_empty() && !query.is_empty() {
            results = self.build_fallback_suggestions(query);
            self.search_mode = kmd_core::SearchMode::Contains;
        } else if self.search_mode == kmd_core::SearchMode::Fuzzy
            && !builtin_calc::looks_like_math(query)
            && !has_literal_match(&results, query)
        {
            // 퍼지 결과만 있고 질의가 그대로 든 항목이 없으면 (`gmail` →
            // 글자만 흩어져 걸린 앱들) 결과가 비지 않아 웹 제안이 영영 안 나온다.
            // 약어 매칭(`vsc` → Visual Studio Code)일 수도 있으니 상위 몇 개는
            // 그대로 두고 그 아래에 대체 제안을 끼운다.
            let at = results.len().min(WEAK_MATCH_KEEP_TOP);
            let fallback = self.build_fallback_suggestions(query);
            results.splice(at..at, fallback);
        }

        // 범위 밖 폴더 경로를 그대로 쳤으면 맨 위에 "검색 범위에 추가" (docs/18 E1)
        if let Some(item) = kmd_core::scope::path_add_suggestion(
            &self.runtime_config.launcher,
            query,
            self.use_emoji,
        ) {
            results.insert(
                0,
                kmd_core::SearchResult {
                    item,
                    score: SCORE_PLUGIN,
                },
            );
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
            match kmd_core::db::Database::open_default(kmd_core::db::DbRole::Shared) {
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

        // 검색 결과가 비는 가장 흔한 원인은 "그 폴더가 검색 범위 밖"이다.
        // 웹 검색을 권하기 전에 범위 추가 제안을 먼저 보여준다 — 예전에는 이
        // 제안이 `?` 프리픽스 안에만 있어 사용자가 찾을 수 없었다.
        let mut items: Vec<kmd_core::SearchResult> =
            kmd_core::folder_suggest::suggestion_results(&self.runtime_config.launcher, emoji, 2);
        // 제안에 원하는 폴더가 없을 때 직접 넣는 길 (docs/18 E4)
        items.push(kmd_core::SearchResult {
            item: kmd_core::scope::empty_result_hint(emoji),
            score: 0,
        });

        let web_items: Vec<kmd_core::SearchResult> = FALLBACK_IDS
            .iter()
            .filter_map(|id| WEB_SERVICES.iter().find(|s| s.id == *id))
            .map(|service| {
                let item = web::search_result_item(service, query, emoji);
                kmd_core::SearchResult { item, score: 0 }
            })
            .collect();
        items.extend(web_items);

        let multi_items = web::multi_llm_result_items(
            query,
            &self.runtime_config.launcher.multi_llm_providers,
            emoji,
        );
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

/// 이름 매칭이 약할 때 대체 제안 위에 남겨 둘 퍼지 결과 수.
const WEAK_MATCH_KEEP_TOP: usize = 5;

/// 결과 중 질의 토큰이 모두 (대소문자 무시) 이름이나 경로에 그대로 들어간
/// 항목이 있는가 — 글자만 흩어져 걸린 퍼지 매칭과 구분한다.
fn has_literal_match(results: &[kmd_core::SearchResult], query: &str) -> bool {
    let tokens: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if tokens.is_empty() {
        return true;
    }
    results.iter().any(|r| {
        let name = r.item.name.to_lowercase();
        let path = r.item.path.to_lowercase();
        tokens
            .iter()
            .all(|t| name.contains(t.as_str()) || path.contains(t.as_str()))
    })
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

/// 데몬에게 현재 위치를 묻는다 — (장소, 못 얻은 이유). 블로킹 IPC.
fn current_location() -> (Option<kmd_core::route::Place>, Option<String>) {
    use kmd_core::ipc::{send_request_result, Request, Response};
    match send_request_result(&Request::Location) {
        Ok(Response::Location {
            lat,
            lng,
            accuracy_m,
            age_secs,
        }) => (
            Some(kmd_core::route::here_place(lat, lng, accuracy_m, age_secs)),
            None,
        ),
        Ok(Response::Error { message }) => (None, Some(message)),
        Ok(other) => (None, Some(format!("예기치 않은 데몬 응답: {other:?}"))),
        Err(_) => (
            None,
            Some("데몬이 실행 중이 아니라 현재 위치를 알 수 없습니다".to_string()),
        ),
    }
}
