//! Application state, messages, update/view/subscription — the Elm core.
//!
//! Renders a Spotlight-like floating launcher: search bar always visible,
//! results list + status bar appear only when there are results.
//!
//! **Key features**:
//! - Async engine loading — window appears instantly
//! - Singleton toggle via `kmd_core::single_instance::Guard`
//! - Window position/width persisted between sessions
//! - Resizable width with min/max constraints

use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use iced::keyboard;
use iced::widget::operation::scroll_to;
use iced::widget::scrollable as scrollable_mod;
use iced::widget::text::Wrapping;
use iced::widget::{
    container, image, mouse_area, row, scrollable, stack, text, text_input, Column, Space,
};
use iced::{
    window, Background, Border, Color, Element, Fill, Length, Padding, Point, Size, Subscription,
    Task,
};

use kmd_core::plugin::{builtin_calc, builtin_shell};
use kmd_core::single_instance::Guard;
use kmd_core::web;
use kmd_core::{IndexItem, ItemKind, Source};

use crate::theme::DesktopTheme;
use crate::window_state::WindowState;

mod context_action;
mod engine_boot;
mod focus_flow;
mod launch;
mod results;
mod scope;
mod search_routing;
mod settings;
mod ui_scale;
mod view;
mod window_resize;

use context_action::context_actions_for;
pub use context_action::ContextAction;
use engine_boot::{EngineSlot, QuickEngineSlot};
use results::{estimate_title_max_chars, items_to_results, relaxed_hangul_query, truncate_str};
pub use ui_scale::{collapsed_window_height, full_window_height, initial_window_height, UiScale};
use window_resize::UNCLOAK_AFTER_RESIZE;

// ─── Constants ────────────────────────────────────────────────────────────────

pub const DEFAULT_WIDTH: f32 = 1000.0;
const SEARCH_LIMIT: usize = 50;
/// 빈 입력일 때 보여줄 최근 실행 항목 수 (TUI와 동일).
const RECENT_DISPLAY_LIMIT: usize = 20;
const SCORE_PLUGIN: u32 = u32::MAX;

/// 카드 외곽 래퍼 padding — 바깥 링(peach)과 창 가장자리 사이 간격.
/// Windows 불투명 창에서는 이 간격이 창 배경색 띠로 노출되는 데다 DPI 소수
/// 배율에서 우측이 서브픽셀로 잘려 상하좌우 마진이 비대칭으로 보이므로 0
/// (teal 테두리가 창 가장자리에 밀착). macOS 투명 pill은 이중 테두리 유지.
const CARD_PAD: f32 = if cfg!(target_os = "windows") {
    0.0
} else {
    2.0
};

/// 창 높이를 결과 유무에 따라 바꾸지 않고 full 로 고정할지.
///
/// v0.10.1(e34e241)에서 창 접기를 도입한 이유는 오직 하나 — 빈 영역을 투명
/// 픽셀로 채우면 **투명 합성이 안 되는 환경**(Windows on ARM VM 등 wgpu alpha
/// mode 가 Opaque 로 폴백)에서 거대한 검은 사각형이 되기 때문이었다. 그런데
/// 창 리사이즈는 그 자체로 컴포지터가 이전 프레임을 새 크기로 늘려 합성하는
/// 구간을 만든다 — "결과창이 뜨고 사라질 때 화면이 깨진다"는 증상의 원인이다.
///
/// macOS 는 Quartz 가 알파를 항상 정확히 합성하므로 검은 사각형 문제가 없다.
/// (Windows 는 v0.10.2 부터 아예 불투명 창이라 창을 접어야 큰 색 덩어리가 안
/// 남는다 — 그쪽은 리사이즈 경로를 유지한다.)
/// 그래서 macOS 만 v0.10.1 이전처럼 높이를 고정하고, 카드만 콘텐츠 크기로
/// 늘었다 줄었다 한다. 리사이즈가 없으면 늘어날 이전 프레임도 없다.
const FIXED_WINDOW_HEIGHT: bool = cfg!(target_os = "macos");

/// macOS 서피스 레이어 고정 재시도 간격/횟수 — 렌더러 초기화가 늦는 환경 대비.
const SURFACE_STABILIZE_RETRY: Duration = Duration::from_millis(150);
const MAX_SURFACE_STABILIZE_RETRIES: u8 = 4;

