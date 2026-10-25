//! keymander Desktop — GPU-accelerated Spotlight-like launcher
//!
//! A borderless, transparent, floating search window powered by iced.
//! Shares the same kmd-core search engine and portable data as the CLI.
//!
//! **Singleton toggle**: launching a second instance signals the first to quit.

// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod app_icons;
mod brand_icons;
mod engine;
mod launch_perf;
mod platform;
mod query_prefix;
mod system_icons;
mod theme;
mod window_state;

use iced::{window, Color, Point, Size};
use std::fs::{self, OpenOptions};
use std::sync::Mutex;

use crate::app::{
    collapsed_window_height, full_window_height, initial_window_height, DEFAULT_WIDTH,
};
use crate::window_state::WindowState;

fn should_print_version() -> bool {
    std::env::args()
        .skip(1)
        .any(|arg| matches!(arg.as_str(), "--version" | "-V" | "version"))
}

fn print_version() {
    println!("kmd-desktop {}", env!("CARGO_PKG_VERSION"));
    println!("kmd-core {}", kmd_core::Index::current_version());
    println!("target {}", std::env::consts::ARCH);
    println!("os {}", std::env::consts::OS);
}

/// Default position: horizontally centered, vertically at 1/3 from top.
fn default_position(win: Size, monitor: Size) -> Point {
    Point::new(
        (monitor.width - win.width) / 2.0,
        (monitor.height / 3.0).max(0.0),
    )
}

/// 바이너리에 임베드된 아이콘 PNG를 32x32 RGBA로 디코딩하여 윈도우 아이콘 생성.
fn create_icon() -> Option<window::Icon> {
    const ICON_PNG: &[u8] = include_bytes!("../../../assets/icon.png");
    let img = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png).ok()?;
    let resized = img.resize_exact(32, 32, image::imageops::FilterType::Lanczos3);
    let rgba = resized.to_rgba8();
    window::icon::from_rgba(rgba.into_raw(), 32, 32).ok()
}

/// 투명 창 사용 여부 — 부팅 시 1회 결정되어 전 모듈이 같은 값을 본다.
///
/// 투명하면 창 높이를 고정하고 빈 영역을 투명으로 두므로 **창 리사이즈가
/// 사라진다** — 리사이즈 순간 컴포지터가 이전 프레임을 새 크기로 늘려 합성하는
/// 구간이 없어져 "화면이 찢어지는" 증상이 원천적으로 발생하지 않는다.
/// 카드 라운드도 우리가 직접 그리므로 플랫폼 간 UI가 같아진다.
static WINDOW_TRANSPARENT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

pub fn window_transparent() -> bool {
    // 초기화 전(테스트 등)에는 플랫폼 기본값 — macOS/Linux 는 항상 투명 가능.
    *WINDOW_TRANSPARENT.get_or_init(|| !cfg!(target_os = "windows"))
}

