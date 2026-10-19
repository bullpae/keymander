use super::defaults::{default_search_paths, existing_unique_dirs};
use super::*;

#[test]
fn 기본_검색경로는_존재하는_폴더만_중복없이() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let got = existing_unique_dirs([
        Some(a.clone()),
        None,
        Some(dir.path().join("missing")),
        Some(b.clone()),
        Some(a.clone()), // 같은 폴더가 두 경로로 들어와도 한 번만
    ]);
    assert_eq!(got, vec![a, b]);
}

#[test]
fn 기본_검색경로에_중복이_없다() {
    let paths = default_search_paths();
    for (i, p) in paths.iter().enumerate() {
        assert!(p.is_dir(), "존재하지 않는 기본 경로: {}", p.display());
        assert!(!paths[..i].contains(p), "중복된 기본 경로: {}", p.display());
    }
}
use crate::CONFIG_FILENAME;

// ── 잘못된 값 거부 (2026-08-27) ────────────────────────────────────
//
// 예전에는 파싱 실패 시 기존 값을 유지한 채 Ok(())를 반환해서,
// `kmd config set general.render_fps abc`가 성공 메시지 + 종료코드 0을
// 내면서 값은 그대로였다. TUI는 그 결과마저 버리고 "변경됨"으로 표시했다.

// ── 설정 키 레지스트리 ─────────────────────────────────────────────

#[test]
fn 레지스트리_키는_전부_get과_set이_된다() {
    // 매크로가 양쪽을 함께 생성하므로 비대칭은 원천적으로 불가능하지만,
    // 선언 자체가 잘못돼도(오타난 필드 경로 등) 여기서 걸린다.
    let mut c = Config::default();
    for key in Config::registry_keys() {
        let v = c
            .get_value(key)
            .unwrap_or_else(|| panic!("레지스트리 키 '{key}' 를 get_value가 모른다"));
        c.set_value(key, &v)
            .unwrap_or_else(|e| panic!("레지스트리 키 '{key}' 를 set_value가 거부한다: {e}"));
        assert_eq!(
            c.get_value(key).as_deref(),
            Some(v.as_str()),
            "'{key}' 왕복 값이 달라졌다"
        );
    }
}

#[test]
fn 레지스트리_숫자_키는_잘못된_값을_거부한다() {
    let mut c = Config::default();
    // num 종류만 골라낸다 — 값이 숫자/불리언으로 파싱되는 키
    let numeric: Vec<&str> = Config::registry_keys()
        .iter()
        .copied()
        .filter(|k| {
            let v = Config::default().get_value(k).unwrap_or_default();
            v.parse::<i64>().is_ok() || v.parse::<bool>().is_ok()
        })
        .collect();
    assert!(numeric.len() >= 10, "숫자/불리언 키가 충분히 잡혀야 한다");

    for key in numeric {
        assert!(
            c.set_value(key, "완전히 잘못된 값").is_err(),
            "'{key}' 가 잘못된 값을 조용히 받아들인다"
        );
    }
}

#[test]
fn 레지스트리와_명시적_arm이_겹치지_않는다() {
    // 같은 키가 양쪽에 있으면 레지스트리가 먼저 이겨 명시적 arm이
    // 죽은 코드가 된다 — 읽는 사람이 잘못된 곳을 고치게 된다.
    // get_value/set_value 와 레지스트리 선언은 registry.rs 에 함께 있다.
    let src = include_str!("registry.rs");
    let (_, after) = src.split_once("pub fn get_value").expect("get_value");
    let (get_body, rest) = after.split_once("pub fn set_value").expect("set_value");
    for key in Config::registry_keys() {
        let needle = format!("\"{key}\" =>");
        assert!(
            !get_body.contains(&needle),
            "'{key}' 가 레지스트리와 get_value 양쪽에 있다"
        );
        assert!(
            !rest.contains(&needle),
            "'{key}' 가 레지스트리와 set_value 양쪽에 있다"
        );
    }
}