const QUIT_POLL_MS: u64 = 300;
const WARMUP_IDLE_MS: u64 = 400;
const FOCUS_RETRY_MS: u64 = 120;
const MAX_FOCUS_RETRIES: u8 = 3;
const AUTOSTART_STATUS_REFRESH_MS: u64 = 1500;

// ─── App State ────────────────────────────────────────────────────────────────

pub struct App {
    query: String,
    results: Vec<kmd_core::SearchResult>,
    search_mode: kmd_core::SearchMode,
    selected: usize,
    engine: kmd_core::SearchEngine,
    theme: DesktopTheme,
    ui: UiScale,
    input_id: iced::widget::Id,
    scrollable_id: iced::widget::Id,
    window_id: Option<window::Id>,
    raw_window_id: Option<u64>,
    use_emoji: bool,
    daemon_autostart_enabled: Option<bool>,
    daemon_autostart_last_checked_at: Option<std::time::Instant>,
    daemon_autostart_check_in_flight: bool,
    daemon_autostart_toggle_in_flight: bool,
    runtime_config: kmd_core::Config,
    last_results_signature: u64,
    /// 비동기 검색 응답 세대 — perform_search마다 증가한다. 쿼리가 바뀐 뒤
    /// 도착한 오래된(느린) 데몬 응답이 현재 결과를 덮어쓰지 않게 하는 가드.
    search_generation: u64,
    loading: bool,
    engine_slot: EngineSlot,
    quick_engine_slot: QuickEngineSlot,
    /// full 인덱스 엔진이 이미 적용되었는지 — 늦게 도착한 quick 엔진이
    /// full 엔진을 덮어쓰지 않도록 가드
    full_engine_loaded: bool,
    /// 실행 이력 frecency 맵 — 부팅 시 1회 로드해 키 입력마다의 DB 조회를 제거
    frecency: kmd_core::history::FrecencyMap,
    db: kmd_core::db::Database,
    /// 본문 검색용 공유 DB (`<data_dir>/kmd.db` — 데몬이 인덱싱, 여기선 읽기만).
    /// 위의 `db`(desktop 전용 히스토리 DB)와 다른 파일이다. `?` 첫 사용 시 지연 오픈.
    content_db: Option<kmd_core::db::Database>,
    _guard: Guard,

    // ── Window geometry ───────────────────────────────────────────────
    window_width: f32,
    /// 현재 적용된(또는 적용 요청한) 창 높이 — 결과 유무에 따라 collapsed↔full 전환
    window_height: f32,
    /// 축소 디바운스 토큰 — 값이 바뀌면 대기 중이던 축소 요청은 무효가 된다.
    /// (확대가 끼어들거나 더 최근의 축소 요청이 들어온 경우)
    shrink_token: u64,
    /// 대기 중인 축소 목표 높이. `sync_window_height` 는 매 메시지마다 호출되므로
    /// 이 값이 없으면 축소가 적용되기 전에 도착한 포커스·타이머 메시지마다
    /// 예약이 재발행되어 타이머가 끝없이 리셋된다(창이 영영 접히지 않음).
    pending_shrink_to: Option<f32>,
    /// Windows 전용 — 리사이즈 잔상을 가리려고 DWM 합성에서 창을 뺀 상태인지.
    window_cloaked: bool,
    /// cloak 해제 예약 토큰 — 가장 최근 예약만 유효.
    cloak_token: u64,
    /// cloak 시작 시각 — 예약이 유실돼도 다음 메시지에서 강제 해제하는 안전망.
    cloaked_at: Option<std::time::Instant>,
    /// macOS CAMetalLayer 리사이즈 고정 적용 완료 여부 (재시도 중단 조건).
    surface_layer_stabilized: bool,
    /// 창 높이를 결과 유무와 무관하게 고정할지 ([`FIXED_WINDOW_HEIGHT`]).
    /// 테스트에서 리사이즈 경로를 검증하려고 뒤집을 수 있도록 필드로 둔다.
    fixed_window_height: bool,
    window_state: WindowState,
    state_dirty: bool,
    window_focused: bool,
    last_query_changed_at: std::time::Instant,
    app_started_at: std::time::Instant,
    ime_trace_seq: u64,
    ime_trace_enabled: bool,