/// 설정과 실행 환경을 보고 투명 창 사용 여부를 확정한다 (창 생성 전 1회).
///
/// Windows 는 DX12 HWND 스왑체인이 `alpha_modes=[Opaque]`라 그대로는 투명이
/// 불가능하고, DirectComposition 스왑체인으로 바꿔야 PreMultiplied 를 쓸 수 있다.
/// 소프트웨어 렌더러(tiny-skia)는 그 경로가 없으므로 불투명으로 되돌린다.
fn init_window_transparency(config: &kmd_core::Config) {
    let requested = !config
        .general
        .window_transparency
        .eq_ignore_ascii_case("off");
    // 긴급 킬스위치 — 배포본에서 설정 파일을 못 고치는 상황 대비.
    let killed = std::env::var_os("KMD_NO_TRANSPARENT").is_some();
    let software = matches!(
        config.general.renderer.trim().to_ascii_lowercase().as_str(),
        "software" | "tiny-skia" | "cpu"
    ) || std::env::var("ICED_BACKEND")
        .is_ok_and(|v| v.eq_ignore_ascii_case("tiny-skia"));

    let transparent = requested && !killed && !(cfg!(target_os = "windows") && software);
    let _ = WINDOW_TRANSPARENT.set(transparent);

    if !transparent {
        tracing::info!(
            "투명 창 비활성 (요청={requested}, 킬스위치={killed}, 소프트웨어렌더러={software}) \
             — 불투명 창 + 리사이즈 방식으로 동작"
        );
    }

    // Windows: wgpu 가 DirectComposition 스왑체인을 쓰도록 지정한다. 기본값
    // (DxgiFromHwnd)은 per-pixel 알파를 지원하지 않아 투명 픽셀이 검게 그려진다.
    // 창을 만들기 **전에** 적용되어야 하며, vendor/iced_wgpu 패치가 이 환경변수를
    // 읽어 wgpu 인스턴스에 전달한다.
    #[cfg(target_os = "windows")]
    if std::env::var_os("WGPU_DX12_PRESENTATION_SYSTEM").is_none() {
        std::env::set_var("WGPU_DX12_PRESENTATION_SYSTEM", "visual");
        tracing::info!("DX12 presentation = DirectComposition visual (투명 창 지원)");
    }

    // Windows: 백엔드를 DX12 로 고정한다. 위 DirectComposition 설정은 DX12 에만
    // 적용되는데, wgpu 는 Vulkan → DX12 순으로 어댑터를 모은 뒤 외장 GPU 를
    // 우선하므로 NVIDIA 외장 GPU(MX250 등)가 있으면 Vulkan 을 잡는다. NVIDIA 의
    // Windows Vulkan 서피스는 alpha_modes=[Opaque] 라 빈 영역이 검게 칠해진다.
    #[cfg(target_os = "windows")]
    if transparent && std::env::var_os("WGPU_BACKEND").is_none() {
        std::env::set_var("WGPU_BACKEND", "dx12");
        tracing::info!("wgpu backend = DX12 (Vulkan 우회 — 투명 창은 DX12 에서만 보장)");
    }
}

/// config.general.renderer → iced 렌더러 선택 환경변수.
///
/// "software"는 wgpu 어댑터 프로빙(가상 GPU 환경에서 수 초 소요)을 생략하고
/// tiny-skia로 직행한다. 사용자가 이미 ICED_BACKEND를 지정했다면 존중한다.
fn apply_renderer_preference(renderer: &str) {
    if std::env::var_os("ICED_BACKEND").is_some() {
        return;
    }
    let backend = match renderer.trim().to_ascii_lowercase().as_str() {
        "software" | "tiny-skia" | "cpu" => Some("tiny-skia"),
        "gpu" | "wgpu" => Some("wgpu"),
        _ => None, // "auto" — iced 기본 폴백 체인 (wgpu → tiny-skia)
    };
    if let Some(backend) = backend {
        std::env::set_var("ICED_BACKEND", backend);
        tracing::info!("Renderer preference: {backend} (config general.renderer)");
    }
}

