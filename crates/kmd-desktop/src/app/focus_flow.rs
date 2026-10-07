use super::*;

impl App {
    fn unfocused_typing_grace_ms(&self) -> u64 {
        if cfg!(target_os = "macos") {
            1500
        } else {
            500
        }
    }

    fn focused_typing_grace_ms(&self) -> u64 {
        if cfg!(target_os = "macos") {
            1500
        } else {
            700
        }
    }

    fn should_hold_unfocus_for_ime(&self) -> bool {
        if self.query.is_empty() {
            return false;
        }
        let typing_recently = self.last_query_changed_at.elapsed()
            < Duration::from_millis(self.unfocused_typing_grace_ms());
        if typing_recently {
            return true;
        }
        cfg!(target_os = "macos") && self.should_limit_reorder_for_ime()
    }

    pub(super) fn handle_got_raw_window_id(&mut self, raw_id: u64) -> Task<Message> {
        self.log_ime_state("GotRawWindowId:start");
        self.raw_window_id = Some(raw_id);
        // 투명 창이면 라운드를 카드가 직접 그린다 — DWM 코너 클립(고정 8px)을
        // 함께 걸면 우리가 그린 모서리가 잘려 형태가 깨진다.
        if !crate::window_transparent() {
            crate::platform::apply_native_rounded_corners(raw_id);
        }
        crate::platform::force_foreground(raw_id);
        if self.reset_ime_on_launch {
            crate::platform::force_english_ime(raw_id);
        }
        // 리사이즈 잔상 교정 (macOS CAMetalLayer) — 창이 살아 있는 지금이 가장 이른 시점.
        let stabilize = self.handle_stabilize_surface_layer(0);
        let rest = if self.query.is_empty() {
            let focus = self.request_focus();
            #[cfg(target_os = "windows")]
            {
                Task::batch([focus, Self::schedule_focus_retry(1)])
            }
            #[cfg(not(target_os = "windows"))]
            {
                focus
            }
        } else {
            Task::none()
        };
        Task::batch([stabilize, rest])
    }

    /// CAMetalLayer 리사이즈 고정을 적용하고, 아직 레이어가 없으면 짧게 재시도한다.
    ///
    /// wgpu 서피스는 창 생성과 함께 만들어지므로 보통 첫 시도에 성공한다. 느린
    /// 환경에서 어댑터 프로빙이 늦어지는 경우를 대비해 몇 번만 더 두드린다.
    /// (소프트웨어 렌더러 폴백이면 끝내 실패하는데, 그때는 늘어날 레이어 자체가 없다)
    pub(super) fn handle_stabilize_surface_layer(&mut self, attempt: u8) -> Task<Message> {
        if !cfg!(target_os = "macos") || self.surface_layer_stabilized {
            return Task::none();
        }
        if crate::platform::stabilize_surface_layer() {
            self.surface_layer_stabilized = true;
            return Task::none();
        }
        if attempt >= MAX_SURFACE_STABILIZE_RETRIES {
            tracing::info!("CAMetalLayer를 찾지 못함 — 리사이즈 고정 생략 (소프트웨어 렌더러?)");
            return Task::none();
        }
        let next = attempt + 1;
        Task::future(async move {
            tokio::time::sleep(SURFACE_STABILIZE_RETRY).await;
            Message::StabilizeSurfaceLayer(next)
        })
    }

    pub(super) fn handle_ensure_focus(&mut self, attempt: u8) -> Task<Message> {
        self.log_ime_state("EnsureFocus:start");
        if attempt > MAX_FOCUS_RETRIES || !self.query.is_empty() {
            self.log_ime_state("EnsureFocus:skip");
            return Task::none();
        }

        if !self.window_focused {
            #[cfg(target_os = "windows")]
            {
                if let Some(raw_id) = self.raw_window_id {
                    if !crate::platform::is_our_window_foreground(raw_id) {
                        return Task::none();
                    }
                } else {
                    return Task::none();
                }
            }
            #[cfg(not(target_os = "windows"))]
            return Task::none();
        }

        let focus_task = self.request_focus();
        if attempt == MAX_FOCUS_RETRIES {
            focus_task
        } else {
            Task::batch([focus_task, Self::schedule_focus_retry(attempt + 1)])
        }
    }