    // ── Focus tracking ─────────────────────────────────────────────────
    /// 디버그/테스트용: focus 요청 총 횟수
    focus_request_count: u32,
    /// 부팅 직후 포커스 폭주를 방지 — 첫 포커스 이후 안정화 기간에는 재포커싱 차단
    boot_focus_done: bool,

    // ── IME ───────────────────────────────────────────────────────────
    reset_ime_on_launch: bool,
    /// 상태바에 잠깐 띄울 알림 (저장 실패 등). 다음 검색에서 지워진다.
    /// 이벤트는 검색 결과가 아니므로 결과 목록에 끼워 넣지 않는다.
    status_message: Option<String>,
    /// `:scope`에서 방금 뺀 폴더 — Ctrl+Z로 되돌린다 (docs/18)
    scope_undo: Option<std::path::PathBuf>,
    /// 범위 변경 후 엔진 교체가 끝나면 상태줄에 남길 결과
    scope_report: Option<scope::ScopeReport>,

    // ── Startup warmup ────────────────────────────────────────────────
    full_warmup_started: bool,
    warmup_token: u64,
}

// ─── Messages ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Message {
    QueryChanged(String),
    ResultClicked(usize),
    KeyEvent(keyboard::Key, keyboard::Modifiers),
    BrandClicked,
    BrandRightClicked,
    StartWindowDrag,
    StartWindowResize(window::Direction),
    GotWindowId(Option<window::Id>),
    GotRawWindowId(u64),
    WarmupTick(u64),
    EnsureFocus(u8),
    EngineReady,
    CheckQuitSignal,
    WindowEvent(window::Id, window::Event),
    ShellDone(Result<String, String>),
    /// 상세 패널의 컨텍스트 액션 실행
    RunAction(ContextAction),
    /// 포커스 잃은 후 debounce 확인 → 여전히 포커스 없으면 종료
    CheckUnfocusedExit,
    /// 투명 배경 클릭 → Spotlight처럼 앱 닫기
    BackgroundClicked,
    /// 디바운스된 창 축소 적용 — 토큰이 최신일 때만 유효 (SHRINK_DEBOUNCE 참고)
    ApplyPendingShrink(u64),
    /// Windows 리사이즈 cloak 해제 — 토큰이 최신일 때만 유효
    UncloakWindow(u64),
    /// macOS CAMetalLayer 리사이즈 고정 재시도 (레이어 생성이 늦을 때)
    StabilizeSurfaceLayer(u8),
    /// view 재렌더 후 1프레임 뒤 포커스 재요청 (트리 변경 후 포커스 유실 복구).
    // 생성처가 cfg(windows) 뒤라 macOS 빌드에서만 미생성으로 보인다 — 삭제 금지.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    DelayedRefocus,
    /// 백그라운드 아이콘 프리워밍 완료 → 캐시된 아이콘으로 리렌더 유도
    IconsReady,
    /// 데몬 자동시작 상태 조회 완료
    AutostartStatusLoaded(Result<bool, String>),
    /// 데몬 자동시작 토글 완료
    AutostartToggleFinished(Result<String, String>),
    /// 엔진 교체(EngineReady) 후 IME 조합이 끝나면 검색 결과를 갱신
    EngineSwapSettled,
    /// 비동기 quick 엔진 로드 완료 → 빈 엔진과 교체
    QuickEngineReady,
    /// 비동기 클립보드 히스토리 검색 완료 — generation이 현재 세대와 다르면 폐기
    ClipSearchFinished {
        generation: u64,
        result: Result<Vec<kmd_core::ipc::ClipHit>, String>,
    },
    /// 클립보드 붙여넣기/복사 IPC 완료 — 성공할 때만 런처를 닫는다 (정직한 오류)
    ClipActionFinished(Result<String, String>),
    /// `:scope` 추가·제거 후 리빌드 완료 → 새 캐시로 엔진 교체 (docs/18)
    ScopeReindexed(Result<String, String>),
}

// ─── Boot ─────────────────────────────────────────────────────────────────────

impl App {
    fn log_ime_state(&mut self, label: &str) {
        if !self.ime_trace_enabled {
            return;
        }
        self.ime_trace_seq = self.ime_trace_seq.wrapping_add(1);
        tracing::debug!(
            seq = self.ime_trace_seq,
            elapsed_ms = self.app_started_at.elapsed().as_millis() as u64,
            since_input_ms = self.last_query_changed_at.elapsed().as_millis() as u64,
            window_focused = self.window_focused,
            raw_window_id = ?self.raw_window_id,
            query = %self.query,
            query_len = self.query.chars().count(),
            jamo_tail = self.should_limit_reorder_for_ime(),
            "{label}"
        );
    }

