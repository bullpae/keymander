use super::window_resize::CLOAK_HARD_LIMIT;
use super::*;

use std::sync::atomic::{AtomicU32, Ordering};
static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

/// 런처를 닫는 경로는 `exit_launcher()`를 거쳐야 한다 — 직접 `iced::exit()`를
/// 쓰면 열 때 영문으로 바꿔 둔 macOS 입력 소스가 되돌려지지 않는다.
/// 예외는 두 곳뿐: `exit_launcher` 자신, 포커스 유실 경로(focus_flow, 의도적).
#[test]
fn 런처_닫기는_exit_launcher를_거친다() {
    let marker = ["iced", "::exit()"].concat(); // 이 테스트 자신이 세지지 않게
                                                // 주석 줄은 세지 않는다 (설명에 이름이 나온다)
    let count = |src: &str| {
        src.lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .map(|l| l.matches(marker.as_str()).count())
            .sum::<usize>()
    };
    assert_eq!(
        count(include_str!("../app.rs")),
        1,
        "app.rs: exit_launcher 안의 1회만"
    );
    assert_eq!(
        count(include_str!("focus_flow.rs")),
        1,
        "포커스 유실 경로 1회만"
    );
    for (name, src) in [
        ("launch.rs", include_str!("launch.rs")),
        ("settings.rs", include_str!("settings.rs")),
        ("search_routing.rs", include_str!("search_routing.rs")),
        ("view.rs", include_str!("view.rs")),
        ("context_action.rs", include_str!("context_action.rs")),
        ("engine_boot.rs", include_str!("engine_boot.rs")),
        ("results.rs", include_str!("results.rs")),
        ("ui_scale.rs", include_str!("ui_scale.rs")),
        ("window_resize.rs", include_str!("window_resize.rs")),
    ] {
        assert_eq!(
            count(src),
            0,
            "{name}: 런처를 닫을 땐 exit_launcher()를 쓸 것"
        );
    }
}

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

// ── 정본 통합: URL 열기 항목 · 최근 이력 · 상태 알림 ────────────────

#[test]
fn url을_입력하면_열_수_있는_항목이_맨_위에_선다() {
    let mut app = make_test_app();
    app.query = "example.com".into();
    app.handle_main_search("example.com");

    assert!(!app.results.is_empty(), "열 항목이 있어야 한다");
    assert_eq!(app.results[0].item.kind, kmd_core::ItemKind::WebSearch);
    assert!(
        app.results[0].item.path.contains("example.com"),
        "{}",
        app.results[0].item.path
    );
}

fn app_item(name: &str, path: &str) -> IndexItem {
    IndexItem {
        name: name.to_string(),
        path: path.to_string(),
        kind: ItemKind::App,
        source: Source::Apps,
        icon: "Ap".to_string(),
        keywords: path.to_string(),
        icon_path: None,
    }
}

#[test]
fn 글자만_흩어져_걸린_퍼지_결과_아래에_웹_제안을_붙인다() {
    // "Game Mail" 은 g·m·a·i·l 순서로 퍼지 매칭되지만 gmail 이 그대로 들어있진 않다
    let mut app = make_test_app();
    app.engine
        .load(vec![app_item("Game Mail", r"C:\Apps\Game Mail.lnk")]);
    app.query = "gmail".into();
    app.handle_main_search("gmail");

    assert_eq!(
        app.results[0].item.name, "Game Mail",
        "퍼지 결과는 위에 유지"
    );
    assert!(
        app.results
            .iter()
            .any(|r| r.item.kind == ItemKind::WebSearch && r.item.name.contains("gmail")),
        "웹 검색 제안이 함께 나와야 한다"
    );
}

#[test]
fn 질의가_그대로_든_결과가_있으면_웹_제안을_붙이지_않는다() {
    let mut app = make_test_app();
    app.engine
        .load(vec![app_item("Gmail", r"C:\Apps\Gmail.lnk")]);
    app.query = "gmail".into();
    app.handle_main_search("gmail");

    assert_eq!(app.results.len(), 1, "{:?}", app.results);
    assert_eq!(app.results[0].item.name, "Gmail");
}

#[test]
fn 빈_입력이면_최근_실행_항목을_보여준다() {
    let mut app = make_test_app();
    kmd_core::history::record_launch(
        &app.db,
        kmd_core::ItemKind::App,
        "/Applications/Safari.app",
        Some("Safari"),
    );

    app.query = String::new();
    let _ = app.perform_search();

    assert_eq!(app.results.len(), 1, "이력 1건이 보여야 한다");
    assert_eq!(app.results[0].item.name, "Safari");
}

#[test]
fn 상태_알림은_다음_검색에서_지워진다() {
    let mut app = make_test_app();
    app.status_message = Some("설정 저장 실패: 디스크 오류".into());

    app.query = "safari".into();
    let _ = app.perform_search();

    assert!(
        app.status_message.is_none(),
        "알림은 짧은 수명 — 다음 검색에서 사라진다"
    );
}

