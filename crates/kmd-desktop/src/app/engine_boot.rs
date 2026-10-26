//! 엔진 부팅 — quick/full 엔진 비동기 로드, 유휴 워밍업, 엔진 교체 처리

use super::*;

/// 비동기 엔진 로드 결과.
///
/// 엔진을 만들 때 읽은 config를 **그대로** 넘긴다. 예전에는 파생 필드 9개를
/// 하나씩 복사해 넘기고, 받는 쪽에서 `runtime_config`를 위해 디스크를 한 번 더
/// 읽었다 — 두 번 읽는 사이 파일이 바뀌면 엔진과 설정이 서로 다른 config를 보게 된다.
pub(super) struct EngineLoadResult {
    engine: kmd_core::SearchEngine,
    config: kmd_core::Config,
}

pub(super) type EngineSlot = Arc<Mutex<Option<EngineLoadResult>>>;

/// 비동기 quick 엔진 로드 결과 슬롯 — 부팅 직후 창 표시를 막지 않기 위해
/// PATH 스캔/캐시 로드를 백그라운드 스레드에서 수행한다.
pub(super) type QuickEngineSlot = Arc<Mutex<Option<kmd_core::SearchEngine>>>;

impl App {
    /// quick 엔진(PATH + 시스템 명령)을 백그라운드 스레드에서 로드한다.
    ///
    /// 이전에는 App 생성자에서 동기로 빌드해 창 표시 자체가 디스크 스캔을
    /// 기다렸다 — 캐시 미스(첫 설치)나 느린 VM에서 부팅이 수백 ms~수 초
    /// 지연되는 원인. 빈 엔진으로 즉시 창을 띄우고 완료 시 교체한다.
    pub(super) fn spawn_quick_engine_load_task(
        slot: QuickEngineSlot,
        config: kmd_core::Config,
    ) -> Task<Message> {
        Task::future(async move {
            match tokio::task::spawn_blocking(move || {
                let engine = crate::engine::create_quick_search_engine(&config);
                if let Ok(mut guard) = slot.lock() {
                    *guard = Some(engine);
                } else {
                    tracing::error!("quick_engine_slot mutex poisoned — quick 엔진 저장 실패");
                }
            })
            .await
            {
                Ok(()) => {}
                Err(e) => tracing::error!("quick 엔진 로드 태스크 패닉: {e}"),
            }
            Message::QuickEngineReady
        })
    }

    pub(super) fn initial_boot_tasks(
        input_id: iced::widget::Id,
        skip_warmup: bool,
    ) -> Task<Message> {
        let focus_task = iced::widget::operation::focus::<Message>(input_id);
        let id_task = window::oldest().map(Message::GotWindowId);
        #[cfg(target_os = "windows")]
        let focus_chain = {
            let delayed = Task::future(async {
                tokio::time::sleep(Duration::from_millis(16)).await;
                Message::DelayedRefocus
            });
            Task::batch([focus_task, delayed])
        };
        #[cfg(not(target_os = "windows"))]
        let focus_chain = focus_task;
        if skip_warmup {
            Task::batch([focus_chain, id_task])
        } else {
            let warmup_task = Self::schedule_warmup_tick(0);
            Task::batch([focus_chain, id_task, warmup_task])
        }
    }

    pub(super) fn with_activity_warmup(&mut self, base_task: Task<Message>) -> Task<Message> {
        if self.full_warmup_started {
            base_task
        } else {
            self.warmup_token = self.warmup_token.wrapping_add(1);
            let warmup_task = Self::schedule_warmup_tick(self.warmup_token);
            Task::batch([base_task, warmup_task])
        }
    }

    fn schedule_warmup_tick(token: u64) -> Task<Message> {
        Task::future(async move {
            tokio::time::sleep(Duration::from_millis(WARMUP_IDLE_MS)).await;
            Message::WarmupTick(token)
        })
    }