    pub fn new(
        guard: Guard,
        config: kmd_core::Config,
        window_state: WindowState,
    ) -> (Self, Task<Message>) {
        Self::new_with_db_path(guard, config, window_state, None)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        guard: Guard,
        config: kmd_core::Config,
        window_state: WindowState,
        db_path: std::path::PathBuf,
    ) -> (Self, Task<Message>) {
        Self::new_with_db_path(guard, config, window_state, Some(db_path))
    }

    fn new_with_db_path(
        guard: Guard,
        config: kmd_core::Config,
        window_state: WindowState,
        db_path_override: Option<std::path::PathBuf>,
    ) -> (Self, Task<Message>) {
        // 3단계 부팅: 빈 엔진으로 즉시 창 표시 → quick 인덱스 비동기 로드 →
        // 유휴 시점에 full 인덱스 워밍업 (spawn_full_engine_load_task)
        tracing::info!("Booting with empty engine — quick index loads asynchronously");
        let engine = kmd_core::SearchEngine::new();
        let db_path =
            db_path_override.unwrap_or_else(|| kmd_core::db::DbRole::Desktop.default_path());
        let db = match kmd_core::db::Database::open(&db_path) {
            Ok(db) => db,
            Err(e) => {
                // 파일 DB 열기 실패 시 in-memory로 폴백하되 경고 기록.
                // frecency 히스토리는 이 세션에서만 유효하고 영구 저장되지 않는다.
                tracing::warn!(
                    "DB 파일 열기 실패 ({e}) — in-memory로 폴백, 히스토리가 저장되지 않습니다: {}",
                    db_path.display()
                );
                kmd_core::db::Database::open_in_memory().expect("in-memory DB 초기화 실패")
            }
        };
        let theme = crate::theme::from_name(&config.general.theme);
        let use_emoji = config.general.emoji_icons;
        let reset_ime = config.general.reset_ime_on_launch;
        let ui = UiScale::new(config.general.font_size, config.general.visible_rows);
        let window_width = window_state.width.unwrap_or(DEFAULT_WIDTH);

        let input_id = iced::widget::Id::unique();
        let scrollable_id = iced::widget::Id::unique();
        let engine_slot: EngineSlot = Arc::new(Mutex::new(None));
        let quick_engine_slot: QuickEngineSlot = Arc::new(Mutex::new(None));
        // 검색 핫패스(키 입력마다)에서 DB를 조회하지 않도록 부팅 시 1회 로드
        let frecency = kmd_core::history::load_boost_map(&db);
        let config_for_quick = config.clone();

        let app = Self {
            query: String::new(),
            results: Vec::new(),
            search_mode: kmd_core::SearchMode::Fuzzy,
            selected: 0,
            engine,
            theme,
            ui,
            input_id: input_id.clone(),
            scrollable_id,
            window_id: None,
            raw_window_id: None,
            use_emoji,
            daemon_autostart_enabled: None,
            daemon_autostart_last_checked_at: None,
            daemon_autostart_check_in_flight: false,
            daemon_autostart_toggle_in_flight: false,
            runtime_config: config,
            last_results_signature: 0,
            search_generation: 0,
            loading: true,
            engine_slot,
            quick_engine_slot: quick_engine_slot.clone(),
            full_engine_loaded: false,
            frecency,
            db,
            content_db: None,
            _guard: guard,
            window_width,
            window_height: if FIXED_WINDOW_HEIGHT || crate::window_transparent() {
                ui.full_window_height
            } else {
                ui.collapsed_window_height
            },
            shrink_token: 0,
            pending_shrink_to: None,
            window_cloaked: false,
            cloak_token: 0,
            cloaked_at: None,
            surface_layer_stabilized: false,
            // [실험] Windows에서 투명이 되면 macOS처럼 높이를 고정할 수 있다.
            fixed_window_height: FIXED_WINDOW_HEIGHT || crate::window_transparent(),
            window_state,
            state_dirty: false,
            window_focused: true,
            last_query_changed_at: std::time::Instant::now(),
            app_started_at: std::time::Instant::now(),
            ime_trace_seq: 0,
            ime_trace_enabled: std::env::var_os("KMD_IME_TRACE").is_some(),
            focus_request_count: 0,
            boot_focus_done: false,
            reset_ime_on_launch: reset_ime,
            status_message: None,
            scope_undo: None,
            scope_report: None,
            full_warmup_started: false,
            warmup_token: 0,
        };

        let quick_task = Self::spawn_quick_engine_load_task(quick_engine_slot, config_for_quick);
        let boot_tasks = Self::initial_boot_tasks(input_id, false);
        (app, Task::batch([quick_task, boot_tasks]))
    }

