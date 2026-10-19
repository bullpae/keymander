//! 창 높이 동기화 — 결과 유무에 따른 축소 디바운스와 Windows 리사이즈 cloak

use super::*;

/// 창 축소를 지연시키는 시간.
///
/// 결과가 사라질 때 창을 곧바로 접으면 두 가지 문제가 생긴다.
/// 1. 백스페이스 연타·한 줄 삭제(`macro:Home;Shift+End;Delete`)처럼 텍스트가
///    빠르게 줄어드는 동안 full → hint → collapsed 로 리사이즈가 연달아 난다.
/// 2. 리사이즈 직후 1~2프레임은 스왑체인에 이전(큰) 프레임이 남아 있어 DXGI가
///    그것을 새 창 크기로 stretch 합성한다 — 결과 목록처럼 정보가 빽빽한
///    화면이 압축돼 "화면이 찢어진" 것처럼 보인다.
///
/// 축소를 짧게 지연하면 연속 변화가 한 번으로 합쳐지고, 실제 리사이즈 시점에는
/// 이미 "빈 카드"(거의 단색)가 렌더돼 있어 stretch 왜곡이 눈에 띄지 않는다.
/// 확대는 결과가 즉시 보여야 하므로 지연하지 않는다.
/// Windows 키 오토리피트 간격(~31ms)보다 길게 잡아 연타를 흡수한다.
const SHRINK_DEBOUNCE: Duration = Duration::from_millis(60);

/// Windows: 리사이즈 cloak 을 풀기까지 기다리는 시간 (안전망 상한).
///
/// 정상 경로에서는 `window::Event::Resized` 를 받은 뒤 [`UNCLOAK_AFTER_RESIZE`] 만
/// 기다리므로 실제 cloak 시간은 한 프레임 남짓이다. 이 상수는 Resized 가 오지
/// 않는 예외 상황에서도 창이 영영 숨지 않게 하는 하드 리밋이다.
pub(super) const CLOAK_HARD_LIMIT: Duration = Duration::from_millis(150);

/// Windows: Resized 수신 후 cloak 을 푸는 지연 — 새 크기 프레임이 present 될 시간.
pub(super) const UNCLOAK_AFTER_RESIZE: Duration = Duration::from_millis(16);

/// cloak 을 걸 만한 축소 비율 하한.
///
/// 왜곡 크기는 절대 픽셀이 아니라 축소 "배율"에 비례한다 — 10배로 접히면 결과 목록이
/// 10배로 눌린 프레임이 보인다. 1.25배 미만(예: 창을 몇 px 다듬는 정도)은 눈에
/// 띄지 않으므로 굳이 창을 숨겨 깜빡이게 만들지 않는다.
const CLOAK_MIN_RATIO: f32 = 1.25;

impl App {
    // ─── Window height sync ───────────────────────────────────────────────

    /// 현재 상태에 맞는 창 높이.
    ///
    /// 결과가 없을 때 창을 pill 높이로 접는 이유: 빈 영역을 투명 픽셀로 채우는
    /// 방식은 창 투명 합성이 안 되는 환경(VM 가상 GPU, 소프트웨어 렌더러 폴백 등
    /// — 특히 Windows on ARM VM)에서 거대한 검은 사각형으로 렌더링된다.
    /// 창 자체를 콘텐츠 크기로 유지하면 렌더러와 무관하게 문제가 없다.
    fn desired_window_height(&self) -> f32 {
        let u = &self.ui;
        // 높이 고정 플랫폼: 창은 그대로 두고 카드만 콘텐츠 크기로 바뀐다.
        // (view 는 이미 결과가 없으면 card_stack=Shrink + 투명 bg_dismiss=Fill 로
        //  그리므로, 빈 영역은 창이 접힌 것과 똑같이 보인다)
        if self.fixed_window_height {
            return u.full_window_height;
        }
        if !self.results.is_empty() {
            u.full_window_height
        } else if !self.query.trim().is_empty() {
            u.collapsed_window_height + u.hint_area_height
        } else {
            u.collapsed_window_height
        }
    }

    /// 창 높이가 상태와 어긋나면 리사이즈 태스크를 반환 (일치하면 no-op).
    ///
    /// 확대는 즉시, 축소는 [`SHRINK_DEBOUNCE`] 뒤에 적용한다 — 연속 축소를
    /// 한 번으로 합치고 리사이즈 순간의 stretch 왜곡을 감춘다.
    pub(super) fn sync_window_height(&mut self) -> Task<Message> {
        let desired = self.desired_window_height();
        if (desired - self.window_height).abs() < 0.5 {
            return Task::none();
        }

        // 확대: 즉시 적용하고 대기 중이던 축소는 무효화한다.
        if desired > self.window_height {
            self.shrink_token = self.shrink_token.wrapping_add(1);
            self.pending_shrink_to = None;
            self.window_height = desired;
            return self.resize_window_to(desired);
        }

        // 이미 같은 목표로 예약돼 있으면 타이머를 살려 둔다 — 재예약하면
        // 무관한 메시지가 올 때마다 타이머가 리셋돼 축소가 영영 적용되지 않는다.
        if self
            .pending_shrink_to
            .is_some_and(|h| (h - desired).abs() < 0.5)
        {
            return Task::none();
        }

        // 축소: 토큰을 갱신해 이전 대기 요청을 무효화하고 지연 적용한다.
        // window_height 는 실제 적용 시점에 갱신해야 그 사이의 확대를
        // "확대"로 정확히 판정할 수 있다.
        self.shrink_token = self.shrink_token.wrapping_add(1);
        self.pending_shrink_to = Some(desired);
        let token = self.shrink_token;
        Task::future(async move {
            tokio::time::sleep(SHRINK_DEBOUNCE).await;
            Message::ApplyPendingShrink(token)
        })
    }

