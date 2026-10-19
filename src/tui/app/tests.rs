use super::execute::start_quick_action;
use super::search::{handle_help_query, handle_prompt_query, handle_transform_query};
use super::settings_bridge::apply_settings_action;
use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn test_state() -> AppState {
    AppState {
        query: String::new(),
        results: Vec::new(),
        selected_index: 0,
        total_items: 0,
        show_preview: false,
        preview_width_percent: 40,
        search_mode: SearchMode::Fuzzy,
        should_quit: false,
        quit_on_launch: false,
        hangul_mode: false,
        hangul_auto: false,
        composing: None,
        composer: HangulComposer::new(),
        drill_stack: Vec::new(),
        drill_path: None,
        status_message: None,
        scope_undo: None,
        settings: None,
        is_portable: false,
        use_emoji: true,
        llm_autopilot: false,
        config: kmd_core::Config::default(),
        frecency: Default::default(),
        pending_shell: None,
        cached_effective_query: String::new(),
        dirty: true,
    }
}

// ── quick action은 이벤트 루프를 막지 않는다 (REF-05) ────────────────

#[test]
fn quick_action_결과는_도착하기_전까지_기다리고_도착하면_반영한다() {
    use kmd_core::plugin::ExtensionAction;
    let mut state = test_state();
    let (tx, rx) = std::sync::mpsc::channel();
    state.pending_shell = Some(rx);

    poll_pending_shell(&mut state);
    assert!(
        state.pending_shell.is_some(),
        "아직 안 끝났으면 계속 기다린다"
    );

    tx.send(ExtensionAction::Display("boom".into())).unwrap();
    poll_pending_shell(&mut state);
    assert!(state.pending_shell.is_none());
    assert_eq!(state.status_message.as_deref(), Some("\u{274C} boom"));
}

#[test]
fn quick_action_워커가_결과없이_죽으면_실행중에_머물지_않는다() {
    let mut state = test_state();
    let (tx, rx) = std::sync::mpsc::channel::<kmd_core::plugin::ExtensionAction>();
    state.pending_shell = Some(rx);
    drop(tx); // 워커 패닉 흉내

    poll_pending_shell(&mut state);
    assert!(state.pending_shell.is_none());
    assert!(state
        .status_message
        .as_deref()
        .is_some_and(|m| m.contains("비정상 종료")));
}

#[test]
fn quick_action_실행중에는_겹쳐_시작하지_않는다() {
    let mut state = test_state();
    let (_tx, rx) = std::sync::mpsc::channel();
    state.pending_shell = Some(rx);

    let item = kmd_core::IndexItem {
        name: "x".into(),
        path: "echo hi".into(),
        kind: ItemKind::Shell,
        source: kmd_core::index::Source::Plugin,
        icon: String::new(),
        keywords: String::new(),
        icon_path: None,
    };
    start_quick_action(&mut state, item);
    assert!(state
        .status_message
        .as_deref()
        .is_some_and(|m| m.contains("아직 끝나지 않았습니다")));
}

// ── 설정 저장은 성공한 뒤에만 상태를 바꾼다 (REF-01) ────────────────
//
// 예전에는 state.config를 먼저 교체하고 dirty를 내린 다음 저장해서, 저장이
// 실패하면 파일은 옛 값·메모리는 새 값·편집창은 "저장됨"이 되는 불일치가
// 남았다.

/// 설정 모달을 열고 편집한 뒤 저장 액션까지 돌린다 — 저장된 파일을 돌려준다.
fn save_via_modal(
    state: &mut AppState,
    engine: &mut SearchEngine,
    edit: impl FnOnce(&mut kmd_core::Config),
) -> kmd_core::Config {
    let mut edited = state.config.clone();
    edit(&mut edited);
    state.settings = Some(crate::tui::settings::SettingsState::new(edited));
    if let Some(s) = state.settings.as_mut() {
        s.dirty = true;
    }
    let settings_state = state.settings.take().expect("모달");
    apply_settings_action(
        state,
        settings_state,
        SettingsAction::Save {
            needs_rebuild: false,
        },
        engine,
    );
    let dir = state.config.config_path.clone().unwrap();
    kmd_core::Config::load(dir.parent().unwrap()).expect("저장된 설정 재로드")
}