#[test]
fn 숫자_필드에_숫자가_아닌_값은_거부한다() {
    let mut c = Config::default();
    let before = c.general.render_fps;

    let err = c
        .set_value("general.render_fps", "이건숫자가아님")
        .expect_err("파싱 실패는 오류여야 한다");

    assert!(matches!(err, ConfigError::InvalidValue { .. }));
    assert_eq!(c.general.render_fps, before, "실패 시 값이 바뀌면 안 된다");
    // 메시지에 무엇이 잘못됐는지 담긴다
    let msg = err.to_string();
    assert!(msg.contains("general.render_fps"), "키 표시: {msg}");
    assert!(msg.contains("이건숫자가아님"), "입력값 표시: {msg}");
}

#[test]
fn 불리언_필드에_아무_문자열이나_받지_않는다() {
    let mut c = Config::default();
    assert!(c.set_value("general.show_preview", "yes").is_err());
    assert!(c.set_value("general.show_preview", "false").is_ok());
    assert!(!c.general.show_preview);
}

#[test]
fn 유효한_값은_정상_반영된다() {
    let mut c = Config::default();
    c.set_value("general.render_fps", "45").unwrap();
    assert_eq!(c.general.render_fps, 45);
    assert_eq!(c.get_value("general.render_fps").as_deref(), Some("45"));
}

// ── 저장: 주석 보존 + 원자적 교체 ─────────────────────────────────

fn temp_config_path(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("kmd_cfg_test_{tag}_{}.toml", std::process::id()));
    p
}

// ── 편집 가능 키 전수 왕복 ─────────────────────────────────────────
//
// v0.16.6에서 TUI 저장이 registry_keys()만 훑어, 명시적 arm으로 처리되는
// 11개 키(everything_path·keymap.*·각종 providers/prefixes)가 화면에서
// 편집해도 저장되지 않았다. all_editable_keys()가 정본임을 여기서 지킨다.

#[test]
fn 모든_편집키가_왕복한다() {
    let mut c = Config::default();
    for key in Config::all_editable_keys() {
        let before = c
            .get_value(key)
            .unwrap_or_else(|| panic!("get_value 없음: {key}"));
        // 읽은 값을 그대로 다시 넣는 것은 항상 성공해야 한다
        c.set_value(key, &before)
            .unwrap_or_else(|e| panic!("set_value 실패: {key} — {e}"));
        let after = c.get_value(key).unwrap();
        assert_eq!(before, after, "왕복이 값을 바꿨다: {key}");
    }
}

#[test]
fn 편집키_목록에_중복이_없다() {
    let keys = Config::all_editable_keys();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        keys.len(),
        sorted.len(),
        "레지스트리와 명시적 키 목록이 겹친다 — 한쪽이 죽은 코드가 된다"
    );
}

#[test]
fn 변경분_적용이_모든_키에서_동작한다() {
    // 저장 경로가 실제로 쓰는 패턴: 두 설정을 키 단위로 비교해 다른 것만 적용
    let base = Config::default();
    let mut edited = Config::default();
    edited.launcher.keymap.kanata_path = Some("/tmp/kanata".into());
    edited.launcher.spell_providers = vec!["naver_spell".into()];
    edited.launcher.everything_path = Some("/tmp/es.exe".into());

    let mut target = Config::default();
    for key in Config::all_editable_keys() {
        let (old, new) = (base.get_value(key), edited.get_value(key));
        if old != new {
            if let Some(v) = new {
                target.set_value(key, &v).unwrap();
            }
        }
    }

    assert_eq!(
        target.launcher.keymap.kanata_path.as_deref(),
        Some(std::path::Path::new("/tmp/kanata")),
        "keymap.kanata_path가 반영되지 않았다"
    );
    assert_eq!(target.launcher.spell_providers, vec!["naver_spell"]);
    assert_eq!(
        target.launcher.everything_path.as_deref(),
        Some(std::path::Path::new("/tmp/es.exe"))
    );
}