    /// 디바운스된 축소 적용 — 최신 토큰이고 여전히 축소가 필요할 때만 리사이즈.
    pub(super) fn apply_pending_shrink(&mut self, token: u64) -> Task<Message> {
        if token != self.shrink_token {
            return Task::none(); // 더 최근 요청(또는 확대)이 이 요청을 대체함
        }
        self.pending_shrink_to = None;
        let desired = self.desired_window_height();
        // 대기 중 상태가 바뀌어 축소가 필요 없어졌으면 무시한다.
        if desired >= self.window_height - 0.5 {
            return Task::none();
        }
        let needs_cloak = Self::should_cloak_shrink(self.window_height, desired);
        self.window_height = desired;
        // 눈에 띌 만한 축소는 stretch 왜곡이 그대로 드러나므로 Windows 에서만
        // cloak 으로 가린다. (macOS 는 CAMetalLayer gravity 교정으로 늘어나지 않는다)
        let cloak = if needs_cloak {
            self.cloak_for_resize()
        } else {
            Task::none()
        };
        Task::batch([cloak, self.resize_window_to(desired)])
    }

    // ─── Windows 리사이즈 cloak ───────────────────────────────────────────

    /// 이 축소가 cloak 으로 가릴 만한지 — 플랫폼 호출과 분리한 순수 판정.
    pub(super) fn should_cloak_shrink(from: f32, to: f32) -> bool {
        to > 0.0 && from / to >= CLOAK_MIN_RATIO
    }

    /// 리사이즈 직전에 창을 DWM 합성에서 빼고, 해제를 예약한다.
    ///
    /// Windows 외에서는 no-op — macOS/Linux 는 컴포지터 쪽 교정으로 해결한다.
    fn cloak_for_resize(&mut self) -> Task<Message> {
        if !cfg!(target_os = "windows") {
            return Task::none();
        }
        let Some(raw_id) = self.raw_window_id else {
            return Task::none();
        };
        if !self.window_cloaked {
            if !crate::platform::set_window_cloaked(raw_id, true) {
                return Task::none(); // DWM 미지원 환경 — 그대로 진행
            }
            self.window_cloaked = true;
            self.cloaked_at = Some(std::time::Instant::now());
        }
        self.schedule_uncloak(CLOAK_HARD_LIMIT)
    }

    /// cloak 해제를 `delay` 뒤로 예약한다 (이전 예약은 토큰으로 무효화).
    pub(super) fn schedule_uncloak(&mut self, delay: Duration) -> Task<Message> {
        self.cloak_token = self.cloak_token.wrapping_add(1);
        let token = self.cloak_token;
        Task::future(async move {
            tokio::time::sleep(delay).await;
            Message::UncloakWindow(token)
        })
    }

    /// 예약된 cloak 해제 — 최신 토큰일 때만 적용.
    pub(super) fn apply_uncloak(&mut self, token: u64) -> Task<Message> {
        if token != self.cloak_token {
            return Task::none();
        }
        self.uncloak_now();
        Task::none()
    }

    /// 즉시 cloak 해제 (idempotent). 예약이 유실돼도 창이 숨은 채 남지 않게 한다.
    fn uncloak_now(&mut self) {
        if !self.window_cloaked {
            return;
        }
        self.window_cloaked = false;
        self.cloaked_at = None;
        if let Some(raw_id) = self.raw_window_id {
            let _ = crate::platform::set_window_cloaked(raw_id, false);
        }
    }

    /// 안전망: 어떤 이유로든 해제 메시지가 유실돼 cloak 이 하드 리밋을 넘겼으면
    /// 다음 메시지 처리에서 무조건 되돌린다.
    pub(super) fn enforce_cloak_deadline(&mut self) {
        if let Some(started) = self.cloaked_at {
            if started.elapsed() > CLOAK_HARD_LIMIT * 2 {
                tracing::warn!("cloak 해제 예약 유실 — 강제 해제");
                self.uncloak_now();
            }
        }
    }

    /// 창을 주어진 높이로 리사이즈 (폭은 현재 값 유지).
    fn resize_window_to(&self, height: f32) -> Task<Message> {
        // window::Settings 의 min/max 폭과 동일한 범위로 클램프
        let width = self.window_width.clamp(420.0, 1600.0);
        match self.window_id {
            Some(id) => window::resize(id, Size::new(width, height)),
            None => window::oldest().then(move |maybe_id| match maybe_id {
                Some(id) => window::resize(id, Size::new(width, height)),
                None => Task::none(),
            }),
        }
    }
}
