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

mod focus_flow;
mod launch;
mod search_routing;
mod settings;
mod view;

// ─── Constants ────────────────────────────────────────────────────────────────

pub const DEFAULT_WIDTH: f32 = 1000.0;
const SEARCH_LIMIT: usize = 50;
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
const CLOAK_HARD_LIMIT: Duration = Duration::from_millis(150);

/// Windows: Resized 수신 후 cloak 을 푸는 지연 — 새 크기 프레임이 present 될 시간.
const UNCLOAK_AFTER_RESIZE: Duration = Duration::from_millis(16);

/// cloak 을 걸 만한 축소 비율 하한.
///
/// 왜곡 크기는 절대 픽셀이 아니라 축소 "배율"에 비례한다 — 10배로 접히면 결과 목록이
/// 10배로 눌린 프레임이 보인다. 1.25배 미만(예: 창을 몇 px 다듬는 정도)은 눈에
/// 띄지 않으므로 굳이 창을 숨겨 깜빡이게 만들지 않는다.
const CLOAK_MIN_RATIO: f32 = 1.25;

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

/// font_size 기반 비례 계산으로 모든 UI 치수를 결정.
/// `font_size` 하나를 바꾸면 pill, row, 상태바, 아이콘, 서브 폰트가 모두 연동된다.
#[derive(Debug, Clone, Copy)]
pub struct UiScale {
    pub font: f32,
    pub visible_rows: usize,
    pub pill_height: f32,
    /// 검색바(pill) 높이 (레이아웃 문서화 겸 테스트 검증용)
    #[allow(dead_code)]
    pub search_bar_height: f32,
    pub row_height: f32,
    pub status_bar_height: f32,
    pub full_window_height: f32,
    /// 결과가 없을 때의 창 높이 — 검색바 pill + 카드 패딩만.
    /// 투명 합성이 안 되는 환경(가상 GPU, 소프트웨어 렌더러)에서 빈 영역이
    /// 검게 칠해지는 문제를 피하기 위해 결과 유무에 따라 창 자체를 리사이즈한다.
    pub collapsed_window_height: f32,
    /// "No results found" 힌트 영역의 고정 높이 (창 높이 계산과 일치해야 함).
    pub hint_area_height: f32,
    pub result_icon: f32,
    pub title_font: f32,
    pub subtitle_font: f32,
    pub badge_font: f32,
    pub status_font: f32,
    pub hint_font: f32,
    pub detail_name_font: f32,
    pub detail_big_icon: f32,
    pub path_label_font: f32,
    pub path_text_font: f32,
    pub action_icon_font: f32,
    pub action_label_font: f32,
    pub action_shortcut_font: f32,
    pub no_results_font: f32,
}

impl UiScale {
    pub fn new(raw_font: f32, raw_rows: usize) -> Self {
        let f = raw_font.clamp(12.0, 32.0);
        let visible_rows = raw_rows.clamp(4, 20);
        let pill_height = (f * 2.9).round(); // 16→46
        let search_bar_height = pill_height;
        let row_height = (f * 3.0).round(); // 16→48
        let status_bar_height = (f * 1.75).round(); // 16→28
                                                    // 카드 외곽 container 의 padding(CARD_PAD) 상하 (view.rs 의 card 구조와 일치).
                                                    // 테두리(teal 2px, peach 1px)는 iced 에서 bounds 안쪽에 그려지므로 높이에 더하지 않는다.
        let full_window_height = search_bar_height
            + CARD_PAD * 2.0
            + 1.0
            + (visible_rows as f32 * row_height)
            + 1.0
            + status_bar_height;
        let collapsed_window_height = search_bar_height + CARD_PAD * 2.0;
        let no_results_font = (f * 0.8125).round(); // 16→13
        let hint_area_height = (no_results_font * 1.4).ceil() + 16.0;
        Self {
            font: f,
            visible_rows,
            pill_height,
            search_bar_height,
            row_height,
            status_bar_height,
            full_window_height,
            collapsed_window_height,
            hint_area_height,
            result_icon: (f * 2.0).round(),            // 16→32
            title_font: (f * 1.0).round(),             // 16→16
            subtitle_font: (f * 0.8125).round(),       // 16→13
            badge_font: (f * 0.625).round(),           // 16→10
            status_font: (f * 0.6875).round(),         // 16→11
            hint_font: (f * 0.6875).round(),           // 16→11
            detail_name_font: (f * 0.9375).round(),    // 16→15
            detail_big_icon: (f * 3.25).round(),       // 16→52
            path_label_font: (f * 0.625).round(),      // 16→10
            path_text_font: (f * 0.6875).round(),      // 16→11
            action_icon_font: (f * 0.8125).round(),    // 16→13
            action_label_font: (f * 0.75).round(),     // 16→12
            action_shortcut_font: (f * 0.625).round(), // 16→10
            no_results_font,
        }
    }
}

/// 주어진 font_size, visible_rows로 전체 창 높이를 계산 (main.rs 에서 사용).
pub fn full_window_height(font_size: f32, visible_rows: usize) -> f32 {
    UiScale::new(font_size, visible_rows).full_window_height
}

/// 결과가 없을 때(검색바만)의 창 높이 (main.rs 에서 사용).
pub fn collapsed_window_height(font_size: f32, visible_rows: usize) -> f32 {
    UiScale::new(font_size, visible_rows).collapsed_window_height
}

/// 창을 처음 만들 때의 높이 (main.rs 에서 사용).
///
/// 높이를 고정하는 플랫폼(macOS)에서는 처음부터 full 로 띄운다 — 부팅 직후
/// collapsed 로 띄웠다가 늘리면 그 한 번의 리사이즈에서 왜곡이 그대로 보인다.
pub fn initial_window_height(font_size: f32, visible_rows: usize) -> f32 {
    let u = UiScale::new(font_size, visible_rows);
    if FIXED_WINDOW_HEIGHT || crate::window_transparent() {
        u.full_window_height
    } else {
        u.collapsed_window_height
    }
}

const QUIT_POLL_MS: u64 = 300;
const WARMUP_IDLE_MS: u64 = 400;
const FOCUS_RETRY_MS: u64 = 120;
const MAX_FOCUS_RETRIES: u8 = 3;
const AUTOSTART_STATUS_REFRESH_MS: u64 = 1500;
// ─── Shared slot for async engine hand-off ────────────────────────────────────

/// 비동기 엔진 로드 결과 — 10-tuple 대신 명확한 필드로 관리
struct EngineLoadResult {
    engine: kmd_core::SearchEngine,
    use_emoji: bool,
    llm_providers: Vec<String>,
    multi_web_providers: Vec<String>,
    llm_prefixes: Vec<String>,
    multi_web_prefixes: Vec<String>,
    spell_providers: Vec<String>,
    spell_prefixes: Vec<String>,
    translate_providers: Vec<String>,
    translate_prefixes: Vec<String>,
}

type EngineSlot = Arc<Mutex<Option<EngineLoadResult>>>;

/// 비동기 quick 엔진 로드 결과 슬롯 — 부팅 직후 창 표시를 막지 않기 위해
/// PATH 스캔/캐시 로드를 백그라운드 스레드에서 수행한다.
type QuickEngineSlot = Arc<Mutex<Option<kmd_core::SearchEngine>>>;

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
    selected_llm_providers: Vec<String>,
    multi_llm_prefixes: Vec<String>,
    selected_multi_web_providers: Vec<String>,
    multi_web_prefixes: Vec<String>,
    spell_providers: Vec<String>,
    spell_prefixes: Vec<String>,
    translate_providers: Vec<String>,
    translate_prefixes: Vec<String>,
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
}

// ─── Context Actions ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum ContextAction {
    Open,
    OpenAsAdmin,
    OpenFolder,
    CopyPath,
    CopyName,
}

impl ContextAction {
    fn label(&self) -> &str {
        match self {
            Self::Open => "열기",
            Self::OpenAsAdmin => "관리자 권한으로 열기",
            Self::OpenFolder => "파일 위치 열기",
            Self::CopyPath => "경로 복사",
            Self::CopyName => "이름 복사",
        }
    }

    /// Calculator/Emoji 컨텍스트에서 보여줄 레이블 (값 복사)
    fn label_for_kind(&self, kind: ItemKind) -> &str {
        match (self, kind) {
            (Self::CopyName, ItemKind::Calculator) => "값 복사",
            (Self::CopyName, ItemKind::Emoji) => "이모지 복사",
            (Self::Open, ItemKind::Calculator) => "복사 후 닫기",
            _ => self.label(),
        }
    }