#[test]
fn 읽기_실패는_기본값으로_덮지_않는다() {
    let dir = std::env::temp_dir().join(format!(
        "kmd_cfg_broken_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(CONFIG_FILENAME);

    // 문법이 깨진 설정 파일
    std::fs::write(&path, b"this is [not valid toml").unwrap();

    let r = Config::update_and_save(&path, |c| c.general.render_fps = 99);
    assert!(r.is_err(), "깨진 설정을 기본값으로 덮어쓰면 안 된다");

    // 원본이 그대로 남아야 한다
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("not valid toml"), "원본이 보존되어야 한다");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 동시 편집: 남의 변경을 덮지 않는다 (REF-01) ────────────────────
//
// save()는 메모리 전체를 쓰므로, 오래 열어 둔 창이 저장하면 그 사이 다른
// 곳에서 바뀐 값까지 되돌린다. update_and_save는 저장 직전에 파일을 다시
// 읽어 변경분만 얹는다.

#[test]
fn 설정이_없으면_배포본의_시드를_한_번만_심는다() {
    let exe_dir = tempfile::tempdir().unwrap();
    let cfg_dir = tempfile::tempdir().unwrap();
    let path = cfg_dir.path().join(CONFIG_FILENAME);

    // 시드가 없으면 아무것도 만들지 않는다
    seed_config(&path, exe_dir.path());
    assert!(!path.exists());

    std::fs::write(
        exe_dir.path().join(SEED_CONFIG_FILENAME),
        "[general]\ntheme = \"nord\"\n",
    )
    .unwrap();
    seed_config(&path, exe_dir.path());
    assert!(std::fs::read_to_string(&path).unwrap().contains("nord"));

    // 이미 사용자 설정이 있으면 Config::load가 시드를 다시 심지 않는다
    std::fs::write(&path, "[general]\ntheme = \"snow\"\n").unwrap();
    let loaded = Config::load(cfg_dir.path()).unwrap();
    assert_eq!(loaded.general.theme, "snow");
}

#[test]
fn 변경분만_적용해_다른_곳의_수정을_보존한다() {
    let dir = std::env::temp_dir().join(format!(
        "kmd_cfg_concurrent_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(CONFIG_FILENAME);

    // 초기 상태 저장
    let mut base = Config {
        config_path: Some(path.clone()),
        ..Default::default()
    };
    base.general.theme = "default".into();
    base.general.render_fps = 30;
    base.save().unwrap();

    // A가 설정을 읽어 든다(오래 열어 둔 창)
    let stale = Config::load(&dir).unwrap();
    assert_eq!(stale.general.theme, "default");

    // 그 사이 B가 테마를 바꿔 저장
    Config::update_and_save(&path, |c| c.general.theme = "nord".into()).unwrap();

    // A가 자기 항목(fps)만 바꿔 저장 — B의 테마를 되돌리면 안 된다
    let saved = Config::update_and_save(&path, |c| {
        c.general.render_fps = stale.general.render_fps + 15;
    })
    .unwrap();

    assert_eq!(saved.general.render_fps, 45, "내 변경은 반영된다");
    assert_eq!(saved.general.theme, "nord", "남의 변경을 덮지 않는다");

    // 디스크에도 동일하게 남아야 한다
    let reloaded = Config::load(&dir).unwrap();
    assert_eq!(reloaded.general.theme, "nord");
    assert_eq!(reloaded.general.render_fps, 45);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 저장_실패는_오류로_전달된다() {
    // 일반 파일을 부모로 지정 — 그 아래에는 디렉터리를 만들 수 없다
    let dir = std::env::temp_dir().join(format!(
        "kmd_cfg_failpath_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let blocker = dir.join("not-a-dir");
    std::fs::write(&blocker, b"x").unwrap();

    let err = Config::update_and_save(&blocker.join(CONFIG_FILENAME), |c| {
        c.general.render_fps = 99;
    });
    assert!(err.is_err(), "저장 불가 경로는 오류여야 한다");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 저장이_사용자_주석을_보존한다() {
    let path = temp_config_path("comments");
    let _ = std::fs::remove_file(&path);
    std::fs::write(
        &path,
        "# 내가 손으로 단 머리말
[general]
# FPS 설명 주석
render_fps = 30
",
    )
    .unwrap();

    let mut c = Config {
        config_path: Some(path.clone()),
        ..Default::default()
    };
    c.set_value("general.render_fps", "45").unwrap();
    c.save().unwrap();

    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains("# 내가 손으로 단 머리말"),
        "머리말 주석 보존
{saved}"
    );
    assert!(
        saved.contains("# FPS 설명 주석"),
        "키 주석 보존
{saved}"
    );
    assert!(
        saved.contains("render_fps = 45"),
        "값 반영
{saved}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn 저장_후_임시파일이_남지_않는다() {
    let path = temp_config_path("atomic");
    let _ = std::fs::remove_file(&path);

    let c = Config {
        config_path: Some(path.clone()),
        ..Default::default()
    };
    c.save().unwrap();

    assert!(path.exists(), "저장된 파일 존재");
    // 임시 파일명은 프로세스·스레드별로 고유하므로 이름을 특정할 수 없다.
    // 대신 **이 테스트가 만든 파일**의 이름으로 시작하는 tmp만 센다
    // (공용 temp 디렉터리에는 무관한 .tmp 파일이 널려 있다).
    let stem = path.file_stem().unwrap().to_string_lossy().to_string();
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with(&stem) && n.contains(".toml.tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "임시 파일은 rename으로 사라져야 한다: {leftovers:?}"
    );
    // 저장 결과가 다시 읽히는 온전한 TOML인지
    let reloaded: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(reloaded.general.render_fps, c.general.render_fps);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn 깨진_기존_파일이어도_저장은_성공한다() {
    // 병합 대상이 파싱 안 되면 새 직렬화 결과로 대체한다 — 저장 실패보다 낫다
    let path = temp_config_path("broken");
    std::fs::write(&path, "이건 = = 올바른 TOML이 아니다 [[[").unwrap();

    let c = Config {
        config_path: Some(path.clone()),
        ..Default::default()
    };
    c.save().unwrap();

    let reloaded: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(reloaded.general.render_fps, 30);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_default_config() {
    let config = Config::default();
    assert_eq!(config.general.render_fps, 30);
    assert_eq!(config.launcher.file_search_provider, "auto");
    assert!(config.keybindings.global_hotkey.is_empty());
    assert_eq!(config.launcher.kind_weights.directory, 80);
    assert!(config.launcher.index_directories);
    assert!(!config.launcher.scan_drives);
    assert_eq!(config.launcher.drive_scan_depth, 2);
    assert_eq!(
        config.launcher.multi_llm_providers,
        vec!["chatgpt", "gemini", "claude", "grok", "perplexity"]
    );
    assert_eq!(
        config.launcher.multi_llm_prefixes,
        vec!["@llm", "@ll", "@multi", "@cmp", "@compare"]
    );
    assert_eq!(
        config.launcher.multi_web_providers,
        vec!["google", "naver_search", "daum"]
    );
    assert_eq!(
        config.launcher.multi_web_prefixes,
        vec![
            "@m",
            "@mw",
            "@msearch",
            "@multisearch",
            "@searchall",
            "@krsearch"
        ]
    );
    assert_eq!(
        config.launcher.spell_providers,
        vec!["naver_spell", "pusan_spell"]
    );
    assert_eq!(config.launcher.spell_prefixes, vec!["@sp", "@spell"]);
    assert_eq!(
        config.launcher.translate_providers,
        vec!["google_translate", "papago", "deepl"]
    );
    assert_eq!(
        config.launcher.translate_prefixes,
        vec!["@tr", "@trko", "@tren"]
    );
    assert_eq!(config.launcher.keymap.backend, "kanata");
    assert_eq!(config.launcher.keymap.active_profile, "vim-nav");
}

#[test]
fn test_get_set_value() {
    let mut config = Config::default();
    config.set_value("general.theme", "nord").unwrap();
    assert_eq!(config.get_value("general.theme"), Some("nord".to_string()));

    config
        .set_value("launcher.kind_weights.directory", "90")
        .unwrap();
    assert_eq!(
        config.get_value("launcher.kind_weights.directory"),
        Some("90".to_string())
    );

    config
        .set_value("launcher.multi_llm_providers", "chatgpt,claude,perplexity")
        .unwrap();
    assert_eq!(
        config.get_value("launcher.multi_llm_providers"),
        Some("chatgpt,claude,perplexity".to_string())
    );
    config
        .set_value("launcher.multi_llm_prefixes", "llm,ll,cmp")
        .unwrap();
    assert_eq!(
        config.get_value("launcher.multi_llm_prefixes"),
        Some("@llm,@ll,@cmp".to_string())
    );

    config
        .set_value("launcher.multi_web_providers", "google,daum")
        .unwrap();
    assert_eq!(
        config.get_value("launcher.multi_web_providers"),
        Some("google,daum".to_string())
    );
    config
        .set_value("launcher.multi_web_prefixes", "m,mw,msearch")
        .unwrap();
    assert_eq!(
        config.get_value("launcher.multi_web_prefixes"),
        Some("@m,@mw,@msearch".to_string())
    );
    config
        .set_value("launcher.spell_prefixes", "sp,spell")
        .unwrap();
    assert_eq!(
        config.get_value("launcher.spell_prefixes"),
        Some("@sp,@spell".to_string())
    );
    config
        .set_value("launcher.translate_prefixes", "tr,trko,tren")
        .unwrap();
    assert_eq!(
        config.get_value("launcher.translate_prefixes"),
        Some("@tr,@trko,@tren".to_string())
    );
    config
        .set_value("launcher.keymap.active_profile", "coding.kbd")
        .unwrap();
    assert_eq!(
        config.get_value("launcher.keymap.active_profile"),
        Some("coding.kbd".to_string())
    );
}

#[test]
fn test_unknown_key() {
    let mut config = Config::default();
    assert!(config.set_value("nonexistent.key", "val").is_err());
}

#[test]
fn ignore_extensions_정규화_점_별표_대소문자() {
    let config: Config = toml::from_str(
        r#"
[launcher]
ignore_extensions = ["cat", ".SF", "*.Inf", "  ", "*."]
"#,
    )
    .unwrap();
    let mut got: Vec<String> = config
        .launcher
        .ignored_extension_set()
        .into_iter()
        .collect();
    got.sort();
    assert_eq!(got, vec!["cat", "inf", "sf"]);
    // 키를 안 적으면 비어 있다 (기존 설정 호환)
    assert!(Config::default().launcher.ignore_extensions.is_empty());
}

#[test]
fn 키가_없으면_기본_폴더와_기본_제외_목록이_합산된다() {
    let c = Config::from_toml_str("[launcher]\nmax_results = 10\n").unwrap();
    assert_eq!(c.launcher.include_user_folders, Some(true));
    assert_eq!(c.launcher.use_default_ignores, Some(true));
    assert!(c.launcher.search_paths.is_empty());
    assert_eq!(
        c.launcher.effective_ignore_patterns().len(),
        DEFAULT_IGNORE_PATTERNS.len()
    );
    assert_eq!(
        c.launcher.effective_search_paths(),
        default_user_folders(),
        "기본 폴더만 — 예전 기본값과 같은 범위"
    );
}

#[test]
fn 레거시_search_paths와_ignore_patterns는_대체_의미를_지킨다() {
    let dir = tempfile::tempdir().unwrap();
    let toml = format!(
        "[launcher]\nsearch_paths = [{:?}]\nignore_patterns = [\"only_this\"]\n",
        dir.path().display().to_string()
    );
    let c = Config::from_toml_str(&toml).unwrap();
    assert_eq!(c.launcher.include_user_folders, None, "레거시 판정");
    assert_eq!(c.launcher.use_default_ignores, None);
    assert_eq!(
        c.launcher.effective_search_paths(),
        vec![dir.path().to_path_buf()]
    );
    assert_eq!(
        c.launcher.effective_ignore_patterns(),
        vec!["only_this".to_string()]
    );

    // 저장해도 스위치 키를 새로 쓰지 않는다 — 레거시 파일은 레거시로 남는다
    let out = toml::to_string(&c).unwrap();
    assert!(!out.contains("include_user_folders"));
    assert!(!out.contains("use_default_ignores"));
}

#[test]
fn 스위치를_켜면_목록은_기본값에_더해진다() {
    let c = Config::from_toml_str(
        "[launcher]\nuse_default_ignores = true\nignore_patterns = [\"plugins\", \".git\"]\n",
    )
    .unwrap();
    let eff = c.launcher.effective_ignore_patterns();
    assert_eq!(
        eff.len(),
        DEFAULT_IGNORE_PATTERNS.len() + 1,
        ".git은 중복이라 한 번만"
    );
    assert!(eff.contains(&"plugins".to_string()));
}