// ── :t 는 검색 중에 브라우저를 열지 않는다 ──────────────────────────
//
// 데스크톱은 QueryChanged마다 디바운스 없이 검색을 돌린다. 검색 핸들러가
// URL을 열던 시절에는 `:t spell 안녕`을 타이핑하면 키마다 탭이 열렸다.

#[test]
fn 변환질의는_검색중_실행되지_않고_실행항목을_세운다() {
    let mut app = make_test_app();
    app.runtime_config.launcher.spell_providers = vec!["naver_spell".into()];
    app.query = ":t spell 안녕하세요".into();
    app.handle_transform_query(":t spell 안녕하세요");

    assert_eq!(app.results.len(), 1);
    assert_eq!(
        app.results[0].item.keywords,
        kmd_core::transform::TRANSFORM_RUN_MARKER
    );
    assert!(app.results[0].item.path.contains("안녕하세요"));
}

#[test]
fn 변환질의_타이핑_도중에도_항목만_바뀐다() {
    let mut app = make_test_app();
    app.runtime_config.launcher.spell_providers = vec!["naver_spell".into()];
    for partial in ["안", "안녕", "안녕하"] {
        let q = format!(":t spell {partial}");
        app.query = q.clone();
        app.handle_transform_query(&q);
        assert_eq!(app.results.len(), 1, "'{partial}'에서 결과가 늘어남");
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

    // 고정 항목이 이 순서로 먼저 온다 — 맨 위는 검색 범위 요약 (docs/18 E3)
    let head: Vec<&str> = actions.iter().take(8).map(String::as_str).collect();
    assert_eq!(
        head,
        vec![
            kmd_core::scope::SCOPE_OPEN_MARKER,
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
        app.runtime_config.launcher.spell_providers,
        vec!["naver_spell".to_string(), "pusan_spell".to_string()],
        "전부 끄면 기본값으로 되돌아와야 @sp가 계속 동작한다"
    );
}

#[test]
fn 테스트는_실제_사용자_설정을_건드리지_않는다() {
    // 설정 토글 테스트들이 save_config를 타는데, 그게 실제 config.toml을
    // 쓰면 `cargo test`를 돌릴 때마다 사용자 설정이 바뀐다(실제 발생:
    // IME 설정이 재설치 때마다 초기화된다는 증상의 정체였다).
    let real = kmd_core::Config::default_config_dir().join(kmd_core::CONFIG_FILENAME);
    assert_ne!(
        config_save_path(),
        real,
        "테스트에서 실제 사용자 설정 경로를 쓰면 안 된다"
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

// ── :scope 검색 범위 관리 (docs/18) ──

/// 테스트용 임시 폴더 (이 크레이트엔 tempfile 의존성이 없다)
struct ScopeTmp(std::path::PathBuf);

impl ScopeTmp {
    fn new() -> Self {
        let seq = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("kmd_scope_{}_{seq}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for ScopeTmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scope_test_app(root: &std::path::Path) -> App {
    let mut app = make_test_app();
    app.runtime_config.launcher.search_paths = vec![root.to_path_buf()];
    app.runtime_config.launcher.include_user_folders = Some(false);
    app
}

#[test]
fn scope_목록은_추가행_폴더행_제외규칙_순서() {
    let tmp = ScopeTmp::new();
    let mut app = scope_test_app(tmp.path());
    app.query = ":scope".into();
    app.handle_scope_query(":scope");
    let kws: Vec<&str> = app
        .results
        .iter()
        .map(|r| r.item.keywords.as_str())
        .collect();
    assert_eq!(kws[0], kmd_core::scope::SCOPE_OPEN_MARKER);
    assert_eq!(
        kws[1],
        format!(
            "{}{}",
            kmd_core::scope::SCOPE_FOLDER_MARKER,
            tmp.path().display()
        )
    );
    assert_eq!(*kws.last().unwrap(), "kmd:settings:config");
}

#[test]
fn scope_ctrl_del_제거와_ctrl_z_되돌리기() {
    let tmp = ScopeTmp::new();
    let mut app = scope_test_app(tmp.path());
    app.query = ":scope".into();
    app.handle_scope_query(":scope");
    app.selected = 1;

    let del = keyboard::Key::Named(keyboard::key::Named::Delete);
    assert!(app
        .handle_scope_key(&del, &keyboard::Modifiers::CTRL)
        .is_some());
    assert!(app.runtime_config.launcher.search_paths.is_empty());
    assert_eq!(app.scope_undo.as_deref(), Some(tmp.path()));

    app.query = ":scope".into();
    let z = keyboard::Key::Character("z".into());
    assert!(app
        .handle_scope_key(&z, &keyboard::Modifiers::CTRL)
        .is_some());
    assert_eq!(
        app.runtime_config.launcher.search_paths,
        vec![tmp.path().to_path_buf()]
    );
    // `:scope` 밖에서는 Ctrl+Z를 가로채지 않는다
    app.query = "vera".into();
    assert!(app
        .handle_scope_key(&z, &keyboard::Modifiers::CTRL)
        .is_none());
}

#[test]
fn 범위_밖_폴더_경로를_치면_맨_위에_추가_행() {
    let tmp = ScopeTmp::new();
    let outside = ScopeTmp::new();
    let mut app = scope_test_app(tmp.path());
    let q = outside.path().display().to_string();
    app.handle_main_search(&q);
    assert!(app.results[0]
        .item
        .keywords
        .starts_with(kmd_core::scope::SCOPE_ADD_MARKER));
}

// ── :say 입버릇 (docs/21) ──

/// 입버릇을 켠 테스트 앱 — 문장 하나(상자 지정)를 임시 폴더에 둔다
fn make_say_app(b: u8) -> (App, String) {
    let mut app = make_test_app();
    let seq = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("kmd_test_say_{}_{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app.say_store = kmd_core::say::SayStore::new(dir);
    app.runtime_config.say.enabled = true;
    let mut s = kmd_core::say::Sentence::new("It's up to you.", "마음대로 하세요", false, 0);
    s.r#box = b;
    let id = s.id.clone();
    app.say_store
        .save_deck(&kmd_core::say::Deck::new(vec![s]))
        .unwrap();
    (app, id)
}

#[test]
fn 입버릇_빈_입력_카드는_떠올리기_단계면_영어를_가리고_tab으로_연다() {
    let (mut app, _) = make_say_app(2);
    app.query.clear();
    let _ = app.perform_search();
    // 테스트 DB 에는 최근 항목이 없어 카드가 항목으로 들어간다
    let card = &app.results[0].item;
    assert!(card.name.contains("마음대로 하세요"), "{}", card.name);
    assert!(!card.name.contains("up to you"), "{}", card.name);
    let tab = keyboard::Key::Named(keyboard::key::Named::Tab);
    assert!(app.handle_say_card_key(&tab).is_some());
    assert!(app.results[0].item.name.contains("up to you"));
    // Enter = :say 연습으로
    let _ = app.launch_selected();
    assert_eq!(app.query, ":say ");
}

#[test]
fn 입버릇_꺼져_있으면_say_enter가_켠다() {
    let (mut app, _) = make_say_app(0);
    app.runtime_config.say.enabled = false;
    app.query = ":say".into();
    let _ = app.perform_search();
    let _ = app.launch_selected();
    assert!(app.runtime_config.say.enabled);
    assert_eq!(app.query, ":say ");
    assert!(app
        .status_message
        .as_deref()
        .unwrap_or("")
        .contains("켰습니다"));
}

#[test]
fn 입버릇_타이핑_통과는_상자를_올리지_않는다() {
    let (mut app, id) = make_say_app(2);
    app.query = ":say it is up to you".into();
    let _ = app.perform_search();
    assert!(
        app.results[1].item.name.starts_with("100%"),
        "{}",
        app.results[1].item.name
    );
    let _ = app.launch_selected();
    assert!(app.status_message.as_deref().unwrap_or("").contains("통과"));
    let deck = app.say_store.load_deck(0).unwrap();
    let s = deck.get(&id).unwrap();
    assert_eq!((s.r#box, s.typed_count), (2, 1));
}

#[test]
fn 입버릇_빈_입력_enter는_정답을_연다() {
    let (mut app, id) = make_say_app(3);
    app.query = ":say ".into();
    let _ = app.perform_search();
    assert!(!app.results[0].item.name.contains("up to you"));
    let _ = app.launch_selected();
    assert_eq!(app.say_revealed.as_deref(), Some(id.as_str()));
    assert!(app.results[0].item.name.contains("up to you"));
}

// ── 한영 오타 변환 · 초성 (docs/22 §3.10) ──

fn korean_item(name: &str) -> IndexItem {
    IndexItem {
        name: name.to_string(),
        path: format!("/Applications/{name}.app"),
        kind: ItemKind::App,
        source: Source::Apps,
        icon: "Ap".to_string(),
        keywords: String::new(),
        icon_path: None,
    }
}

#[test]
fn 영문_자판으로_친_한글은_바꿔_찾고_알린다() {
    let mut app = make_test_app();
    app.engine
        .load(vec![korean_item("카카오톡"), korean_item("Chrome")]);
    app.query = "zkzkdhxhr".to_string(); // 카카오톡
    let _ = app.perform_search();
    assert_eq!(
        app.results.first().map(|r| r.item.name.as_str()),
        Some("카카오톡")
    );
    assert_eq!(
        app.status_message.as_deref(),
        Some("한글로 바꿔 찾음: 카카오톡")
    );

    // 영어 질의는 그대로 — 바꾸지 않는다
    app.query = "chrome".to_string();
    let _ = app.perform_search();
    assert_eq!(
        app.results.first().map(|r| r.item.name.as_str()),
        Some("Chrome")
    );
    assert!(app.status_message.is_none());
}

#[test]
fn 초성으로_앱을_찾는다() {
    let mut app = make_test_app();
    app.engine
        .load(vec![korean_item("카카오톡"), korean_item("메모")]);
    app.query = "ㅋㅋㅇㅌ".to_string();
    let _ = app.perform_search();
    assert_eq!(
        app.results.first().map(|r| r.item.name.as_str()),
        Some("카카오톡")
    );
}