    /// 열 때 대조 — 창을 띄운 뒤 뒤에서 폴더 시각을 확인해 바뀐 폴더만 다시 읽는다 (docs/22 §3.8)
    pub(super) fn spawn_file_reconcile_task(
        index: Option<Arc<kmd_core::filesearch::FileIndex>>,
        config: &kmd_core::Config,
    ) -> Task<Message> {
        let Some(index) = index else {
            return Task::none();
        };
        let cfg = kmd_core::filesearch::scan::ScanConfig::from_launcher(&config.launcher);
        Task::future(async move {
            match tokio::task::spawn_blocking(move || index.reconciled(&cfg)).await {
                Ok(Some(updated)) => Some(Message::FileIndexUpdated(Arc::new(updated))),
                _ => None,
            }
        })
        .and_then(Task::done)
    }

    pub(super) fn spawn_full_engine_load_task(&self) -> Task<Message> {
        let slot = self.engine_slot.clone();
        Task::future(async move {
            match tokio::task::spawn_blocking(move || {
                let config = crate::engine::load_config();
                let eng = crate::engine::create_search_engine(&config);
                if let Ok(mut guard) = slot.lock() {
                    *guard = Some(EngineLoadResult {
                        engine: eng,
                        config,
                    });
                } else {
                    tracing::error!("engine_slot mutex poisoned — 엔진 로드 결과 저장 실패");
                }
            })
            .await
            {
                Ok(()) => {}
                Err(e) => tracing::error!("엔진 로드 태스크 패닉: {e}"),
            }
            Message::EngineReady
        })
    }

    /// 현재 결과 목록의 아이콘을 백그라운드 스레드에서 미리 추출해 캐시에 채운다.
    ///
    /// `view()`는 캐시만 조회하므로(`cached_icon_for_item`), 렌더 스레드가 OS
    /// 셸 API 호출로 멈추지 않는다. 완료되면 `IconsReady`로 리렌더를 유도한다.
    pub(super) fn spawn_icon_prefetch(&self) -> Task<Message> {
        let items: Vec<(ItemKind, String, Option<String>)> = self
            .results
            .iter()
            .map(|r| (r.item.kind, r.item.path.clone(), r.item.icon_path.clone()))
            .collect();
        if items.is_empty() {
            return Task::none();
        }
        // 새로 추출된 아이콘이 없으면 IconsReady를 보내지 않는다 — 키 입력마다
        // 불필요한 리렌더 프레임을 만들지 않기 위함 (소프트웨어 렌더러에서 체감 큼).
        Task::future(async move {
            tokio::task::spawn_blocking(move || crate::app_icons::prefetch_icons(&items))
                .await
                .unwrap_or(0)
        })
        .then(|newly_extracted| {
            if newly_extracted > 0 {
                Task::done(Message::IconsReady)
            } else {
                Task::none()
            }
        })
    }

    /// IME 조합이 진행 중일 가능성이 높은지 — 최근 입력 직후이거나 자모 꼬리 상태.
    fn is_typing_in_progress(&self) -> bool {
        self.last_query_changed_at.elapsed() < Duration::from_millis(350)
            || self.should_limit_reorder_for_ime()
    }

    pub(super) fn handle_warmup_tick(&mut self, token: u64) -> Task<Message> {
        if self.full_warmup_started || token != self.warmup_token {
            return Task::none();
        }

        // IME 조합/최근 입력 중이면 warmup을 미룬다. loading 토글로 인한 뷰
        // 재렌더가 첫 한글 조합(marked text)을 깨뜨리는 것을 방지. (macOS 자모 분리)
        if self.is_typing_in_progress() {
            self.log_ime_state("WarmupTick:defer_for_ime");
            self.warmup_token = self.warmup_token.wrapping_add(1);
            return Self::schedule_warmup_tick(self.warmup_token);
        }

        self.full_warmup_started = true;
        self.loading = true;
        tracing::info!(
            "Starting full engine warmup after {}ms idle",
            WARMUP_IDLE_MS
        );
        self.spawn_full_engine_load_task()
    }