    // ─── Update ───────────────────────────────────────────────────────────

    /// 포커스 가드: UI 상태(query/results)가 변경되면 자동으로 포커스를 복원한다.
    /// 개별 핸들러가 request_focus()를 빠뜨려도 안전하게 보장하는 최후의 안전망.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        let skip_focus_guard = matches!(
            message,
            Message::QueryChanged(_)
                | Message::DelayedRefocus
                | Message::ApplyPendingShrink(_)
                | Message::UncloakWindow(_)
                | Message::StabilizeSurfaceLayer(_)
                | Message::EnsureFocus(_)
                | Message::EngineReady
                | Message::EngineSwapSettled
                | Message::QuickEngineReady
                | Message::IconsReady
                | Message::AutostartStatusLoaded(_)
                | Message::AutostartToggleFinished(_)
                | Message::ScopeReindexed(_)
        );

        let old_query_len = self.query.len();
        let old_results_len = self.results.len();

        // cloak 은 어떤 경로로든 반드시 풀려야 한다 — 매 메시지에서 기한을 확인.
        self.enforce_cloak_deadline();

        let inner_task = self.update_inner(message);
        // 결과 유무가 바뀌면 창 높이를 동기화 — 어떤 메시지 경로에서 바뀌었든 보장
        let task = Task::batch([inner_task, self.sync_window_height()]);

        if skip_focus_guard || !self.window_focused || self.is_boot_settled() {
            return task;
        }

        let query_changed = self.query.len() != old_query_len;
        let results_changed = self.results.len() != old_results_len;

        if query_changed || results_changed {
            let focus = self.request_focus();
            Task::batch([task, focus])
        } else {
            task
        }
    }

    fn handle_query_changed(&mut self, query: String) -> Task<Message> {
        self.query = query;
        // macOS: 런처 프로세스 기동 직후 TSM이 한글 입력기를 연결하기 전에
        // 입력된 키는 조합 없이 낱자 자모로 커밋된다(ㅎㅏㄴ). IME가 조합했을
        // 결과(한)로 재조합해 표시/검색을 복원한다. 정상 텍스트는 항등(None).
        if let Some(recomposed) = kmd_core::hangul::recompose_jamo(&self.query) {
            self.log_ime_state("QueryChanged:recompose_jamo");
            self.query = recomposed;
        }
        self.selected = 0;
        self.last_query_changed_at = std::time::Instant::now();
        self.log_ime_state("QueryChanged:assigned");
        // 한글 IME 조합 중에는 검색/리렌더를 지연해 자모 분리 가능성을 줄인다.
        if self.should_limit_reorder_for_ime() {
            self.log_ime_state("QueryChanged:skip_search_due_to_jamo");
            // 조합 중에도 warmup 유휴 타이머를 리셋한다. 그렇지 않으면 부팅 시
            // 예약된 WarmupTick이 첫 조합 도중 발화해 loading 토글 재렌더로
            // marked text가 깨지고 자모가 분리된다. (macOS)
            return self.with_activity_warmup(Task::none());
        }
        let search_task = self.perform_search();
        self.log_ime_state("QueryChanged:perform_search");
        self.with_activity_warmup(search_task)
    }

    fn handle_got_window_id(&mut self, id: Option<window::Id>) -> Task<Message> {
        self.window_id = id;
        crate::launch_perf::mark_window();
        tracing::info!(
            "Window id acquired {} ms after boot",
            self.app_started_at.elapsed().as_millis()
        );
        match id {
            Some(id) => {
                let saved_x = self.window_state.x;
                let saved_y = self.window_state.y;
                let width = self.window_width;
                let win_h = self.ui.full_window_height;
                let ensure_visible = window::monitor_size(id).then(move |maybe_size| {
                    let (Some(x), Some(y), Some(mon)) = (saved_x, saved_y, maybe_size) else {
                        return Task::none();
                    };

                    let w = width.clamp(420.0, 1200.0);
                    let h = win_h;
                    let outside = x + w < 40.0
                        || x > mon.width - 40.0
                        || y + h < 20.0
                        || y > mon.height - 20.0;

                    if outside {
                        let recentered =
                            Point::new((mon.width - w) / 2.0, (mon.height / 3.0).max(0.0));
                        window::move_to(id, recentered)
                    } else {
                        Task::none()
                    }
                });

                Task::batch([
                    window::raw_id::<Message>(id).map(Message::GotRawWindowId),
                    ensure_visible,
                ])
            }
            None => self.request_focus(),
        }
    }

    fn handle_autostart_status_loaded(&mut self, result: Result<bool, String>) -> Task<Message> {
        self.daemon_autostart_check_in_flight = false;
        self.daemon_autostart_last_checked_at = Some(std::time::Instant::now());
        match result {
            Ok(installed) => {
                self.daemon_autostart_enabled = Some(installed);
            }
            Err(message) => {
                tracing::warn!("autostart status 조회 실패: {message}");
                self.daemon_autostart_enabled = None;
            }
        }
        if crate::query_prefix::prefix_of(self.query.trim())
            == crate::query_prefix::Prefix::Settings
        {
            let current_query = self.query.clone();
            self.handle_settings_query(&current_query);
        }
        Task::none()
    }

    fn handle_autostart_toggle_finished(
        &mut self,
        result: Result<String, String>,
    ) -> Task<Message> {
        self.daemon_autostart_toggle_in_flight = false;
        match result {
            Ok(message) => tracing::info!("autostart: {message}"),
            Err(message) => tracing::warn!("autostart toggle 실패: {message}"),
        }
        self.query = ":set".to_string();
        self.handle_settings_query(":set");
        let refresh = self.schedule_autostart_status_refresh(true);
        let focus = self.request_focus();
        Task::batch([refresh, focus])
    }

    fn handle_check_quit_signal(&mut self) -> Task<Message> {
        if self.state_dirty {
            self.window_state.save();
            self.state_dirty = false;
        }
        if self._guard.should_quit() {
            self._guard.consume_quit_signal();
            tracing::info!("Received quit signal — exiting");
            self.window_state.save();
            return exit_launcher();
        }
        Task::none()
    }

    /// Quick Action 완료 콜백.
    ///
    /// 실패하면 창을 닫지 않는다 — 닫아버리면 사용자는 명령이 성공한 줄 안다.
    /// 예전에는 실행 실패도 복사 실패도 로그만 남기고 무조건 종료해서, 종료
    /// 코드 검증을 추가해도 GUI에서는 오류를 볼 수 없었다.
    fn handle_shell_done(&mut self, result: Result<String, String>) -> Task<Message> {
        match result {
            Ok(output) => match kmd_core::clipboard::set_text(&output) {
                Ok(()) => {
                    let first_line = output.lines().next().unwrap_or("(no output)");
                    tracing::info!("Shell output copied: {first_line}");
                    exit_launcher()
                }
                Err(e) => {
                    tracing::warn!("클립보드 쓰기 실패: {e}");
                    self.status_message = Some(format!("실행은 됐으나 복사 실패: {e}"));
                    Task::none()
                }
            },
            Err(msg) => {
                tracing::error!("Shell error: {msg}");
                self.status_message = Some(msg);
                Task::none()
            }
        }
    }

    fn handle_clip_search_finished(
        &mut self,
        generation: u64,
        result: Result<Vec<kmd_core::ipc::ClipHit>, String>,
    ) -> Task<Message> {
        // 오래된 응답(그 사이 쿼리가 바뀜)이나 클립보드 모드를 벗어난 뒤
        // 도착한 응답은 폐기한다 — 현재 결과를 덮어쓰지 않는다.
        if generation != self.search_generation
            || crate::query_prefix::prefix_of(self.query.trim())
                != crate::query_prefix::Prefix::Clipboard
        {
            return Task::none();
        }
        let use_emoji = self.use_emoji;
        let query_is_empty = self
            .query
            .trim()
            .strip_prefix(';')
            .or_else(|| self.query.trim().strip_prefix(":clip"))
            .unwrap_or("")
            .trim()
            .is_empty();
        let items = match result {
            Ok(hits) if !hits.is_empty() => hits
                .iter()
                .map(|hit| search_routing::clip_item(hit, use_emoji))
                .collect(),
            Ok(_) => vec![search_routing::clip_notice(
                if query_is_empty {
                    "클립보드 히스토리가 비어 있습니다"
                } else {
                    "일치하는 항목이 없습니다"
                },
                "복사를 하면 여기에 쌓입니다 (설정: [clipboard] history_enabled)",
                use_emoji,
            )],
            Err(message) => vec![search_routing::clip_notice(
                "클립보드 히스토리를 사용할 수 없습니다",
                &message,
                use_emoji,
            )],
        };
        self.apply_contains_items(items);
        self.spawn_icon_prefetch()
    }

    fn update_inner(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::QueryChanged(query) => self.handle_query_changed(query),
            Message::ResultClicked(index) => {
                self.selected = index;
                self.launch_selected()
            }
            Message::KeyEvent(key, modifiers) => {
                // Enter는 수정자까지 봐야 한다 (클립보드 결과의 Cmd/Ctrl+Enter
                // 복사 전용) — text_input on_submit 대신 여기서 일괄 처리한다.
                if matches!(key, keyboard::Key::Named(keyboard::key::Named::Enter)) {
                    return self.handle_submit(&modifiers);
                }
                if let Some(task) = self.handle_scope_key(&key, &modifiers) {
                    return task;
                }
                if let Some(action) = self.match_shortcut(&key, &modifiers) {
                    return self.execute_context_action(action);
                }
                let key_task = self.handle_key(key);
                self.with_activity_warmup(key_task)
            }
            Message::BrandClicked => self.toggle_query_mode(":help"),
            Message::BrandRightClicked => self.toggle_query_mode(":set"),
            Message::StartWindowDrag => match self.window_id {
                Some(id) => window::drag(id),
                None => window::oldest().then(|maybe_id| match maybe_id {
                    Some(id) => window::drag(id),
                    None => Task::none(),
                }),
            },
            Message::StartWindowResize(direction) => match self.window_id {
                Some(id) => window::drag_resize(id, direction),
                None => window::oldest().then(move |maybe_id| match maybe_id {
                    Some(id) => window::drag_resize(id, direction),
                    None => Task::none(),
                }),
            },
            Message::GotWindowId(id) => self.handle_got_window_id(id),
            Message::GotRawWindowId(raw_id) => self.handle_got_raw_window_id(raw_id),
            Message::WarmupTick(token) => self.handle_warmup_tick(token),
            Message::EnsureFocus(attempt) => self.handle_ensure_focus(attempt),
            Message::EngineReady => self.handle_engine_ready(),
            Message::EngineSwapSettled => self.handle_engine_swap_settled(),
            Message::QuickEngineReady => self.handle_quick_engine_ready(),
            Message::IconsReady => Task::none(),
            Message::AutostartStatusLoaded(result) => self.handle_autostart_status_loaded(result),
            Message::AutostartToggleFinished(result) => {
                self.handle_autostart_toggle_finished(result)
            }
            Message::CheckQuitSignal => self.handle_check_quit_signal(),
            Message::WindowEvent(_id, event) => self.handle_window_event(event),
            Message::ShellDone(result) => self.handle_shell_done(result),
            Message::ClipSearchFinished { generation, result } => {
                self.handle_clip_search_finished(generation, result)
            }
            Message::ScopeReindexed(result) => self.handle_scope_reindexed(result),
            Message::ClipActionFinished(result) => match result {
                Ok(message) => {
                    tracing::info!("{message}");
                    exit_launcher()
                }
                Err(message) => {
                    // 실패는 결과 리스트에 표시해 사용자가 원인(항목 만료·데몬
                    // 중지 등)을 보고 다시 시도할 수 있게 한다 — 창을 닫지 않는다.
                    tracing::warn!("클립보드 동작 실패: {message}");
                    self.apply_contains_items(vec![search_routing::clip_notice(
                        "클립보드 동작에 실패했습니다",
                        &message,
                        self.use_emoji,
                    )]);
                    Task::none()
                }
            },
            Message::RunAction(action) => self.execute_context_action(action),
            Message::BackgroundClicked => {
                if self.state_dirty {
                    self.window_state.save();
                }
                exit_launcher()
            }
            Message::ApplyPendingShrink(token) => self.apply_pending_shrink(token),
            Message::UncloakWindow(token) => self.apply_uncloak(token),
            Message::StabilizeSurfaceLayer(attempt) => self.handle_stabilize_surface_layer(attempt),
            Message::DelayedRefocus => self.handle_delayed_refocus(),
            Message::CheckUnfocusedExit => self.handle_check_unfocused_exit(),
        }
    }

    // ─── Subscription ─────────────────────────────────────────────────────

    pub fn subscription(&self) -> Subscription<Message> {
        // event::listen_with로 위젯이 소비한 이벤트도 캡처 (ESC 등)
        let keyboard_sub = iced::event::listen_with(|event, _status, _id| {
            if let iced::Event::Keyboard(kb_event) = event {
                match kb_event {
                    keyboard::Event::KeyPressed { key, modifiers, .. } => {
                        Some(Message::KeyEvent(key, modifiers))
                    }
                    _ => None,
                }
            } else {
                None
            }
        });

        let quit_sub = iced::time::every(Duration::from_millis(QUIT_POLL_MS))
            .map(|_| Message::CheckQuitSignal);

        let window_sub = window::events().map(|(id, event)| Message::WindowEvent(id, event));

        Subscription::batch([keyboard_sub, quit_sub, window_sub])
    }

    pub fn theme(&self) -> iced::Theme {
        iced::Theme::Dark
    }
}