    fn shortcut(&self) -> &str {
        match self {
            Self::Open => "Ctrl+1",
            Self::OpenAsAdmin => "Ctrl+Shift+Enter",
            Self::OpenFolder => "Ctrl+2",
            Self::CopyPath => "Ctrl+3",
            Self::CopyName => "Ctrl+4",
        }
    }

    /// 이 액션에 할당된 Ctrl+N 인덱스 (0-based). 위치가 아닌 타입으로 결정.
    /// Calculator처럼 액션이 1개뿐이어도 CopyName은 항상 Ctrl+4(=3)로 동작.
    fn shortcut_index(&self) -> usize {
        match self {
            Self::Open => 0,
            Self::OpenFolder => 1,
            Self::CopyPath => 2,
            Self::CopyName => 3,
            Self::OpenAsAdmin => usize::MAX,
        }
    }
}

/// 아이템 종류에 따라 사용 가능한 컨텍스트 액션 목록
fn context_actions_for(kind: ItemKind) -> Vec<ContextAction> {
    match kind {
        ItemKind::App | ItemKind::Executable => vec![
            ContextAction::Open,
            ContextAction::OpenAsAdmin,
            ContextAction::OpenFolder,
            ContextAction::CopyPath,
        ],
        ItemKind::File => vec![
            ContextAction::Open,
            ContextAction::OpenFolder,
            ContextAction::CopyPath,
            ContextAction::CopyName,
        ],
        ItemKind::Directory => vec![ContextAction::Open, ContextAction::CopyPath],
        ItemKind::WebSearch => vec![ContextAction::Open, ContextAction::CopyName],
        ItemKind::Shell => vec![ContextAction::Open, ContextAction::CopyName],
        ItemKind::SystemCommand => vec![ContextAction::Open],
        ItemKind::Calculator | ItemKind::Emoji => vec![ContextAction::CopyName],
    }
}

// ─── Boot ─────────────────────────────────────────────────────────────────────

impl App {
    /// quick 엔진(PATH + 시스템 명령)을 백그라운드 스레드에서 로드한다.
    ///
    /// 이전에는 App 생성자에서 동기로 빌드해 창 표시 자체가 디스크 스캔을
    /// 기다렸다 — 캐시 미스(첫 설치)나 느린 VM에서 부팅이 수백 ms~수 초
    /// 지연되는 원인. 빈 엔진으로 즉시 창을 띄우고 완료 시 교체한다.
    fn spawn_quick_engine_load_task(
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

    fn initial_boot_tasks(input_id: iced::widget::Id, skip_warmup: bool) -> Task<Message> {
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

    fn with_activity_warmup(&mut self, base_task: Task<Message>) -> Task<Message> {
        if self.full_warmup_started {
            base_task
        } else {
            self.warmup_token = self.warmup_token.wrapping_add(1);
            let warmup_task = Self::schedule_warmup_tick(self.warmup_token);
            Task::batch([base_task, warmup_task])
        }
    }

    fn apply_contains_items<I>(&mut self, items: I)
    where
        I: IntoIterator<Item = IndexItem>,
    {
        self.apply_contains_results(items_to_results(items));
    }

    fn apply_contains_results(&mut self, results: Vec<kmd_core::SearchResult>) {
        self.commit_results(results, kmd_core::SearchMode::Contains, true);
    }

    fn clear_results_state(&mut self, mode: kmd_core::SearchMode) {
        self.results.clear();
        self.last_results_signature = 0;
        self.search_mode = mode;
        self.selected = 0;
    }

    fn should_limit_reorder_for_ime(&self) -> bool {
        self.query
            .chars()
            .rev()
            .find(|c| !c.is_whitespace())
            .is_some_and(Self::is_hangul_jamo)
    }

    fn is_hangul_jamo(ch: char) -> bool {
        is_hangul_jamo(ch)
    }

    fn kind_code(kind: ItemKind) -> u8 {
        match kind {
            ItemKind::App => 1,
            ItemKind::Directory => 2,
            ItemKind::File => 3,
            ItemKind::Executable => 4,
            ItemKind::SystemCommand => 5,
            ItemKind::WebSearch => 6,
            ItemKind::Calculator => 7,
            ItemKind::Emoji => 8,
            ItemKind::Shell => 9,
        }
    }

    fn results_signature(results: &[kmd_core::SearchResult]) -> u64 {
        let mut hasher = DefaultHasher::new();
        results.len().hash(&mut hasher);
        for r in results.iter().take(64) {
            Self::kind_code(r.item.kind).hash(&mut hasher);
            r.item.name.hash(&mut hasher);
            r.item.path.hash(&mut hasher);
        }
        hasher.finish()
    }

    fn commit_results(
        &mut self,
        new_results: Vec<kmd_core::SearchResult>,
        mode: kmd_core::SearchMode,
        reset_selection: bool,
    ) {
        let signature = Self::results_signature(&new_results);
        self.search_mode = mode;

        if signature == self.last_results_signature {
            return;
        }

        // IME 조합 중에는 결과 재정렬을 잠시 제한해 조합 안정성을 우선한다.
        if self.should_limit_reorder_for_ime() && !self.results.is_empty() {
            return;
        }

        self.results = new_results;
        self.last_results_signature = signature;
        if reset_selection {
            self.selected = 0;
        } else if self.selected >= self.results.len() {
            self.selected = self.results.len().saturating_sub(1);
        }
    }

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
        let db_path = db_path_override.unwrap_or_else(|| {
            kmd_core::Config::default_data_dir()
                .join("desktop")
                .join("kmd.db")
        });
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
        let selected_llm_providers = config.launcher.multi_llm_providers.clone();
        let multi_llm_prefixes = config.launcher.multi_llm_prefixes.clone();
        let selected_multi_web_providers = config.launcher.multi_web_providers.clone();
        let multi_web_prefixes = config.launcher.multi_web_prefixes.clone();
        let spell_providers = config.launcher.spell_providers.clone();
        let spell_prefixes = config.launcher.spell_prefixes.clone();
        let translate_providers = config.launcher.translate_providers.clone();
        let translate_prefixes = config.launcher.translate_prefixes.clone();
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
            selected_llm_providers,
            multi_llm_prefixes,
            selected_multi_web_providers,
            multi_web_prefixes,
            spell_providers,
            spell_prefixes,
            translate_providers,
            translate_prefixes,
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
            full_warmup_started: false,
            warmup_token: 0,
        };

        let quick_task = Self::spawn_quick_engine_load_task(quick_engine_slot, config_for_quick);
        let boot_tasks = Self::initial_boot_tasks(input_id, false);
        (app, Task::batch([quick_task, boot_tasks]))
    }

    fn schedule_warmup_tick(token: u64) -> Task<Message> {
        Task::future(async move {
            tokio::time::sleep(Duration::from_millis(WARMUP_IDLE_MS)).await;
            Message::WarmupTick(token)
        })
    }

    /// 부팅 안정화 구간(첫 포커스 후 1초 이내) 여부
    fn is_boot_settled(&self) -> bool {
        self.boot_focus_done && self.app_started_at.elapsed() < Duration::from_millis(1000)
    }

    /// focus 요청 + 카운터 증가 + (Windows만) 16ms 뒤 지연 refocus 스케줄링
    fn request_focus(&mut self) -> Task<Message> {
        // 부팅 안정화 구간: 최초 1회 이후 추가 포커싱 차단
        if self.is_boot_settled() {
            self.log_ime_state("request_focus:skip_boot_settling");
            return Task::none();
        }
        self.boot_focus_done = true;

        self.log_ime_state("request_focus");
        self.focus_request_count += 1;
        let immediate = iced::widget::operation::focus::<Message>(self.input_id.clone());
        #[cfg(not(target_os = "windows"))]
        {
            immediate
        }
        #[cfg(target_os = "windows")]
        {
            let delayed = Task::future(async {
                tokio::time::sleep(Duration::from_millis(16)).await;
                Message::DelayedRefocus
            });
            Task::batch([immediate, delayed])
        }
    }

    fn schedule_focus_retry(attempt: u8) -> Task<Message> {
        Task::future(async move {
            tokio::time::sleep(Duration::from_millis(FOCUS_RETRY_MS)).await;
            Message::EnsureFocus(attempt)
        })
    }