    pub(super) fn handle_engine_ready(&mut self) -> Task<Message> {
        // warmup이 입력 전에 시작된 경우, 엔진 스왑/loading 토글이 조합 도중
        // 재렌더를 일으켜 자모가 분리될 수 있다. 조합 중이면 스왑 전체를 미룬다.
        // (engine_slot이 결과를 계속 보관하므로 이후 재발화 시 그대로 적용)
        if self.is_typing_in_progress() {
            self.log_ime_state("EngineReady:defer_for_ime");
            return Task::future(async {
                tokio::time::sleep(Duration::from_millis(400)).await;
                Message::EngineReady
            });
        }
        let loaded = self
            .engine_slot
            .lock()
            .unwrap_or_else(|e| {
                tracing::error!("engine_slot mutex poisoned — 복구 시도");
                e.into_inner()
            })
            .take();

        if let Some(res) = loaded {
            let EngineLoadResult { engine, config } = res;
            self.engine = engine;
            self.full_engine_loaded = true;
            self.use_emoji = config.general.emoji_icons;
            self.runtime_config = config;
            self.loading = false;
            crate::launch_perf::mark_engine();
            tracing::info!("Search engine ready");
            // `:scope` 변경 결과 — 재검색이 알림을 지우므로 그 뒤에 남긴다
            let scope_msg = self.take_scope_report_message();
            if !self.query.trim().is_empty() {
                // IME 조합 중이면 검색/뷰 갱신을 미뤄 자모 분리를 막는다.
                if self.is_typing_in_progress() {
                    self.status_message = scope_msg.or(self.status_message.take());
                    return Task::future(async {
                        tokio::time::sleep(Duration::from_millis(400)).await;
                        Message::EngineSwapSettled
                    });
                }
                let task = self.perform_search();
                if scope_msg.is_some() {
                    self.status_message = scope_msg;
                }
                return task;
            }
            if scope_msg.is_some() {
                self.status_message = scope_msg;
            }
        } else {
            self.loading = false;
        }
        Task::none()
    }

    pub(super) fn handle_engine_swap_settled(&mut self) -> Task<Message> {
        if self.query.trim().is_empty() {
            return Task::none();
        }
        // 아직 입력 중이면 다시 미룬다 — 조합이 끝난 뒤에만 갱신.
        if self.is_typing_in_progress() {
            return Task::future(async {
                tokio::time::sleep(Duration::from_millis(400)).await;
                Message::EngineSwapSettled
            });
        }
        self.perform_search()
    }

    pub(super) fn handle_quick_engine_ready(&mut self) -> Task<Message> {
        // full 엔진이 먼저 준비된 경우 quick 결과는 폐기 (더 작은 인덱스)
        if self.full_engine_loaded {
            return Task::none();
        }
        // IME 조합 중이면 스왑을 미룬다 — EngineReady와 동일한 보호.
        // 단, 쿼리가 비어 있으면 조합 중일 수 없으므로 즉시 스왑한다
        // (last_query_changed_at이 부팅 시각으로 초기화되어 있어
        //  is_typing_in_progress만 보면 부팅 직후 항상 지연된다).
        if !self.query.is_empty() && self.is_typing_in_progress() {
            self.log_ime_state("QuickEngineReady:defer_for_ime");
            return Task::future(async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                Message::QuickEngineReady
            });
        }
        let loaded = self
            .quick_engine_slot
            .lock()
            .unwrap_or_else(|e| {
                tracing::error!("quick_engine_slot mutex poisoned — 복구 시도");
                e.into_inner()
            })
            .take();
        if let Some(engine) = loaded {
            self.engine = engine;
            self.loading = false;
            crate::launch_perf::mark_engine();
            tracing::info!(
                "Quick engine applied {} ms after boot",
                self.app_started_at.elapsed().as_millis()
            );
            if !self.query.trim().is_empty() {
                return self.perform_search();
            }
        }
        Task::none()
    }
}