    pub(super) fn handle_window_event(&mut self, event: window::Event) -> Task<Message> {
        match event {
            window::Event::Focused => {
                self.log_ime_state("WindowEvent:Focused");
                self.window_focused = true;
                // 창이 실제로 화면에 떠 포커스를 받은 시점 — 이제부터 이
                // 인스턴스가 토글(핫키로 닫기) 대상이 된다. 이 마커가 없으면
                // 콜드 스타트 중 재-핫키가 부팅 중인 창을 죽인다. (멱등)
                self._guard.mark_window_shown();
                // 부팅 안정화 구간이면 이미 initial_boot_tasks에서 포커스를 줬으므로 skip
                if self.is_boot_settled() {
                    self.log_ime_state("WindowEvent:Focused:skip_boot_settling");
                    return Task::none();
                }
                let typing_recently = self.last_query_changed_at.elapsed()
                    < Duration::from_millis(self.focused_typing_grace_ms());
                if !self.query.is_empty() && typing_recently {
                    self.log_ime_state("WindowEvent:Focused:skip_recent_typing");
                    return Task::none();
                }
                let focus_now = self.request_focus();
                if self.query.is_empty() {
                    #[cfg(target_os = "windows")]
                    {
                        let focus_retry = Self::schedule_focus_retry(1);
                        return Task::batch([focus_now, focus_retry]);
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        return focus_now;
                    }
                }
                focus_now
            }
            window::Event::Moved(point) => {
                self.window_state.x = Some(point.x);
                self.window_state.y = Some(point.y);
                self.state_dirty = true;
                Task::none()
            }
            window::Event::Resized(size) => {
                // 사용자 드래그 리사이즈 시 base width 저장
                let w = size.width.max(420.0);
                if (w - self.window_width).abs() > 1.0 {
                    self.window_width = w;
                    self.window_state.width = Some(w);
                    self.state_dirty = true;
                }
                // cloak 중이었다면 새 크기가 실제로 적용된 지금부터 한 프레임만 더
                // 기다렸다 되돌린다 — 하드 리밋(150ms)까지 기다릴 필요가 없다.
                if self.window_cloaked {
                    return self.schedule_uncloak(UNCLOAK_AFTER_RESIZE);
                }
                Task::none()
            }
            window::Event::Unfocused => {
                self.log_ime_state("WindowEvent:Unfocused");
                // 부팅 안정화 구간: macOS는 창 생성 직후 Unfocused를 보내므로 무시
                if self.is_boot_settled() {
                    self.log_ime_state("WindowEvent:Unfocused:ignored_boot_settling");
                    return Task::none();
                }
                if self.should_hold_unfocus_for_ime() {
                    self.log_ime_state("WindowEvent:Unfocused:ignored_ime_grace");
                    return Task::none();
                }
                self.window_focused = false;
                Task::future(async {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    Message::CheckUnfocusedExit
                })
            }
            _ => Task::none(),
        }
    }

    pub(super) fn handle_delayed_refocus(&mut self) -> Task<Message> {
        self.log_ime_state("DelayedRefocus:start");
        // IME 조합 중(최근 타이핑)에는 지연 refocus가 조합 상태를 깨뜨릴 수 있다.
        if !self.query.is_empty()
            && self.last_query_changed_at.elapsed() < Duration::from_millis(350)
        {
            self.log_ime_state("DelayedRefocus:skip_recent_typing");
            return Task::none();
        }
        self.focus_request_count += 1;
        iced::widget::operation::focus::<Message>(self.input_id.clone())
    }

    pub(super) fn handle_check_unfocused_exit(&mut self) -> Task<Message> {
        self.log_ime_state("CheckUnfocusedExit:start");
        if self.window_focused {
            return Task::none();
        }

        #[cfg(target_os = "macos")]
        if self.should_hold_unfocus_for_ime() {
            self.log_ime_state("CheckUnfocusedExit:retry_ime_grace");
            return Task::future(async {
                tokio::time::sleep(Duration::from_millis(250)).await;
                Message::CheckUnfocusedExit
            });
        }

        // Windows: GetForegroundWindow로 정확히 확인
        #[cfg(target_os = "windows")]
        if let Some(raw_id) = self.raw_window_id {
            if crate::platform::is_our_window_foreground(raw_id) {
                self.window_focused = true;
                let typing_recently =
                    self.last_query_changed_at.elapsed() < Duration::from_millis(700);
                if !self.query.is_empty() && typing_recently {
                    return Task::none();
                }
                let focus_now = self.request_focus();
                let focus_retry = Self::schedule_focus_retry(1);
                return Task::batch([focus_now, focus_retry]);
            }
        }
        // macOS/Linux: spurious unfocus는 이미 Unfocused 핸들러에서 차단했으므로
        // 여기까지 도달했다면 실제 포커스 유실이다.

        if self.state_dirty {
            self.window_state.save();
        }
        // 여기서만 exit_launcher()가 아니라 iced::exit()다 — 입력 소스를 되돌리지
        // 않는다. 포커스를 잃고 닫히는 경로라 이미 **다른 앱이 앞에 있고**, 그 상태에서
        // TIS로 한글을 고르면 메뉴바만 바뀌고 실제 입력은 영문으로 남는 반쪽 전환이
        // 생긴다(platform::restore_input_source 참조). 헷갈리는 반쪽보다 영문이 낫다.
        iced::exit()
    }
}