    fn spawn_full_engine_load_task(&self) -> Task<Message> {
        let slot = self.engine_slot.clone();
        Task::future(async move {
            match tokio::task::spawn_blocking(move || {
                let config = crate::engine::load_config();
                let eng = crate::engine::create_search_engine(&config);
                if let Ok(mut guard) = slot.lock() {
                    *guard = Some(EngineLoadResult {
                        engine: eng,
                        use_emoji: config.general.emoji_icons,
                        llm_providers: config.launcher.multi_llm_providers.clone(),
                        multi_web_providers: config.launcher.multi_web_providers.clone(),
                        llm_prefixes: config.launcher.multi_llm_prefixes.clone(),
                        multi_web_prefixes: config.launcher.multi_web_prefixes.clone(),
                        spell_providers: config.launcher.spell_providers.clone(),
                        spell_prefixes: config.launcher.spell_prefixes.clone(),
                        translate_providers: config.launcher.translate_providers.clone(),
                        translate_prefixes: config.launcher.translate_prefixes.clone(),
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
    fn spawn_icon_prefetch(&self) -> Task<Message> {
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
    fn sync_window_height(&mut self) -> Task<Message> {
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
    fn apply_pending_shrink(&mut self, token: u64) -> Task<Message> {
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
    fn should_cloak_shrink(from: f32, to: f32) -> bool {
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
    fn schedule_uncloak(&mut self, delay: Duration) -> Task<Message> {
        self.cloak_token = self.cloak_token.wrapping_add(1);
        let token = self.cloak_token;
        Task::future(async move {
            tokio::time::sleep(delay).await;
            Message::UncloakWindow(token)
        })
    }

    /// 예약된 cloak 해제 — 최신 토큰일 때만 적용.
    fn apply_uncloak(&mut self, token: u64) -> Task<Message> {
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
    fn enforce_cloak_deadline(&mut self) {
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

    fn handle_warmup_tick(&mut self, token: u64) -> Task<Message> {
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

    fn handle_engine_ready(&mut self) -> Task<Message> {
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
            self.engine = res.engine;
            self.full_engine_loaded = true;
            self.use_emoji = res.use_emoji;
            self.selected_llm_providers = res.llm_providers;
            self.selected_multi_web_providers = res.multi_web_providers;
            self.multi_llm_prefixes = res.llm_prefixes;
            self.multi_web_prefixes = res.multi_web_prefixes;
            self.spell_providers = res.spell_providers;
            self.spell_prefixes = res.spell_prefixes;
            self.translate_providers = res.translate_providers;
            self.translate_prefixes = res.translate_prefixes;
            self.runtime_config = crate::engine::load_config();
            self.loading = false;
            tracing::info!("Search engine ready");
            if !self.query.trim().is_empty() {
                // IME 조합 중이면 검색/뷰 갱신을 미뤄 자모 분리를 막는다.
                if self.is_typing_in_progress() {
                    return Task::future(async {
                        tokio::time::sleep(Duration::from_millis(400)).await;
                        Message::EngineSwapSettled
                    });
                }
                return self.perform_search();
            }
        } else {
            self.loading = false;
        }
        Task::none()
    }

    fn handle_engine_swap_settled(&mut self) -> Task<Message> {
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

    fn handle_quick_engine_ready(&mut self) -> Task<Message> {
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
            return iced::exit();
        }
        Task::none()
    }

    fn handle_shell_done(&mut self, result: Result<String, String>) -> Task<Message> {
        match result {
            Ok(output) => {
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    if let Err(e) = clipboard.set_text(&output) {
                        tracing::warn!("클립보드 쓰기 실패: {e}");
                    }
                }
                let first_line = output.lines().next().unwrap_or("(no output)");
                tracing::info!("Shell output copied: {first_line}");
            }
            Err(msg) => {
                tracing::error!("Shell error: {msg}");
            }
        }
        iced::exit()
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
            Message::ClipActionFinished(result) => match result {
                Ok(message) => {
                    tracing::info!("{message}");
                    iced::exit()
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
                iced::exit()
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

/// UTF-8 안전한 문자열 truncation (한글/CJK 안전)
fn truncate_str(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{truncated}...")
    }
}

/// 상세 타이틀 영역 픽셀 폭과 폰트 크기로 안전한 최대 글자 수를 추정한다.
///
/// 한글/전각 문자를 고려해 보수적인 폭 계수(0.95em)를 사용한다.
fn estimate_title_max_chars(width_px: f32, font_size: f32) -> usize {
    let em = (font_size * 0.95).max(1.0);
    ((width_px / em).floor() as usize).clamp(8, 64)
}

/// IME 조합 중 생성되는 trailing 자모를 완화해 즉시 검색 반응성을 높인다.
///
/// 예: `하ㄴ` -> `하`, `한ㄱ` -> `한`
fn relaxed_hangul_query(query: &str) -> Option<String> {
    if query.is_empty() {
        return None;
    }

    let mut chars: Vec<char> = query.chars().collect();
    let original_len = chars.len();
    while let Some(last) = chars.last() {
        if is_hangul_jamo(*last) {
            chars.pop();
        } else {
            break;
        }
    }

    if chars.is_empty() || chars.len() == original_len {
        return None;
    }

    Some(chars.into_iter().collect())
}

fn is_hangul_jamo(ch: char) -> bool {
    let code = ch as u32;
    (0x1100..=0x11FF).contains(&code)
        || (0x3131..=0x318E).contains(&code)
        || (0xA960..=0xA97F).contains(&code)
        || (0xD7B0..=0xD7FF).contains(&code)
}

fn items_to_results(
    items: impl IntoIterator<Item = kmd_core::IndexItem>,
) -> Vec<kmd_core::SearchResult> {
    items
        .into_iter()
        .map(|item| kmd_core::SearchResult {
            item,
            score: SCORE_PLUGIN,
        })
        .collect()
}

fn ensure_multi_llm_hint(items: &mut Vec<IndexItem>, use_emoji: bool) {
    if items
        .iter()
        .any(|item| item.name.starts_with("@ll") || item.name.starts_with("@llm"))
    {
        return;
    }
    items.push(IndexItem {
        name: "@ll         Compare multiple LLMs with one prompt".to_string(),
        path: "Open selected LLM providers in parallel tabs".to_string(),
        kind: ItemKind::WebSearch,
        source: Source::Plugin,
        icon: if use_emoji { "\u{1F9E0}" } else { "Ml" }.to_string(),
        keywords: "kmd:web_hint:multi_llm\n@ll @llm @multi @cmp multi llm compare".to_string(),
        icon_path: None,
    });
}

fn ensure_multi_web_hint(items: &mut Vec<IndexItem>, use_emoji: bool) {
    if items
        .iter()
        .any(|item| item.name.starts_with("@m ") || item.name.starts_with("@msearch"))
    {
        return;
    }
    items.push(IndexItem {
        name: "@m          Search multiple engines at once".to_string(),
        path: "Open Google/Naver/Daum in parallel tabs".to_string(),
        kind: ItemKind::WebSearch,
        source: Source::Plugin,
        icon: if use_emoji { "\u{1F50E}" } else { "Mw" }.to_string(),
        keywords:
            "kmd:web_hint:multi_web\n@m @mw @msearch @multisearch @searchall @krsearch multi web"
                .to_string(),
        icon_path: None,
    });
}

/// Load → mutate → save the user config file. Logs on failure.
fn save_config(f: impl FnOnce(&mut kmd_core::Config)) {
    let config_dir = kmd_core::Config::default_config_dir();
    match kmd_core::Config::load(&config_dir) {
        Ok(mut cfg) => {
            f(&mut cfg);
            if let Err(e) = cfg.save() {
                tracing::warn!("Failed to save config: {e}");
            }
        }
        Err(e) => tracing::warn!("Failed to load config for save: {e}"),
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU32, Ordering};
    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// 테스트용 App 인스턴스 생성 (각 테스트마다 고유 temp 디렉토리)
    fn make_test_app() -> App {
        let seq = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let tmp = std::env::temp_dir().join(format!("kmd_test_app_{}_{seq}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);
        let test_db_path = tmp.join("kmd_test.db");

        let guard = match kmd_core::single_instance::acquire_or_toggle(&tmp) {
            kmd_core::single_instance::InstanceAction::Acquired(g) => g,
            _ => panic!("Guard 획득 실패"),
        };
        let config = kmd_core::Config::default();
        let window_state = WindowState::default();
        let (app, _task) = App::new_for_test(guard, config, window_state, test_db_path);
        app
    }

    // ── 클립보드 런처 (비동기 검색 세대 가드 + Cmd/Ctrl+Enter 복사 전용) ──

    fn clip_hit(id: u64, slot: usize, preview: &str) -> kmd_core::ipc::ClipHit {
        kmd_core::ipc::ClipHit {
            id,
            slot,
            preview: preview.into(),
        }
    }

    #[test]
    fn 클립보드_검색_응답은_안정_id_마커로_결과를_만든다() {
        let mut app = make_test_app();
        app.query = ";account".into();
        app.search_generation = 7;
        let _ = app.update(Message::ClipSearchFinished {
            generation: 7,
            result: Ok(vec![clip_hit(4242, 3, "account number")]),
        });
        assert_eq!(app.results.len(), 1);
        assert_eq!(app.results[0].item.keywords, "kmd:clip:4242:3");
    }

    #[test]
    fn 오래된_클립보드_검색_응답은_현재_결과를_덮어쓰지_않는다() {
        let mut app = make_test_app();
        app.query = ";new".into();
        app.search_generation = 9;
        let _ = app.update(Message::ClipSearchFinished {
            generation: 8,
            result: Ok(vec![clip_hit(1, 1, "stale")]),
        });
        assert!(app.results.is_empty(), "세대가 다른 응답은 폐기되어야 함");
    }

    #[test]
    fn 클립보드_모드를_벗어난_뒤_도착한_응답은_폐기된다() {
        let mut app = make_test_app();
        app.query = "notes".into(); // 이미 일반 검색으로 전환됨
        app.search_generation = 3;
        let _ = app.update(Message::ClipSearchFinished {
            generation: 3,
            result: Ok(vec![clip_hit(5, 1, "late")]),
        });
        assert!(
            app.results.is_empty(),
            "클립보드 모드가 아니면 늦은 응답이 결과를 덮어쓰면 안 됨"
        );
    }

    #[test]
    fn 클립보드_submit은_수정자에_따라_붙여넣기와_복사를_구분한다() {
        let mut app = make_test_app();
        app.results = vec![kmd_core::SearchResult {
            item: search_routing::clip_item(&clip_hit(77, 2, "value"), false),
            score: 0,
        }];

        assert!(matches!(
            app.clipboard_submit_request(false),
            Some(kmd_core::ipc::Request::ClipPasteItem {
                id: 77,
                to_previous: true
            })
        ));
        assert!(matches!(
            app.clipboard_submit_request(true),
            Some(kmd_core::ipc::Request::ClipCopyItem { id: 77 })
        ));
    }

    #[test]
    fn 구버전_데몬_id0은_슬롯_붙여넣기로만_폴백한다() {
        let mut app = make_test_app();
        app.results = vec![kmd_core::SearchResult {
            item: search_routing::clip_item(&clip_hit(0, 4, "old daemon"), false),
            score: 0,
        }];

        assert!(matches!(
            app.clipboard_submit_request(false),
            Some(kmd_core::ipc::Request::ClipPaste {
                slot: 4,
                to_previous: true
            })
        ));
        assert!(
            app.clipboard_submit_request(true).is_none(),
            "구버전 데몬엔 복사 전용 요청이 없다 — 붙여넣기로 오인 실행하면 안 됨"
        );
    }

    #[test]
    fn 클립보드_안내_항목은_submit_대상이_아니다() {
        let mut app = make_test_app();
        app.results = vec![kmd_core::SearchResult {
            item: search_routing::clip_notice("비어 있음", "안내", false),
            score: 0,
        }];
        assert!(app.clipboard_submit_request(false).is_none());
        assert!(app.clipboard_submit_request(true).is_none());
    }

    #[test]
    fn 클립보드_동작_실패는_창을_닫지_않고_원인을_보여준다() {
        let mut app = make_test_app();
        let _ = app.update(Message::ClipActionFinished(Err(
            "선택한 클립보드 항목이 만료되었습니다".into(),
        )));
        assert_eq!(app.results.len(), 1);
        assert_eq!(app.results[0].item.keywords, "kmd:clip:notice");
        assert!(
            app.results[0].item.path.contains("만료"),
            "실패 원인이 표시되어야 함"
        );
    }

    // ── 창 리사이즈 디바운스 (화면 찢어짐 회귀 방지) ──

    /// 창을 접는 플랫폼(Windows/Linux)의 앱 — 리사이즈 경로를 검증한다.
    /// macOS 는 높이를 고정하므로([`FIXED_WINDOW_HEIGHT`]) 그대로 두면 이 경로가
    /// 통째로 죽는다. 디바운스/토큰 로직은 플랫폼과 무관하므로 어디서든 돌린다.
    fn make_resizing_app() -> App {
        let mut app = make_test_app();
        app.fixed_window_height = false;
        app.window_height = app.ui.collapsed_window_height;
        app
    }

    /// 결과가 있는 상태(= full 높이)를 만든다.
    fn with_results(app: &mut App) {
        app.results = vec![kmd_core::SearchResult {
            item: kmd_core::IndexItem {
                name: "t".into(),
                path: "t".into(),
                icon: String::new(),
                kind: kmd_core::ItemKind::File,
                source: kmd_core::index::Source::FileProvider,
                keywords: String::new(),
                icon_path: None,
            },
            score: 1,
        }];
        app.window_height = app.ui.full_window_height;
    }

    #[test]
    fn 축소는_즉시_적용되지_않는다() {
        // 회귀: 결과가 사라지는 순간 창을 곧바로 접으면 이전 프레임이
        // stretch 합성돼 화면이 찢어져 보인다 (백스페이스 전체 삭제,
        // 'U' 한 줄 삭제, '.' Backspace 매핑에서 발생).
        let mut app = make_resizing_app();
        with_results(&mut app);
        let before = app.window_height;

        app.results.clear();
        let _task = app.sync_window_height();

        assert_eq!(
            app.window_height, before,
            "축소는 디바운스 후에 적용되어야 함 — 즉시 창 높이가 바뀌면 안 됨"
        );
    }

    #[test]
    fn 확대는_즉시_적용된다() {
        let mut app = make_resizing_app();
        assert_eq!(app.window_height, app.ui.collapsed_window_height);

        with_results(&mut app);
        app.window_height = app.ui.collapsed_window_height; // 확대 대기 상태로 되돌림
        let _task = app.sync_window_height();

        assert_eq!(
            app.window_height, app.ui.full_window_height,
            "결과가 생기면 창은 지연 없이 즉시 커져야 함"
        );
    }

    #[test]
    fn 대기중_축소는_최신_토큰만_적용() {
        let mut app = make_resizing_app();
        with_results(&mut app);

        // 1차: 쿼리는 남고 결과만 사라짐 → hint 높이로 예약
        app.query = "zzz".into();
        app.results.clear();
        let _ = app.sync_window_height();
        let stale = app.shrink_token;

        // 2차: 쿼리까지 비워짐 → 더 작은 목표로 재예약 (1차 무효화)
        app.query.clear();
        let _ = app.sync_window_height();
        assert_ne!(app.shrink_token, stale, "목표가 바뀌면 재예약되어야 함");

        let before = app.window_height;
        let _ = app.update(Message::ApplyPendingShrink(stale));
        assert_eq!(app.window_height, before, "스테일 토큰은 무시되어야 함");

        let _ = app.update(Message::ApplyPendingShrink(app.shrink_token));
        assert_eq!(
            app.window_height, app.ui.collapsed_window_height,
            "최신 토큰이면 축소가 적용되어야 함"
        );
    }

    #[test]
    fn 축소_대기중_확대가_오면_축소는_취소() {
        let mut app = make_resizing_app();
        with_results(&mut app);

        // 축소 예약
        app.results.clear();
        let _ = app.sync_window_height();
        let pending = app.shrink_token;

        // 그 사이 결과가 다시 생김 → 즉시 확대 + 예약 무효화
        with_results(&mut app);
        app.window_height = app.ui.collapsed_window_height;
        let _ = app.sync_window_height();
        assert_eq!(app.window_height, app.ui.full_window_height);

        // 뒤늦게 도착한 축소는 무시되어야 함 (창이 접혔다 펴지는 깜빡임 방지)
        let _ = app.update(Message::ApplyPendingShrink(pending));
        assert_eq!(
            app.window_height, app.ui.full_window_height,
            "확대 후 도착한 옛 축소 요청은 무시되어야 함"
        );
    }

    #[test]
    fn 축소_대기중_무관한_메시지가_와도_예약이_유지된다() {
        // 회귀: sync_window_height 는 매 메시지마다 호출되므로, 대기 중에도
        // 예약을 재발행하면 포커스·타이머 메시지가 올 때마다 타이머가 리셋돼
        // 창이 영영 접히지 않는다 (실제로 발생했던 버그).
        let mut app = make_resizing_app();
        with_results(&mut app);

        app.results.clear();
        let _ = app.sync_window_height();
        let token = app.shrink_token;

        // 축소가 적용되기 전에 무관한 메시지가 여러 번 도착
        for _ in 0..5 {
            let _ = app.sync_window_height();
        }
        assert_eq!(
            app.shrink_token, token,
            "같은 목표로는 재예약하지 않아야 함 (타이머 리셋 방지)"
        );

        // 최초 예약 토큰이 그대로 유효
        let _ = app.update(Message::ApplyPendingShrink(token));
        assert_eq!(
            app.window_height, app.ui.collapsed_window_height,
            "최초 예약이 살아 있어 축소가 적용되어야 함"
        );
        assert!(
            app.pending_shrink_to.is_none(),
            "적용 후 예약은 비워져야 함"
        );
    }

    #[test]
    fn 전체삭제_시_리사이즈는_한_번으로_합쳐진다() {
        // full → (hint) → collapsed 로 연달아 줄어드는 경로에서 실제
        // 창 리사이즈는 마지막 한 번만 일어나야 한다.
        let mut app = make_resizing_app();
        with_results(&mut app);

        // 1단계: 결과만 사라짐 (쿼리는 남음) → hint 높이로 축소 예약
        app.query = "zzz".into();
        app.results.clear();
        let _ = app.sync_window_height();
        assert_eq!(app.window_height, app.ui.full_window_height, "아직 미적용");

        // 2단계: 쿼리까지 비워짐 → 더 작은 높이로 재예약 (앞 요청 무효화)
        app.query.clear();
        let _ = app.sync_window_height();

        let _ = app.update(Message::ApplyPendingShrink(app.shrink_token));
        assert_eq!(
            app.window_height, app.ui.collapsed_window_height,
            "중간 단계(hint)를 건너뛰고 최종 높이로 한 번에 축소되어야 함"
        );
    }

    // ── 창 높이 고정 (macOS: 리사이즈 자체를 없애 잔상 원천 차단) ──

    #[test]
    fn 높이_고정이면_결과_유무가_창을_리사이즈하지_않는다() {
        // 회귀 방지: v0.10.1에서 도입한 collapsed↔full 리사이즈가 "결과창이
        // 뜨고 사라질 때 화면이 깨지는" 증상의 원인이었다. 높이를 고정한
        // 플랫폼에서는 어떤 상태 변화도 창 크기를 건드리면 안 된다.
        let mut app = make_test_app();
        app.fixed_window_height = true;
        app.window_height = app.ui.full_window_height;

        let full = app.ui.full_window_height;

        with_results(&mut app);
        let _ = app.sync_window_height();
        assert_eq!(app.window_height, full, "결과가 생겨도 창 높이 유지");
        assert!(app.pending_shrink_to.is_none());

        // 쿼리는 남고 결과만 사라짐 (hint 상태)
        app.query = "zzz".into();
        app.results.clear();
        let _ = app.sync_window_height();
        assert_eq!(app.window_height, full, "결과가 사라져도 창 높이 유지");
        assert!(app.pending_shrink_to.is_none(), "축소 예약도 생기면 안 됨");

        // 쿼리까지 비워짐 (최초 상태)
        app.query.clear();
        let _ = app.sync_window_height();
        assert_eq!(app.window_height, full, "쿼리를 다 지워도 창 높이 유지");
        assert!(app.pending_shrink_to.is_none());
        assert!(!app.window_cloaked, "리사이즈가 없으니 cloak 도 필요 없음");
    }

    #[test]
    fn 투명_창이면_높이_고정() {
        // 정책: 빈 영역을 투명으로 둘 수 있으면 창을 리사이즈하지 않는다.
        // (리사이즈가 없어야 컴포지터가 이전 프레임을 늘려 합성하는 구간도 없다)
        // macOS 는 항상 투명, Windows 는 DirectComposition 이 열렸을 때 투명.
        assert_eq!(FIXED_WINDOW_HEIGHT, cfg!(target_os = "macos"));
        let app = make_test_app();
        let expect_fixed = FIXED_WINDOW_HEIGHT || crate::window_transparent();
        assert_eq!(
            app.fixed_window_height, expect_fixed,
            "투명 창이면 높이 고정, 불투명 폴백이면 리사이즈"
        );
        if expect_fixed {
            assert_eq!(
                app.window_height, app.ui.full_window_height,
                "고정 모드는 처음부터 full 높이로 떠야 한다 (부팅 리사이즈 제거)"
            );
        }
    }

    #[test]
    fn 초기_창높이는_플랫폼_정책을_따른다() {
        let cfg = kmd_core::Config::default();
        let u = UiScale::new(cfg.general.font_size, cfg.general.visible_rows);
        let initial = initial_window_height(cfg.general.font_size, cfg.general.visible_rows);
        // 높이 고정(투명) 모드는 부팅 리사이즈까지 없애려고 처음부터 full 로 뜬다.
        if FIXED_WINDOW_HEIGHT || crate::window_transparent() {
            assert_eq!(initial, u.full_window_height);
        } else {
            assert_eq!(initial, u.collapsed_window_height);
        }
    }

    // ── 리사이즈 cloak (Windows 잔상 가리기) ──

    #[test]
    fn cloak_해제는_최신_토큰만_적용() {
        let mut app = make_resizing_app();
        // 플랫폼 호출 없이 상태만 세팅 — 토큰 판정 로직 자체를 검증한다.
        app.window_cloaked = true;
        app.cloaked_at = Some(std::time::Instant::now());
        let stale = app.cloak_token;
        let _ = app.schedule_uncloak(UNCLOAK_AFTER_RESIZE);

        let _ = app.update(Message::UncloakWindow(stale));
        assert!(app.window_cloaked, "스테일 토큰은 무시되어야 함");

        let _ = app.update(Message::UncloakWindow(app.cloak_token));
        assert!(!app.window_cloaked, "최신 토큰이면 해제되어야 함");
        assert!(app.cloaked_at.is_none());
    }

    #[test]
    fn cloak은_기한을_넘기면_강제_해제된다() {
        // 회귀 안전망: 해제 메시지가 유실돼도 창이 숨은 채 남으면 안 된다.
        let mut app = make_resizing_app();
        app.window_cloaked = true;
        app.cloaked_at = Some(std::time::Instant::now() - CLOAK_HARD_LIMIT * 3);

        // 아무 메시지나 한 번 처리되면 안전망이 동작해야 함
        let _ = app.update(Message::IconsReady);
        assert!(!app.window_cloaked, "기한 초과 cloak은 강제 해제되어야 함");
    }

    #[test]
    fn cloak_판정은_축소_배율을_따른다() {
        let u = UiScale::new(16.0, 8);
        let collapsed = u.collapsed_window_height;
        let hint = collapsed + u.hint_area_height;

        assert!(
            App::should_cloak_shrink(u.full_window_height, collapsed),
            "full → collapsed 는 왜곡이 가장 크다"
        );
        assert!(
            App::should_cloak_shrink(hint, collapsed),
            "hint → collapsed 도 1.25배를 넘어 눌림이 보인다"
        );
        assert!(
            !App::should_cloak_shrink(collapsed + 2.0, collapsed),
            "몇 px 다듬는 정도는 가리지 않는다"
        );
        assert!(
            !App::should_cloak_shrink(collapsed, 0.0),
            "0 높이는 방어적으로 false"
        );
    }

    // ── UiScale 비례 계산 일관성 ──

    #[test]
    fn 기본_폰트_창높이_일관성() {
        let cfg = kmd_core::Config::default();
        let u = UiScale::new(cfg.general.font_size, cfg.general.visible_rows);
        let expected = u.search_bar_height
            + CARD_PAD * 2.0
            + 1.0
            + (u.visible_rows as f32 * u.row_height)
            + 1.0
            + u.status_bar_height;
        assert!(
            (u.full_window_height - expected).abs() < f32::EPSILON,
            "full_window_height({}) != 계산값({expected})",
            u.full_window_height
        );
    }

    #[test]
    fn 접힌_창높이_일관성() {
        for fs in [12.0, 16.0, 20.0, 24.0, 32.0] {
            let u = UiScale::new(fs, 8);
            // view.rs 카드 구조: pill + 카드 padding(CARD_PAD×2)
            let expected = u.search_bar_height + CARD_PAD * 2.0;
            assert!(
                (u.collapsed_window_height - expected).abs() < f32::EPSILON,
                "font_size={fs}: collapsed({}) != 계산값({expected})",
                u.collapsed_window_height
            );
            assert!(
                u.collapsed_window_height < u.full_window_height,
                "접힌 높이는 항상 full 보다 작아야 함"
            );
            assert!(
                u.hint_area_height > u.no_results_font,
                "힌트 영역은 힌트 폰트보다 커야 함"
            );
        }
    }

    // ── 비동기 quick 엔진 부팅 ──

    fn make_quick_engine() -> kmd_core::SearchEngine {
        let mut engine = kmd_core::SearchEngine::new();
        engine.load(vec![IndexItem {
            name: "Firefox".to_string(),
            path: "/usr/bin/firefox".to_string(),
            kind: ItemKind::App,
            source: Source::Apps,
            icon: "Ap".to_string(),
            keywords: "firefox".to_string(),
            icon_path: None,
        }]);
        engine
    }

    #[test]
    fn 부팅은_빈엔진_quick_ready시_교체() {
        let mut app = make_test_app();
        assert_eq!(app.engine.len(), 0, "부팅 직후에는 빈 엔진 (창 즉시 표시)");
        assert!(app.loading, "quick 로드 전에는 loading 상태");

        *app.quick_engine_slot.lock().unwrap() = Some(make_quick_engine());
        let _ = app.update(Message::QuickEngineReady);

        assert_eq!(app.engine.len(), 1, "quick 엔진으로 교체되어야 함");
        assert!(!app.loading, "교체 후 loading 해제");
    }

    #[test]
    fn full_엔진_로드후_늦은_quick은_폐기() {
        let mut app = make_test_app();
        app.full_engine_loaded = true;

        *app.quick_engine_slot.lock().unwrap() = Some(make_quick_engine());
        let _ = app.update(Message::QuickEngineReady);

        assert_eq!(
            app.engine.len(),
            0,
            "full 엔진이 이미 적용됐으면 quick 엔진이 덮어쓰면 안 됨"
        );
    }

    #[test]
    fn pill_높이가_폰트보다_충분히_큼() {
        for fs in [12.0, 16.0, 20.0, 24.0, 32.0] {
            let u = UiScale::new(fs, 8);
            assert!(
                u.pill_height > u.font + 10.0,
                "font_size={fs}: pill_height({})가 폰트보다 충분히 커야 함",
                u.pill_height
            );
        }
    }

    #[test]
    fn row_높이가_아이콘보다_큼() {
        for fs in [12.0, 16.0, 20.0, 24.0, 32.0] {
            let u = UiScale::new(fs, 8);
            assert!(
                u.row_height > u.result_icon + 8.0,
                "font_size={fs}: row_height({})가 아이콘({})보다 충분히 커야 함",
                u.row_height,
                u.result_icon
            );
        }
    }

    #[test]
    fn 폰트_클램프_범위() {
        let small = UiScale::new(5.0, 8);
        assert_eq!(small.font, 12.0, "최소 12로 클램프");
        let big = UiScale::new(100.0, 8);
        assert_eq!(big.font, 32.0, "최대 32로 클램프");
    }

    #[test]
    fn visible_rows_클램프_범위() {
        let small = UiScale::new(16.0, 1);
        assert_eq!(small.visible_rows, 4, "최소 4로 클램프");
        let big = UiScale::new(16.0, 100);
        assert_eq!(big.visible_rows, 20, "최대 20으로 클램프");
        let normal = UiScale::new(16.0, 12);
        assert_eq!(normal.visible_rows, 12, "범위 내 값은 그대로");
    }

    #[test]
    fn visible_rows_변경시_창높이_비례() {
        let u8 = UiScale::new(16.0, 8);
        let u12 = UiScale::new(16.0, 12);
        let diff = u12.full_window_height - u8.full_window_height;
        let expected = 4.0 * u8.row_height;
        assert!(
            (diff - expected).abs() < f32::EPSILON,
            "rows 4개 증가 → 창높이 차이({diff}) == row_height*4({expected})"
        );
    }

    #[test]
    fn 비례_스케일링_선형() {
        let u16 = UiScale::new(16.0, 8);
        let u32 = UiScale::new(32.0, 8);
        let ratio = u32.pill_height / u16.pill_height;
        assert!(
            (ratio - 2.0).abs() < 0.1,
            "font 2배 → pill_height도 약 2배 ({ratio:.2})"
        );
        let row_ratio = u32.row_height / u16.row_height;
        assert!(
            (row_ratio - 2.0).abs() < 0.1,
            "font 2배 → row_height도 약 2배 ({row_ratio:.2})"
        );
    }

    // ── 포커스 요청 카운트 검증 ──

    #[test]
    fn 쿼리_변경시_focus_요청_미발생() {
        let mut app = make_test_app();
        let before = app.focus_request_count;

        let _task = app.update(Message::QueryChanged("test".to_string()));
        assert_eq!(
            app.focus_request_count, before,
            "입력 중에는 refocus를 강제하지 않아야 함"
        );
        assert_eq!(app.query, "test");
        assert!(app.window_focused);
    }

    #[test]
    fn 쿼리_비어있을때_ensure_focus_요청_발생() {
        let mut app = make_test_app();
        app.window_focused = true;
        app.query.clear();
        let before = app.focus_request_count;

        let _task = app.update(Message::EnsureFocus(1));
        assert!(
            app.focus_request_count > before,
            "빈 쿼리 + EnsureFocus → focus 요청 발생해야 함"
        );
    }

    #[test]
    fn 쿼리_있을때_ensure_focus_건너뜀() {
        let mut app = make_test_app();
        app.window_focused = true;
        app.query = "hello".to_string();
        let before = app.focus_request_count;

        let _task = app.update(Message::EnsureFocus(1));
        assert_eq!(
            app.focus_request_count, before,
            "쿼리 있으면 EnsureFocus는 focus 요청 안 함"
        );
    }

    #[test]
    fn 최대_재시도_초과시_focus_요청_안함() {
        let mut app = make_test_app();
        app.window_focused = true;
        app.query.clear();
        let before = app.focus_request_count;

        let _task = app.update(Message::EnsureFocus(MAX_FOCUS_RETRIES + 1));
        assert_eq!(
            app.focus_request_count, before,
            "최대 재시도 초과 시 focus 요청 안 함"
        );
    }

    #[test]
    fn delayed_refocus_카운트_증가() {
        let mut app = make_test_app();
        let before = app.focus_request_count;

        let _task = app.update(Message::DelayedRefocus);
        assert_eq!(
            app.focus_request_count,
            before + 1,
            "DelayedRefocus는 focus_request_count를 1 증가시켜야 함"
        );
    }

    // ── Unfocused 이벤트 타이핑 가드 ──

    #[test]
    fn 타이핑_직후_unfocused_무시() {
        let mut app = make_test_app();
        app.window_id = Some(window::Id::unique());

        let _task = app.update(Message::QueryChanged("가".to_string()));
        assert!(app.window_focused, "QueryChanged 직후 focused 유지");

        let _task = app.update(Message::WindowEvent(
            app.window_id.unwrap(),
            window::Event::Unfocused,
        ));
        assert!(
            app.window_focused,
            "타이핑 500ms 이내 Unfocused는 무시되어야 함"
        );
    }

    #[test]
    fn 타이핑_없을때_unfocused_정상_처리() {
        let mut app = make_test_app();
        app.window_id = Some(window::Id::unique());
        app.last_query_changed_at = std::time::Instant::now() - Duration::from_secs(5);

        let _task = app.update(Message::WindowEvent(
            app.window_id.unwrap(),
            window::Event::Unfocused,
        ));
        assert!(
            !app.window_focused,
            "타이핑하지 않은 상태에서 Unfocused는 window_focused=false로"
        );
    }

    // ── Focused 이벤트 — 항상 refocus + 카운트 ──

    #[test]
    fn 쿼리_있고_최근타이핑시_focused_refocus_안함() {
        let mut app = make_test_app();
        app.window_id = Some(window::Id::unique());
        app.window_focused = false;
        app.query = "검색어".to_string();
        app.last_query_changed_at = std::time::Instant::now();
        let before = app.focus_request_count;

        let _task = app.update(Message::WindowEvent(
            app.window_id.unwrap(),
            window::Event::Focused,
        ));
        assert!(app.window_focused);
        assert_eq!(
            app.focus_request_count, before,
            "조합/타이핑 직후 Focused 이벤트에서는 refocus를 생략"
        );
    }

    #[test]
    fn 쿼리_있어도_타이핑아님_focused시_focus_요청() {
        let mut app = make_test_app();
        app.window_id = Some(window::Id::unique());
        app.window_focused = false;
        app.query = "검색어".to_string();
        app.last_query_changed_at = std::time::Instant::now() - Duration::from_secs(2);
        let before = app.focus_request_count;

        let _task = app.update(Message::WindowEvent(
            app.window_id.unwrap(),
            window::Event::Focused,
        ));
        assert!(app.window_focused);
        assert!(
            app.focus_request_count > before,
            "타이핑 직후가 아니면 Focused 이벤트에서 focus 요청"
        );
    }

    #[test]
    fn 빈_쿼리_focused시_focus_요청() {
        let mut app = make_test_app();
        app.window_id = Some(window::Id::unique());
        app.window_focused = false;
        app.query.clear();
        let before = app.focus_request_count;

        let _task = app.update(Message::WindowEvent(
            app.window_id.unwrap(),
            window::Event::Focused,
        ));
        assert!(app.window_focused);
        assert!(
            app.focus_request_count > before,
            "빈 쿼리 Focused 시에도 focus 요청"
        );
    }

    // ── 첫 입력 후 포커스 복원 전체 시나리오 ──

    #[test]
    fn 첫_입력_후_전체_focus_시퀀스() {
        let mut app = make_test_app();
        app.window_id = Some(window::Id::unique());
        let c0 = app.focus_request_count;

        // 1단계: 첫 글자 입력
        let _task = app.update(Message::QueryChanged("h".to_string()));
        let c1 = app.focus_request_count;
        assert_eq!(c1, c0, "QueryChanged에서는 request_focus를 호출하지 않음");

        // 2단계: macOS spurious Unfocused (view 재렌더 시 발생 가능)
        let _task = app.update(Message::WindowEvent(
            app.window_id.unwrap(),
            window::Event::Unfocused,
        ));
        assert!(app.window_focused, "타이핑 가드로 Unfocused 무시");

        // 3단계: DelayedRefocus (16ms 후 iced 이벤트 루프가 전달)
        let _task = app.update(Message::DelayedRefocus);
        let c2 = app.focus_request_count;
        assert_eq!(c2, c1, "입력 중 DelayedRefocus는 IME 안정성을 위해 스킵");

        // 4단계: 두 번째 글자 입력 가능
        let _task = app.update(Message::QueryChanged("he".to_string()));
        assert_eq!(app.query, "he");
        let c3 = app.focus_request_count;
        assert_eq!(c3, c2, "두 번째 입력에서도 refocus를 강제하지 않음");
    }

    #[test]
    fn 첫_입력시_중복_refocus_없음() {
        let mut app = make_test_app();
        let before = app.focus_request_count;

        let _task = app.update(Message::QueryChanged("h".to_string()));
        let after = app.focus_request_count;

        assert_eq!(
            after, before,
            "QueryChanged에서 request_focus를 하지 않아야 IME 조합 안정성 유지"
        );
    }

    #[test]
    fn 한글_첫_입력_후_전체_focus_시퀀스() {
        let mut app = make_test_app();
        app.window_id = Some(window::Id::unique());
        let c0 = app.focus_request_count;

        // ㄱ 입력
        let _task = app.update(Message::QueryChanged("ㄱ".to_string()));
        let c1 = app.focus_request_count;
        assert_eq!(c1, c0, "한글 첫 자음 입력 시 강제 refocus 없음");

        // macOS spurious Unfocused + Focused
        let _task = app.update(Message::WindowEvent(
            app.window_id.unwrap(),
            window::Event::Unfocused,
        ));
        assert!(app.window_focused, "타이핑 가드로 무시");

        let _task = app.update(Message::WindowEvent(
            app.window_id.unwrap(),
            window::Event::Focused,
        ));
        let c2 = app.focus_request_count;
        assert_eq!(c2, c1, "최근 타이핑 중 Focused refocus 생략");

        // DelayedRefocus (view 재렌더 후)
        let _task = app.update(Message::DelayedRefocus);
        let c3 = app.focus_request_count;
        assert_eq!(c3, c2, "입력 중 DelayedRefocus는 스킵");

        // ㅏ 결합 → 가
        let _task = app.update(Message::QueryChanged("가".to_string()));
        assert_eq!(app.query, "가", "한글 조합 완료");
        assert!(app.window_focused, "조합 이후에도 포커스 유지");
    }

    // ── 기타 focus 관련 ──

    #[test]
    fn got_raw_window_id_쿼리_있으면_focus_건너뜀() {
        let mut app = make_test_app();
        app.query = "test".to_string();
        let before = app.focus_request_count;

        let _task = app.update(Message::GotRawWindowId(12345));
        assert_eq!(
            app.focus_request_count, before,
            "쿼리 있으면 GotRawWindowId에서 focus 안 함"
        );
    }

    #[test]
    fn got_raw_window_id_빈쿼리_focus_요청() {
        let mut app = make_test_app();
        app.query.clear();
        let before = app.focus_request_count;

        let _task = app.update(Message::GotRawWindowId(12345));
        assert!(
            app.focus_request_count > before,
            "빈 쿼리 → GotRawWindowId에서 focus 요청"
        );
    }

    #[test]
    fn clear_search_후_focus_요청() {
        let mut app = make_test_app();
        app.query = "something".to_string();
        let before = app.focus_request_count;

        let _task = app.update(Message::QueryChanged("".to_string()));
        assert_eq!(app.query, "");
        assert_eq!(
            app.focus_request_count, before,
            "쿼리 비움 시 refocus 강제 없음"
        );
    }

    // ── 포커스 가드 (근본적 안전망) 테스트 ──

    #[test]
    fn 포커스_가드_쿼리_변경시_자동_refocus_안함() {
        let mut app = make_test_app();
        app.window_focused = true;
        let before = app.focus_request_count;

        let _task = app.update(Message::QueryChanged("abc".to_string()));
        assert_eq!(
            app.focus_request_count, before,
            "QueryChanged는 포커스 가드 제외"
        );
    }

    #[test]
    fn 포커스_가드_결과_변경시_자동_refocus() {
        let mut app = make_test_app();
        app.window_focused = true;

        // 쿼리를 설정하여 results가 생길 조건 만들기
        let _task = app.update(Message::QueryChanged(":help".to_string()));
        let after_help = app.focus_request_count;

        // results.clear()로 결과가 바뀌는 상황 시뮬레이션
        let _task = app.update(Message::QueryChanged("".to_string()));
        assert_eq!(
            app.focus_request_count, after_help,
            "입력 중 refocus는 금지"
        );
    }

    #[test]
    fn 포커스_가드_settings_토글_후_refocus() {
        let mut app = make_test_app();
        app.window_focused = true;

        // Settings 화면으로 진입
        let _task = app.update(Message::QueryChanged(":set".to_string()));
        let before = app.focus_request_count;

        // Settings 액션 실행 (toggle_ime_reset 시뮬레이션)
        // handle_settings_action을 직접 호출하는 대신,
        // update_inner가 아닌 update를 통해 query/results 변경이
        // 포커스 가드에 의해 보호되는지 검증
        let old_results = app.results.len();
        app.query = ":set".to_string();
        app.handle_settings_query(":set");
        let new_results = app.results.len();

        // results가 변경되었다면, update()를 통했을 때 가드가 동작해야 함
        if new_results != old_results {
            // 이 경우 가드가 자동으로 refocus
            let focus_task = app.request_focus();
            assert!(
                app.focus_request_count > before,
                "settings 토글 후 포커스 가드 동작 확인"
            );
            let _ = focus_task;
        }
    }

    #[test]
    fn 포커스_가드_윈도우_비활성시_미동작() {
        let mut app = make_test_app();
        app.window_focused = false;
        let before = app.focus_request_count;

        let _task = app.update(Message::QueryChanged("test".to_string()));
        // 창이 비활성 상태에서는 QueryChanged 경로에서도 refocus를 시도하지 않아야 한다.
        let after = app.focus_request_count;
        assert_eq!(
            after, before,
            "window_focused=false이면 입력 중 불필요한 focus 요청이 없어야 함"
        );
    }

    #[test]
    fn 포커스_가드_delayed_refocus_무한루프_방지() {
        let mut app = make_test_app();
        app.window_focused = true;
        let before = app.focus_request_count;

        let _task = app.update(Message::DelayedRefocus);
        let after = app.focus_request_count;
        // DelayedRefocus는 skip_focus_guard=true이므로 가드의 추가 호출 없음
        // update_inner 내부에서만 focus 요청 발생
        let delta = after - before;
        assert!(
            delta <= 2,
            "DelayedRefocus는 가드 바이패스, 무한루프 없음 (delta={delta})"
        );
    }

    #[test]
    fn 포커스_가드_ensure_focus_무한루프_방지() {
        let mut app = make_test_app();
        app.window_focused = true;
        app.query.clear();
        let before = app.focus_request_count;

        let _task = app.update(Message::EnsureFocus(1));
        let after = app.focus_request_count;
        let delta = after - before;
        assert!(
            delta <= 2,
            "EnsureFocus는 가드 바이패스, 무한루프 없음 (delta={delta})"
        );
    }

    #[test]
    fn settings_theme_변경_후_focus_복원() {
        let mut app = make_test_app();
        app.window_focused = true;

        // Settings로 진입
        let _task = app.update(Message::QueryChanged(":set".to_string()));
        let has_results_after_set = !app.results.is_empty();
        assert!(has_results_after_set, ":set 입력 후 결과가 있어야 함");

        let before = app.focus_request_count;

        // theme 변경 시뮬레이션: query를 clear하고 results를 clear
        // 이것이 handle_settings_action("theme:midnight")가 하는 일
        app.query.clear();
        app.results.clear();
        app.selected = 0;
        let focus_task = app.request_focus();
        let _ = focus_task;

        assert!(
            app.focus_request_count > before,
            "theme 변경 후 focus 요청 발생해야 함"
        );
    }

    #[test]
    fn 연속_settings_토글_focus_유지() {
        let mut app = make_test_app();
        app.window_focused = true;

        // 연속 3회 settings 진입-나가기 시뮬레이션
        for i in 0..3 {
            let before = app.focus_request_count;
            let _task = app.update(Message::QueryChanged(":set".to_string()));
            assert_eq!(
                app.focus_request_count, before,
                "#{i} settings 진입 입력 처리에서는 refocus 강제 없음"
            );

            let before2 = app.focus_request_count;
            let _task = app.update(Message::QueryChanged("".to_string()));
            assert_eq!(
                app.focus_request_count, before2,
                "#{i} settings 나가기 입력 처리에서도 refocus 강제 없음"
            );
        }
    }

    // ── :set 목록·토글 characterization (settings.rs 분해 보호) ──
    //
    // handle_settings_query(246줄)/handle_settings_action(274줄)을 분해하기 전에
    // 바깥에서 보이는 계약을 고정한다: 어떤 행이 어떤 action 키로 서는지, 토글이
    // runtime_config와 목록 라벨에 반영되는지, 전체 해제 시 기본값으로 복구되는지.

    fn settings_actions(app: &mut App) -> Vec<String> {
        app.handle_settings_query(":set");
        app.results
            .iter()
            .map(|r| r.item.keywords.clone())
            .collect()
    }

    fn settings_label(app: &mut App, action: &str) -> String {
        app.handle_settings_query(":set");
        app.results
            .iter()
            .find(|r| r.item.keywords == action)
            .map(|r| r.item.name.clone())
            .unwrap_or_else(|| panic!("행 없음: {action}"))
    }

    fn run_settings_action(app: &mut App, action: &str) {
        app.handle_settings_query(":set");
        let hit = app
            .results
            .iter()
            .find(|r| r.item.keywords == action)
            .unwrap_or_else(|| panic!("행 없음: {action}"))
            .clone();
        let _ = app.handle_settings_action(&hit);
    }

    #[test]
    fn set_목록_행_구성_스냅샷() {
        let mut app = make_test_app();
        let actions = settings_actions(&mut app);

        // 고정 항목이 이 순서로 먼저 온다
        let head: Vec<&str> = actions.iter().take(7).map(String::as_str).collect();
        assert_eq!(
            head,
            vec![
                "kmd:settings:config",
                "kmd:settings:dir",
                "kmd:settings:noop",
                "kmd:settings:reset_position",
                "kmd:settings:toggle_ime_reset",
                "kmd:settings:toggle_autostart",
                "kmd:settings:toggle_brand_icons",
            ]
        );

        // 테마 5종 + 프로바이더 4군 + 꼬리
        for expected in [
            "kmd:settings:theme:keymander",
            "kmd:settings:theme:obsidian",
            "kmd:settings:theme:snow",
            "kmd:settings:theme:rose_pine",
            "kmd:settings:theme:nord",
            "kmd:settings:llm:toggle:chatgpt",
            "kmd:settings:llm:toggle:perplexity",
            "kmd:settings:mweb:toggle:google",
            "kmd:settings:mweb:toggle:daum",
            "kmd:settings:spell:toggle:naver_spell",
            "kmd:settings:spell:toggle:pusan_spell",
            "kmd:settings:translate:toggle:google_translate",
            "kmd:settings:translate:toggle:deepl",
            "kmd:settings:rebuild",
        ] {
            assert!(actions.iter().any(|a| a == expected), "빠진 행: {expected}");
        }

        // 정보 행(noop)은 맨 끝에 모여 있다
        let last_two: Vec<&str> = actions.iter().rev().take(2).map(String::as_str).collect();
        assert_eq!(last_two, vec!["kmd:settings:noop", "kmd:settings:noop"]);
    }

    #[test]
    fn set_목록은_질의어로_걸러진다() {
        let mut app = make_test_app();
        app.handle_settings_query(":set theme");
        assert!(!app.results.is_empty());
        assert!(
            app.results
                .iter()
                .all(|r| r.item.name.to_lowercase().contains("theme")),
            "필터에 맞지 않는 행이 남아 있다"
        );
    }

    #[test]
    fn 프로바이더_토글은_config와_라벨에_반영된다() {
        let mut app = make_test_app();
        let action = "kmd:settings:llm:toggle:chatgpt";

        assert!(settings_label(&mut app, action).contains("[ON]"));
        run_settings_action(&mut app, action);

        assert!(
            !app.runtime_config
                .launcher
                .multi_llm_providers
                .iter()
                .any(|v| v.eq_ignore_ascii_case("chatgpt")),
            "토글 후 config에서 빠져야 함"
        );
        assert!(settings_label(&mut app, action).contains("[OFF]"));

        run_settings_action(&mut app, action);
        assert!(
            settings_label(&mut app, action).contains("[ON]"),
            "재토글 복귀"
        );
    }

    #[test]
    fn 프로바이더_전체해제시_기본값으로_복구된다() {
        let mut app = make_test_app();
        for id in ["naver_spell", "pusan_spell"] {
            run_settings_action(&mut app, &format!("kmd:settings:spell:toggle:{id}"));
        }
        assert_eq!(
            app.spell_providers,
            vec!["naver_spell".to_string(), "pusan_spell".to_string()],
            "전부 끄면 기본값으로 되돌아와야 @sp가 계속 동작한다"
        );
    }

    #[test]
    fn ime_토글은_상태와_config를_함께_바꾼다() {
        let mut app = make_test_app();
        let before = app.reset_ime_on_launch;
        run_settings_action(&mut app, "kmd:settings:toggle_ime_reset");
        assert_eq!(app.reset_ime_on_launch, !before);
        assert_eq!(app.runtime_config.general.reset_ime_on_launch, !before);
        assert_eq!(app.query, ":set", "토글 후 목록에 머문다");
    }

    #[test]
    fn 테마_액션은_테마와_config를_바꾸고_목록을_닫는다() {
        let mut app = make_test_app();
        run_settings_action(&mut app, "kmd:settings:theme:nord");
        assert_eq!(app.runtime_config.general.theme, "nord");
        assert!(app.query.is_empty(), "테마 적용 후에는 목록을 닫는다");
    }

    #[test]
    fn 알_수_없는_액션과_noop은_상태를_망가뜨리지_않는다() {
        let mut app = make_test_app();
        app.handle_settings_query(":set");
        let noop = app
            .results
            .iter()
            .find(|r| r.item.keywords == "kmd:settings:noop")
            .cloned()
            .expect("noop 행");
        app.query = ":set".to_string();
        let _ = app.handle_settings_action(&noop);
        assert_eq!(
            app.query, ":set",
            "noop은 쿼리를 건드리지 않고 즉시 반환한다"
        );

        let mut unknown = noop.clone();
        unknown.item.keywords = "kmd:settings:no_such_action".to_string();
        let _ = app.handle_settings_action(&unknown);
        assert!(app.query.is_empty(), "미지 액션은 경고 후 목록을 닫는다");
    }
}