fn state_with_config_file(tag: &str) -> (AppState, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "kmd_tui_cfg_{tag}_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(kmd_core::CONFIG_FILENAME);

    let mut state = test_state();
    state.config.config_path = Some(path.clone());
    state.config.save().expect("초기 저장");
    (state, dir)
}

// ── 설정 화면의 모든 항목이 실제로 저장되는가 ───────────────────────
//
// v0.16.6은 registry_keys()만 훑어, 화면에서 편집한 11개 항목(keymap 경로·
// 각종 providers/prefixes 등)이 **저장 성공으로 표시된 채 사라졌다.**
// 저장 후 파일을 다시 읽어 확인한다.

#[test]
fn 레지스트리_밖_항목도_저장된다() {
    let (mut state, dir) = state_with_config_file("nonreg");
    let mut engine = SearchEngine::new();

    let saved = save_via_modal(&mut state, &mut engine, |c| {
        c.launcher.keymap.kanata_path = Some("/tmp/my-kanata".into());
        c.launcher.spell_providers = vec!["pusan_spell".into()];
        c.launcher.translate_prefixes = vec!["@tr2".into()];
        c.launcher.keymap.backend = "kanata".into();
    });

    assert_eq!(
        saved.launcher.keymap.kanata_path.as_deref(),
        Some(std::path::Path::new("/tmp/my-kanata")),
        "kanata 경로가 저장되지 않았다"
    );
    assert_eq!(saved.launcher.spell_providers, vec!["pusan_spell"]);
    assert_eq!(saved.launcher.translate_prefixes, vec!["@tr2"]);
    assert_eq!(saved.launcher.keymap.backend, "kanata");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 레지스트리_항목도_그대로_저장된다() {
    let (mut state, dir) = state_with_config_file("reg");
    let mut engine = SearchEngine::new();

    let saved = save_via_modal(&mut state, &mut engine, |c| {
        c.general.render_fps = 47;
        c.launcher.max_results = 123;
    });

    assert_eq!(saved.general.render_fps, 47);
    assert_eq!(saved.launcher.max_results, 123);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 저장은_다른_곳의_변경을_되돌리지_않는다() {
    let (mut state, dir) = state_with_config_file("concurrent");
    let mut engine = SearchEngine::new();
    let path = state.config.config_path.clone().unwrap();

    // 모달을 여는 사이 다른 곳에서 테마 변경
    kmd_core::Config::update_and_save(&path, |c| c.general.theme = "nord".into()).unwrap();

    let saved = save_via_modal(&mut state, &mut engine, |c| c.general.render_fps = 51);

    assert_eq!(saved.general.render_fps, 51, "내 변경은 반영된다");
    assert_eq!(saved.general.theme, "nord", "남의 변경을 덮지 않는다");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 저장_실패시_메모리_설정과_편집상태가_그대로다() {
    let mut state = test_state();
    let mut engine = SearchEngine::new();

    // 저장 실패를 확실히 유도한다.
    //
    // "없는 디렉터리" 경로로는 부족하다 — Config::save()가 부모를
    // create_dir_all로 만들기 때문에, 권한만 있으면 저장이 성공한다
    // (Windows CI에서 실제로 성공해 이 테스트가 깨졌다).
    // 대신 **일반 파일을 부모로** 지정한다: 파일 아래에는 디렉터리를
    // 만들 수 없으므로 OS·권한과 무관하게 실패한다.
    let tmp_dir = std::env::temp_dir().join(format!(
        "kmd_save_fail_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).expect("테스트 디렉터리 생성");
    let blocker = tmp_dir.join("parent-is-a-file");
    std::fs::write(&blocker, b"not a directory").expect("차단 파일 생성");
    let unwritable = blocker.join("config.toml");

    let before_fps = state.config.general.render_fps;

    let mut edited = kmd_core::Config::default();
    edited.general.render_fps = before_fps + 17;
    edited.config_path = Some(unwritable);

    // 전제 검증: 이 경로가 정말 저장 불가여야 이 테스트가 의미를 갖는다.
    // (예전 버전은 "없는 디렉터리"를 썼는데 save()가 부모를 만들어 성공해
    // 버렸고, 아무것도 검증하지 못한 채 통과/실패를 오갔다.)
    assert!(
        edited.save().is_err(),
        "테스트 전제가 깨졌다 — 이 경로에 저장이 성공해 버린다"
    );

    state.settings = Some(crate::tui::settings::SettingsState::new(edited));
    if let Some(s) = state.settings.as_mut() {
        s.dirty = true;
    }

    let settings_state = state.settings.take().expect("모달 준비됨");
    apply_settings_action(
        &mut state,
        settings_state,
        SettingsAction::Save {
            needs_rebuild: false,
        },
        &mut engine,
    );

    assert_eq!(
        state.config.general.render_fps, before_fps,
        "저장 실패 시 메모리 설정은 그대로여야 한다"
    );
    let settings = state.settings.as_ref().expect("모달이 열린 채 남는다");
    assert!(settings.dirty, "저장 안 됐으므로 dirty가 유지되어야 한다");
    assert!(
        state
            .status_message
            .as_deref()
            .unwrap_or_default()
            .contains("Save failed"),
        "실패를 알려야 한다: {:?}",
        state.status_message
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

// ── :t 는 검색 중에 브라우저를 열지 않는다 ──────────────────────────
//
// 예전에는 검색 핸들러가 곧바로 URL을 열어, 타이핑하는 동안 키를 누를 때마다
// 탭이 열렸다. 이제 실행 항목 하나만 세우고 실제 열기는 Enter에서 한다.

#[test]
fn 변환질의는_검색중_실행되지_않고_실행항목을_세운다() {
    let mut state = test_state();
    state.config.launcher.spell_providers = vec!["naver_spell".into()];
    handle_transform_query(":t spell 안녕하세요", &mut state);

    assert_eq!(state.results.len(), 1, "실행 항목 하나만 선다");
    assert_eq!(
        state.results[0].item.keywords,
        kmd_core::transform::TRANSFORM_RUN_MARKER
    );
    assert!(
        state.results[0].item.path.contains("안녕하세요"),
        "무엇을 보낼지 보여준다: {}",
        state.results[0].item.path
    );
    assert!(
        state.status_message.is_none(),
        "검색 단계에서는 '실행됨' 메시지가 뜨지 않는다"
    );
}

#[test]
fn 변환질의_타이핑_도중에도_항목만_바뀐다() {
    let mut state = test_state();
    state.config.launcher.spell_providers = vec!["naver_spell".into()];
    // "안", "안녕", "안녕하" … 한 글자씩 늘려도 매번 항목 하나뿐이어야 한다
    for partial in ["안", "안녕", "안녕하"] {
        handle_transform_query(&format!(":t spell {partial}"), &mut state);
        assert_eq!(state.results.len(), 1, "'{partial}'에서 결과가 늘어남");
    }
}

#[test]
fn 변환_도움말은_그대로_유지된다() {
    let mut state = test_state();
    handle_transform_query(":t", &mut state);
    assert!(
        state.results.len() > 1,
        ":t 만 입력하면 도움말 목록이 나온다"
    );
}

// ── config 파생 필드 동기화 ─────────────────────────────────────────
//
// 이 필드들은 config의 캐시다. 예전에는 저장 경로에서 12줄을 손으로
// 재대입해 필드가 늘 때 누락되기 쉬웠다 — 이제 sync_config_mirrors 한
// 곳이며, 이 테스트가 "config를 고치면 전부 따라온다"를 지킨다.
#[test]
fn config_변경은_파생_필드에_모두_반영된다() {
    let mut state = test_state();

    state.config.general.show_preview = true;
    state.config.general.preview_width_percent = 55;
    state.config.launcher.quit_on_launch = true;
    state.config.launcher.llm_autopilot = true;

    state.sync_config_mirrors();

    assert!(state.show_preview);
    assert_eq!(state.preview_width_percent, 55);
    assert!(state.quit_on_launch);
    assert!(state.llm_autopilot);
}

#[test]
fn 동기화는_기본_설정과도_일치한다() {
    let mut state = test_state();
    state.sync_config_mirrors();
    let cfg = kmd_core::Config::default();
    assert_eq!(state.show_preview, cfg.general.show_preview);
    assert_eq!(
        state.preview_width_percent,
        cfg.general.preview_width_percent
    );
}

#[test]
fn 설정_저장은_ctrl_p로_바꾼_미리보기를_되돌리지_않는다() {
    let mut state = test_state();
    state.sync_config_mirrors();
    state.show_preview = !state.config.general.show_preview; // Ctrl+P
    let toggled = state.show_preview;

    let mut saved = state.config.clone();
    saved.launcher.quit_on_launch = !saved.launcher.quit_on_launch; // 무관한 설정 저장
    state.replace_config(saved);
    assert_eq!(state.show_preview, toggled);

    // 설정에서 show_preview 자체를 바꿨다면 그 값을 따른다
    let mut saved = state.config.clone();
    saved.general.show_preview = !saved.general.show_preview;
    let wanted = saved.general.show_preview;
    state.replace_config(saved);
    assert_eq!(state.show_preview, wanted);
}

fn type_str(state: &mut AppState, engine: &mut SearchEngine, s: &str) {
    for c in s.chars() {
        handle_key(
            state,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            engine,
            None,
        );
    }
}

// ── 실행 분기 (execute_selected) ────────────────────────────────────
//
// 실행 경로는 상태를 크게 흔드는데(쿼리 교체, 모달 열기, 드릴 진입)
// 지금까지 검증이 없었다. **부작용이 상태 안에서 끝나는 분기만** 다룬다 —
// 셸 실행·클립보드·브라우저 열기·config 저장은 테스트에서 건드리지 않는다.

fn sysitem(name: &str, path: &str, keywords: &str) -> SearchResult {
    SearchResult {
        item: kmd_core::IndexItem {
            name: name.to_string(),
            path: path.to_string(),
            kind: ItemKind::SystemCommand,
            source: kmd_core::index::Source::Plugin,
            icon: String::new(),
            keywords: keywords.to_string(),
            icon_path: None,
        },
        score: 0,
    }
}

/// 선택 항목 하나를 놓고 실행한다.
fn run_selected(state: &mut AppState, engine: &mut SearchEngine, item: SearchResult) {
    state.results = vec![item];
    state.selected_index = 0;
    execute_selected(state, engine, None);
}

#[test]
fn 웹_브라우징_가상_항목을_실행하면_해당_모드로_전환한다() {
    // 예전 TUI는 이 항목을 action::execute로 넘겨 "URL이 없습니다" 오류가 났다
    let mut engine = SearchEngine::new();
    let mut state = test_state();
    let mut item = sysitem("@llm", "여러 LLM에 동시에 질문", "kmd:web_hint:multi_llm");
    item.item.kind = ItemKind::WebSearch;
    run_selected(&mut state, &mut engine, item);
    assert_eq!(state.query, "@llm ");
    assert!(state.status_message.is_none(), "{:?}", state.status_message);
}

#[test]
fn prompt_입력_중에는_설정이_바뀌지_않는다() {
    let mut state = test_state();
    state.config.launcher.prompt_templates = vec![
        kmd_core::config::PromptTemplate {
            name: "a".into(),
            body: "A".into(),
        },
        kmd_core::config::PromptTemplate {
            name: "ab".into(),
            body: "AB".into(),
        },
    ];
    for q in [":prompt rm", ":prompt rm a", ":prompt rm ab"] {
        handle_prompt_query(q, &mut state);
    }
    assert_eq!(state.config.launcher.prompt_templates.len(), 2);
    assert_eq!(
        state.results[0].item.keywords,
        kmd_core::prompt::PROMPT_COMMIT_MARKER,
        "확정은 Enter에서"
    );
}

#[test]
fn 미지_명령_안내를_실행하면_help로_이동한다() {
    let mut engine = SearchEngine::new();
    let mut state = test_state();
    run_selected(
        &mut state,
        &mut engine,
        sysitem("안내", "", "kmd:unknown_cmd"),
    );

    assert_eq!(state.query, ":help");
    assert!(!state.results.is_empty(), "도움말 결과가 채워져야 한다");
}

#[test]
fn bang_힌트를_실행하면_웹검색_쿼리로_전환된다() {
    let mut engine = SearchEngine::new();
    let mut state = test_state();
    run_selected(
        &mut state,
        &mut engine,
        sysitem("웹으로 검색", "@g rust", "kmd:bang_hint:g"),
    );

    assert_eq!(state.query, "@g rust", "항목의 path가 새 쿼리가 된다");
}

#[test]
fn 설정_항목을_실행하면_모달이_열린다() {
    let mut engine = SearchEngine::new();
    let mut state = test_state();
    assert!(state.settings.is_none());

    run_selected(
        &mut state,
        &mut engine,
        sysitem("설정", "", "kmd:tui:open_settings"),
    );
    assert!(state.settings.is_some(), "설정 모달이 열려야 한다");
}

#[test]
fn 도움말_항목을_실행하면_시작_쿼리로_전환된다() {
    let mut engine = SearchEngine::new();
    let mut state = test_state();

    // 공용 도움말 목록에서 시작 쿼리가 있는 항목 하나를 고른다
    let help = kmd_core::query_prefix::help_items(false);
    let item = help
        .into_iter()
        .find(|i| kmd_core::query_prefix::help_query_seed(&i.name).is_some())
        .expect("시작 쿼리가 있는 도움말 항목");
    let seed = kmd_core::query_prefix::help_query_seed(&item.name)
        .unwrap()
        .to_string();

    run_selected(&mut state, &mut engine, SearchResult { item, score: 0 });
    assert_eq!(state.query, seed);
}

// ── 드릴다운 진입/복귀 ──────────────────────────────────────────────

#[test]
fn 드릴다운_진입후_복귀하면_이전_상태가_그대로_돌아온다() {
    let mut state = test_state();

    // 이 저장소의 docs/ 를 대상으로 실제 디렉터리 나열을 태운다
    let dir = std::path::PathBuf::from("docs");
    assert!(dir.is_dir(), "테스트 전제: docs/ 존재");

    state.query = "원래쿼리".to_string();
    state.results = vec![SearchResult {
        item: kmd_core::IndexItem {
            name: "docs".to_string(),
            path: dir.to_string_lossy().to_string(),
            kind: ItemKind::Directory,
            source: kmd_core::index::Source::FileProvider,
            icon: String::new(),
            keywords: String::new(),
            icon_path: None,
        },
        score: 0,
    }];
    state.selected_index = 0;

    drill_into_folder(&mut state);
    assert_eq!(
        state.drill_path.as_deref(),
        Some(dir.as_path()),
        "드릴 경로 설정"
    );
    assert!(state.query.is_empty(), "드릴 진입 시 쿼리는 비운다");
    assert!(!state.results.is_empty(), "디렉터리 내용이 채워져야 한다");

    drill_back(&mut state);
    assert!(state.drill_path.is_none(), "드릴에서 빠져나왔다");
    assert_eq!(state.query, "원래쿼리", "이전 쿼리 복원");
    assert_eq!(state.results.len(), 1, "이전 결과 복원");
}

#[test]
fn 디렉터리가_아니면_드릴다운하지_않는다() {
    let mut state = test_state();
    state.results = vec![sysitem("파일아님", "docs", "")];
    state.selected_index = 0;

    drill_into_folder(&mut state);
    assert!(
        state.drill_path.is_none(),
        "SystemCommand 항목은 드릴 대상이 아니다"
    );
    assert!(state.drill_stack.is_empty(), "스택도 쌓이면 안 된다");
}

// ── 프리픽스 디스패치 테이블 ────────────────────────────────────────
//
// update_search의 14-arm match가 각 프리픽스를 **자기 핸들러로** 보내는지
// 고정한다. arm 하나를 잘못 연결하거나 프리픽스 판정이 바뀌면 사용자는
// "엉뚱한 결과가 나온다"로만 겪고 컴파일러는 아무 말도 안 한다.

fn results_for(query: &str) -> AppState {
    let mut engine = SearchEngine::new();
    let mut state = test_state();
    type_str(&mut state, &mut engine, query);
    state
}

#[test]
fn calc_프리픽스는_계산_결과로_간다() {
    let st = results_for(":calc 2+3");
    assert!(
        st.results.iter().any(|r| r.item.name.contains('5')),
        "2+3 → 5 가 없다: {:?}",
        st.results.iter().map(|r| &r.item.name).collect::<Vec<_>>()
    );
}

#[test]
fn emoji_프리픽스는_이모지_결과로_간다() {
    // 주의: 별칭 뒤 공백부터 내장 한글 조합이 자동으로 켜지므로(§emoji_keyword_started)
    // 여기서 "heart"를 타이핑하면 자모로 조합된다. 라우팅만 확인하려면
    // 공백 없는 상태를 본다.
    let st = results_for(":emoji");
    assert!(
        !st.results.is_empty(),
        "이모지 핸들러로 라우팅되어 목록이 나와야 한다"
    );
}

#[test]
fn emoji_키워드는_한글_조합으로_입력된다() {
    // `:e ` 이후 자동 한글 조합이 켜지므로 로마자 타이핑은 자모가 된다.
    // 한국어 키워드로 이모지를 찾는 것이 이 모드의 의도다.
    let mut engine = SearchEngine::new();
    let mut state = test_state();
    type_str(&mut state, &mut engine, "/e ");
    assert!(state.hangul_mode, "공백 후 한글 조합 활성");

    type_str(&mut state, &mut engine, "gkdns"); // → 하늘
    assert!(
        state.effective_query().contains('하') || state.effective_query().contains('느'),
        "한글로 조합돼야 한다: {}",
        state.effective_query()
    );
}

#[test]
fn shell_프리픽스는_contains_모드로_전환된다() {
    let st = results_for("!echo");
    assert!(!st.results.is_empty());
    assert_eq!(st.search_mode_label(), SearchMode::Contains.label());
}

#[test]
fn version_프리픽스는_버전을_보여준다() {
    let st = results_for(":version");
    let names: Vec<_> = st.results.iter().map(|r| r.item.name.clone()).collect();
    assert!(
        names.iter().any(|n| n.contains(env!("CARGO_PKG_VERSION"))),
        "버전 문자열이 없다: {names:?}"
    );
}

#[test]
fn help_keys_keymap_프리픽스가_각각_결과를_낸다() {
    for q in [":help", ":keys", ":keymap"] {
        assert!(
            !results_for(q).results.is_empty(),
            "'{q}' 가 빈 결과 — 디스패치가 끊겼을 수 있다"
        );
    }
}

#[test]
fn 미지원_콜론_명령은_안내를_최상단에_붙인다() {
    let st = results_for(":이런명령은없다");
    assert!(
        !st.results.is_empty(),
        "일반 검색 폴스루 + 안내가 있어야 한다"
    );
}

#[test]
fn 프리픽스만_입력한_중간_상태에서_패닉하지_않는다() {
    // 사용자는 ":calc"를 치기까지 ":", ":c", ":ca" ... 를 모두 거친다
    for q in [":", ":c", ":ca", ":cal", ":calc", ":e", "!", ">", "@"] {
        let _ = results_for(q);
    }
}

// ── 선택 상태 전이 ──────────────────────────────────────────────────

#[test]
fn 선택_이동은_결과_범위를_벗어나지_않는다() {
    let mut engine = SearchEngine::new();
    let mut state = test_state();
    type_str(&mut state, &mut engine, ":help");
    let n = state.results.len();
    assert!(n >= 2, "이동을 검증하려면 결과가 2개 이상이어야 한다");

    let down = |st: &mut AppState, e: &mut SearchEngine| {
        handle_key(
            st,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            e,
            None,
        )
    };
    let up = |st: &mut AppState, e: &mut SearchEngine| {
        handle_key(st, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), e, None)
    };

    // 위로: 0에서 더 못 올라간다
    up(&mut state, &mut engine);
    assert_eq!(state.selected_index, 0, "맨 위에서 Up은 무동작");

    down(&mut state, &mut engine);
    assert_eq!(state.selected_index, 1);

    // 아래로: 마지막을 넘지 않는다
    for _ in 0..(n + 5) {
        down(&mut state, &mut engine);
    }
    assert_eq!(state.selected_index, n - 1, "맨 아래에서 Down은 무동작");
}

#[test]
fn 쿼리가_바뀌면_선택이_처음으로_돌아간다() {
    let mut engine = SearchEngine::new();
    let mut state = test_state();
    type_str(&mut state, &mut engine, ":help");
    handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        &mut engine,
        None,
    );
    assert_eq!(state.selected_index, 1);

    // 한 글자 더 입력 → 결과가 다시 만들어지므로 선택은 0으로
    type_str(&mut state, &mut engine, "x");
    assert_eq!(state.selected_index, 0, "새 결과에서는 첫 항목 선택");
}

#[test]
fn 입력하면_드릴다운에서_빠져나온다() {
    let mut engine = SearchEngine::new();
    let mut state = test_state();
    state.drill_path = Some(std::path::PathBuf::from("."));

    type_str(&mut state, &mut engine, "a");
    assert!(state.drill_path.is_none(), "타이핑은 드릴 모드를 종료한다");
}

#[test]
fn slash_exit_입력중_한글_조합_안됨() {
    // 회귀: "/e"까지 입력된 순간 이모지 프리픽스로 오인해 한글 조합이
    // 켜지면서 x가 'ㅌ'로 바뀌던 버그 (/exit → /eㅌit)
    let mut engine = SearchEngine::new();
    let mut state = test_state();
    type_str(&mut state, &mut engine, "/exit");
    assert_eq!(state.query, "/exit");
    assert!(!state.hangul_mode);
    assert!(!state.hangul_auto);
}

#[test]
fn emoji_별칭만으로는_한글_조합_비활성() {
    let mut engine = SearchEngine::new();
    let mut state = test_state();
    type_str(&mut state, &mut engine, "/e");
    assert!(!state.hangul_mode, "공백 전에는 한글 조합이 켜지면 안 됨");
}

#[test]
fn emoji_키워드_공백_후_자동_한글_조합() {
    let mut engine = SearchEngine::new();
    let mut state = test_state();
    type_str(&mut state, &mut engine, "/e ");
    assert!(state.hangul_mode && state.hangul_auto);

    // "rk" → ㄱ+ㅏ 조합 중 '가'
    type_str(&mut state, &mut engine, "rk");
    assert!(state.effective_query().ends_with('가'));
}

#[test]
fn emoji_키워드_이탈시_자동_한글_해제() {
    let mut engine = SearchEngine::new();
    let mut state = test_state();
    type_str(&mut state, &mut engine, "/e ");
    assert!(state.hangul_auto);

    // 백스페이스로 공백 제거 → ":e"로 복귀 → 자동 모드 해제
    handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        &mut engine,
        None,
    );
    assert!(!state.hangul_mode);
    assert!(!state.hangul_auto);
}

#[test]
fn tui_도움말은_클립보드_히스토리_항목을_숨긴다() {
    // 회귀 방지: 공용 COMMANDS의 클립보드 항목을 TUI 도움말에 노출하면
    // 선택 시 시작 쿼리 `;`가 일반 파일 검색으로 흘러가 안내와 다르게 동작한다.
    let mut state = test_state();
    handle_help_query(&mut state);
    assert!(
        !state.results.is_empty(),
        "다른 도움말 항목은 그대로 보여야 함"
    );
    assert!(
        state.results.iter().all(|r| {
            !matches!(
                kmd_core::query_prefix::help_query_seed(&r.item.name),
                Some(seed)
                    if kmd_core::query_prefix::prefix_of(seed) == QueryPrefix::Clipboard
            )
        }),
        "클립보드 도움말 항목이 TUI에 노출됨"
    );
}