fn main() -> iced::Result {
    if should_print_version() {
        print_version();
        return Ok(());
    }
    let boot_started = std::time::Instant::now();
    launch_perf::init();

    let desktop_data_dir = kmd_core::Config::default_data_dir().join("desktop");
    let log_dir = desktop_data_dir.join("logs");
    let log_path = log_dir.join("desktop.log");

    // 자사 크레이트는 info까지 기록 — 부팅 단계별 타이밍/엔진 로드 로그가
    // desktop.log에 남아야 느린 환경(VM 등) 진단이 가능하다. 키 입력 단위
    // 로그는 debug 레벨이라 여전히 제외된다.
    let make_filter = || {
        tracing_subscriber::EnvFilter::from_default_env()
            .add_directive("kmd_desktop=info".parse().unwrap())
            .add_directive("kmd_core=info".parse().unwrap())
            // 선택된 GPU 어댑터·백엔드·alpha 모드 — 투명 창이 검게 나오는
            // 환경을 로그만으로 진단하려면 필요하다.
            .add_directive("iced_wgpu::window::compositor=info".parse().unwrap())
    };

    let log_file = fs::create_dir_all(&log_dir).ok().and_then(|_| {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .ok()
    });

    // 파일 I/O를 백그라운드 스레드로 오프로드 — 키 입력마다 UI 스레드가 디스크
    // 쓰기로 멈추지 않게 한다. WorkerGuard는 main 종료 시점까지 살아 있어야
    // 버퍼에 남은 로그가 flush된다.
    let _log_guard = match log_file {
        Some(file) => {
            let (non_blocking, guard) = tracing_appender::non_blocking(file);
            tracing_subscriber::fmt()
                .with_env_filter(make_filter())
                .with_target(false)
                .with_ansi(false)
                .with_writer(non_blocking)
                .init();
            tracing::info!("로그 파일 경로: {}", log_path.display());
            Some(guard)
        }
        None => {
            tracing_subscriber::fmt()
                .with_env_filter(make_filter())
                .with_target(false)
                .init();
            tracing::warn!("파일 로그 초기화 실패: 콘솔 출력만 사용");
            None
        }
    };

    tracing::info!("Starting keymander Desktop");

    // ── Singleton toggle ──────────────────────────────────────────────────
    let data_dir = desktop_data_dir;
    let guard = match kmd_core::single_instance::acquire_or_toggle(&data_dir) {
        kmd_core::single_instance::InstanceAction::Acquired(guard) => guard,
        kmd_core::single_instance::InstanceAction::SignalledExisting => {
            tracing::info!("Signalled existing desktop instance to quit — exiting");
            return Ok(());
        }
    };

    // ── Preload config + window state ─────────────────────────────────────
    let config = engine::load_config();
    launch_perf::enable(config.launcher.keymap.stats);
    let window_state = WindowState::load();
    apply_renderer_preference(&config.general.renderer);
    // 창을 만들기 전에 투명 여부를 확정해야 한다 (창 설정·레이아웃·렌더에 모두 영향).
    init_window_transparency(&config);

    // Windows 불투명 창의 배경색 — 카드가 덮지 않는 픽셀(라운드 모서리 바깥,
    // 힌트 영역)이 검정 대신 테마 배경색으로 칠해진다.
    let opaque_bg = {
        let t = theme::from_name(&config.general.theme);
        Color {
            a: 1.0,
            ..t.background
        }
    };

    let base_width = window_state
        .width
        .unwrap_or(DEFAULT_WIDTH)
        .clamp(420.0, 1200.0);
    let win_height = full_window_height(config.general.font_size, config.general.visible_rows);
    // 창을 접는 플랫폼(Windows/Linux)에서는 검색바만 보이는 높이로 시작하고,
    // 결과가 생기면 앱이 full 높이로 리사이즈한다 (app.rs sync_window_height).
    // macOS 는 높이를 고정하므로 처음부터 full 로 띄운다 (app.rs FIXED_WINDOW_HEIGHT).
    let initial_height =
        initial_window_height(config.general.font_size, config.general.visible_rows);
    let min_height = collapsed_window_height(config.general.font_size, config.general.visible_rows)
        .min(initial_height);
    let initial_size = Size::new(base_width, initial_height);

    let position = match (window_state.x, window_state.y) {
        (Some(x), Some(y)) => window::Position::Specific(Point::new(x, y)),
        _ => window::Position::SpecificWith(default_position),
    };

    let icon = create_icon();

    let boot_data = Mutex::new(Some((guard, config, window_state)));

    tracing::info!(
        "Boot preamble done in {} ms — creating window",
        boot_started.elapsed().as_millis()
    );

    iced::application(
        move || {
            let mut lock = boot_data.lock().unwrap_or_else(|e| e.into_inner());
            let (guard, config, ws) = lock
                .take()
                .expect("iced 초기화가 두 번 이상 호출됨 — 프레임워크 버그");
            let boot = app::App::new(guard, config, ws);
            tracing::info!(
                "App state ready in {} ms since process start",
                boot_started.elapsed().as_millis()
            );
            boot
        },
        app::App::update,
        app::App::view,
    )
    .window(window::Settings {
        size: initial_size,
        decorations: false,
        transparent: window_transparent(),
        level: window::Level::AlwaysOnTop,
        position,
        resizable: true,
        visible: true,
        exit_on_close_request: true,
        min_size: Some(Size::new(420.0, min_height)),
        max_size: Some(Size::new(1600.0, win_height)),
        icon,
        ..Default::default()
    })
    .theme(app::App::theme)
    .style(move |_state, _theme| iced::theme::Style {
        background_color: if window_transparent() {
            Color::TRANSPARENT
        } else {
            opaque_bg
        },
        text_color: Color::WHITE,
    })
    .subscription(app::App::subscription)
    .antialiasing(true)
    .run()
}