// ─── Search Logic ─────────────────────────────────────────────────────────────

// 지원하는 검색창 프리픽스 전체 목록은 kmd_core::query_prefix::COMMANDS 레지스트리
// (단일 진실 소스)와 :help 화면을 참고.

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// 런처를 닫는다 — 닫기 전에 할 일을 한 곳에서.
///
/// 예전엔 `iced::exit()`가 20곳에 흩어져 있어 "닫을 때 무엇을 할지"를 넣을 자리가
/// 없었다. 지금 하는 일: 열 때 영문으로 바꿔 둔 입력 소스를 되돌린다(macOS —
/// Windows는 런처 창의 IME만 바꿔 되돌릴 것이 없다). 런처 창이 아직 앞에 있을 때
/// 불려야 안전하다(`platform::restore_input_source` 참조). 포커스를 잃어 닫히는
/// 경로만 예외로 `iced::exit()`를 직접 쓴다(focus_flow).
fn exit_launcher() -> Task<Message> {
    crate::platform::restore_input_source();
    iced::exit()
}

/// 설정 저장 대상 경로.
///
/// 테스트에서는 **실제 사용자 설정 파일을 건드리면 안 된다.** 설정 토글을
/// 검증하는 테스트들이 이 함수를 타는데, 그게 `~/Library/Application Support`의
/// 진짜 config.toml을 쓰는 바람에 `cargo test`를 돌릴 때마다 사용자의 IME
/// 설정·테마·공급자 목록이 바뀌었다(실제 발생: "재설치하면 초기화된다"고
/// 보고된 증상의 정체). 테스트에서는 임시 경로로 격리한다.
fn config_save_path() -> std::path::PathBuf {
    #[cfg(test)]
    {
        // 테스트 바이너리별로 고유한 임시 파일 — 병렬 실행에도 섞이지 않는다.
        std::env::temp_dir().join(format!(
            "kmd_desktop_test_config_{}.toml",
            std::process::id()
        ))
    }
    #[cfg(not(test))]
    {
        kmd_core::Config::default_config_dir().join(kmd_core::CONFIG_FILENAME)
    }
}

/// 설정 파일을 디스크에서 다시 읽어 **바꾸는 항목만** 고쳐 저장한다.
///
/// 통째로 덮어쓰지 않는 이유: 데몬이나 다른 인스턴스가 같은 파일을 만졌을 때
/// 우리 메모리 캐시로 그 변경을 지워버리지 않기 위해서다.
///
/// 실패하면 사용자에게 보여줄 메시지를 돌려준다 — 조용히 로그만 남기면
/// 사용자는 설정이 저장된 줄 안다.
fn save_config(f: impl FnOnce(&mut kmd_core::Config)) -> Option<String> {
    let path = config_save_path();
    match kmd_core::Config::update_and_save(&path, f) {
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("Failed to save config: {e}");
            Some(format!("설정 저장 실패: {e}"))
        }
    }
}

#[cfg(test)]
mod tests;
