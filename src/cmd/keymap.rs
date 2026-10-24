//! `kmd keymap` — keymap backend control (Kanata)

use color_eyre::Result;

pub enum Action {
    Start,
    Stop,
    Status,
    List,
    Use { profile: String },
    Init { profile: Option<String> },
    ListPresets,
    Stats { args: Vec<String>, days: u32 },
}

pub fn run(action: Action) -> Result<()> {
    let mut config = super::load_config()?;

    match action {
        Action::Start => match kmd_core::keymap::start(&config) {
            Ok(msg) => println!("{msg}"),
            Err(e) => println!("{e}"),
        },
        Action::Stop => match kmd_core::keymap::stop() {
            Ok(msg) => println!("{msg}"),
            Err(e) => println!("{e}"),
        },
        Action::Status => {
            println!("keymap status: {}", kmd_core::keymap::status());
            println!(
                "backend: {}, active_profile: {}",
                config.launcher.keymap.backend, config.launcher.keymap.active_profile
            );
        }
        Action::List => {
            let profiles = kmd_core::keymap::list_profiles(&config);
            if profiles.is_empty() {
                println!("등록된 keymap 프로파일이 없습니다.");
                println!("힌트: `kmd keymap init` 으로 기본 프리셋을 설치하세요.");
            } else {
                println!("keymap profiles:");
                for p in profiles {
                    if p == config.launcher.keymap.active_profile {
                        println!("  * {p} (active)");
                    } else {
                        println!("  - {p}");
                    }
                }
            }
        }
        Action::Use { profile } => {
            kmd_core::keymap::validate_profile_name(&profile)
                .map_err(|e| color_eyre::eyre::eyre!(e))?;
            let profile_name = kmd_core::keymap::with_extension(&profile);
            let path = kmd_core::keymap::profile_dir(&config).join(&profile_name);
            if !kmd_core::keymap::exists(&path) {
                return Err(color_eyre::eyre::eyre!(format!(
                    "프로파일 파일이 없습니다: {}\n먼저 `kmd keymap init {}` 실행을 권장합니다.",
                    path.display(),
                    profile_name
                )));
            }
            config.launcher.keymap.active_profile = profile_name.clone();
            config.save()?;
            println!("active profile 변경: {}", profile_name);
        }
        Action::Init { profile } => {
            let profile_name = profile.unwrap_or_else(|| "vim-nav".to_string());
            kmd_core::keymap::validate_profile_name(&profile_name)
                .map_err(|e| color_eyre::eyre::eyre!(e))?;
            let created = kmd_core::keymap::create_profile_template(&config, &profile_name)
                .map_err(|e| color_eyre::eyre::eyre!(e))?;
            let final_name = created
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| profile_name.clone());
            config.launcher.keymap.active_profile = final_name.clone();
            config.save()?;
            println!("프로파일 준비 완료: {}", created.display());
            println!("active profile: {final_name}");

            if kmd_core::keymap::preset_content(&profile_name).is_some() {
                println!("(내장 프리셋 '{profile_name}' 사용)");
            }
        }
        Action::Stats { args, days } => stats(&mut config, &args, days)?,
        Action::ListPresets => {
            println!("사용 가능한 내장 프리셋:");
            for (name, desc) in kmd_core::keymap::list_presets() {
                println!("  {name:<12} {desc}");
            }
            println!();
            println!("설치: kmd keymap init <preset>");
        }
    }

    Ok(())
}

/// `kmd keymap stats` — 손버릇 통계 (docs/19)
fn stats(config: &mut kmd_core::Config, args: &[String], days: u32) -> Result<()> {
    let rest = args.get(1..).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("sync") => return stats_sync(config, rest),
        Some("export") => return stats_export(config, rest),
        Some("compare") => return stats_compare(config, rest, days),
        Some("on") | Some("off") => {
            let on = args[0] == "on";
            config.launcher.keymap.stats = on;
            config.save()?;
            if on {
                println!("손버릇 통계를 켰다 — 데몬이 다음 시작부터 센다 (kmd daemon restart).");
                println!("세는 것: 레이어 수 사용 횟수 · 레이어 밖 맨 화살표 횟수 · 트리거 홀드 시간(100ms 칸).");
                println!("세지 않는 것: 글자, 순서, 시각. 날짜별 합계만 이 기기의 DB 에 남는다.");
            } else {
                println!("손버릇 통계를 껐다 — 데몬 재시작 후 더 세지 않는다. 지우려면 kmd keymap stats clear");
            }
            return Ok(());
        }
        Some("clear") => {
            let n = super::open_db()?.key_stats_clear()?;
            println!("손버릇 통계 {n}줄을 지웠다.");
            return Ok(());
        }
        Some(other) => {
            color_eyre::eyre::bail!(
                "모르는 인자: {other} — on | off | clear | sync | export | compare 중 하나"
            );
        }
        None => {}
    }

    let db = super::open_db()?;
    let (rows, active) = db.key_stats_since(days);
    let effective = kmd_core::keymap::effective_keymap(&config.launcher.keymap);
    let r = kmd_core::keystats::KeyReport::build(&rows, active, days, &effective);
    println!(
        "손버릇 통계 — 최근 {days}일 (데이터 {}일) · 수집 {}",
        r.active_days,
        if config.launcher.keymap.stats {
            "켜짐"
        } else {
            "꺼짐 (kmd keymap stats on)"
        }
    );
    println!();
    if r.is_empty() {
        if config.launcher.keymap.stats {
            for s in r.insights() {
                println!("  {s}");
            }
        } else {
            println!("  수집이 꺼져 있다 — kmd keymap stats on 후 데몬을 다시 시작하면 센다.");
        }
        return Ok(());
    }
    let max = r.moves.first().map_or(1, |m| m.count.max(1));
    println!("  많이 쓴 레이어 수 (레이어 수 {}번)", r.total_moves());
    for m in r.moves.iter().take(12) {
        let bar = (m.count * 24).div_ceil(max) as usize;
        println!(
            "    {:<6} {:<10} {:<16} {:<24} {}",
            m.layer,
            m.key,
            m.target.as_deref().unwrap_or("(배치에 없음)"),
            "█".repeat(bar),
            m.count
        );
    }
    if !r.raw.is_empty() {
        println!();
        println!("  레이어 밖 맨 이동 키 ({}번)", r.total_raw());
        for (k, n) in &r.raw {
            println!("    {k:<10} {n}");
        }
    }
    let holds = r.total_holds();
    if holds > 0 {
        println!();
        println!(
            "  트리거 홀드 시간 ({holds}번 · 그냥 놓음 {}번)",
            r.total_idle()
        );
        let hmax = r.holds.iter().copied().max().unwrap_or(1).max(1);
        for (i, n) in r.holds.iter().enumerate() {
            if *n == 0 {
                continue;
            }
            let label = if i + 1 == kmd_core::keystats::HOLD_BUCKETS {
                "1초 이상".to_string()
            } else {
                format!("{}–{}ms", i * 100, (i + 1) * 100)
            };
            println!(
                "    {label:<10} {:<24} {n}",
                "█".repeat((n * 24).div_ceil(hmax) as usize)
            );
        }
    }
    if r.total_taps() > 0 {
        println!();
        println!("  트리거 탭 동작 (한영 등) {}번", r.total_taps());
    }
    if !r.perf.is_empty() {
        println!();
        println!("  단축키 실행 지연 (중앙값 / 90% · 횟수)");
        for (phase, name) in kmd_core::keystats::PERF_PHASES {
            if let (Some(a), Some(b)) = (r.perf_pct(phase, 50), r.perf_pct(phase, 90)) {
                // 한글 이름은 폭이 달라 맨 뒤에 둔다
                println!(
                    "    {:>7} / {:<7} {:>5}번  {name}",
                    kmd_core::keystats::perf_label(a),
                    kmd_core::keystats::perf_label(b),
                    r.perf_count(phase)
                );
            }
        }
    }
    println!();
    println!("  사부의 해석");
    for s in r.insights() {
        println!("    · {s}");
    }
    Ok(())
}

/// `kmd keymap stats sync <DIR|off>` — 공유 폴더를 정하면 데몬이 저장할 때마다 내보낸다
fn stats_sync(config: &mut kmd_core::Config, rest: &[String]) -> Result<()> {
    let km = &mut config.launcher.keymap;
    match rest.first().map(String::as_str) {
        None => {
            match kmd_core::keystats::sync_dir(km) {
                Some(dir) => println!(
                    "공유 폴더: {} (기기 이름 {})",
                    dir.display(),
                    kmd_core::keystats::device_name()
                ),
                None => println!("공유 폴더가 없다 — kmd keymap stats sync <폴더> 로 정한다."),
            }
            return Ok(());
        }
        Some("off") => {
            km.stats_sync = None;
            config.save()?;
            println!("공유 폴더 내보내기를 껐다 (이미 쓴 파일은 남는다).");
            return Ok(());
        }
        Some(dir) => {
            let abs = kmd_core::keystats::expand_sync_path(dir)
                .map_err(|e| color_eyre::eyre::eyre!("공유 폴더 경로: {e}"))?;
            km.stats_sync = Some(abs);
        }
    }
    config.save()?;
    let km = &config.launcher.keymap;
    let db = super::open_db()?;
    let effective = kmd_core::keymap::effective_keymap(km);
    match kmd_core::keystats::sync_export(&db, km, &effective)? {
        Some(path) => println!("공유 폴더를 정했다 — 지금 내보냄: {}", path.display()),
        None => println!("공유 폴더를 정했다."),
    }
    println!("데몬이 5분마다 이 파일을 새로 쓴다 — kmd daemon restart 후부터.");
    println!("다른 기기에서도 같은 폴더로 sync 를 켜고, 아무 기기에서나 kmd keymap stats compare");
    if !km.stats {
        println!("(수집이 꺼져 있다 — kmd keymap stats on)");
    }
    Ok(())
}

/// `kmd keymap stats export [PATH]` — 경로가 없으면 표준 출력
fn stats_export(config: &kmd_core::Config, rest: &[String]) -> Result<()> {
    let db = super::open_db()?;
    let effective = kmd_core::keymap::effective_keymap(&config.launcher.keymap);
    let snap = kmd_core::keystats::Snapshot::from_db(
        &db,
        &kmd_core::keystats::device_name(),
        &kmd_core::keystats::device_id(&kmd_core::Config::default_data_dir()),
        &effective,
    );
    match rest.first().map(String::as_str) {
        None | Some("-") => println!("{}", serde_json::to_string_pretty(&snap)?),
        Some(p) => {
            let path = snap.write_to(std::path::Path::new(p))?;
            eprintln!("내보냄: {} ({}줄)", path.display(), snap.rows.len());
        }
    }
    Ok(())
}

/// `kmd keymap stats compare [PATH...]` — 이 기기(DB) + 경로(없으면 공유 폴더)의 스냅숏을 나란히
fn stats_compare(config: &kmd_core::Config, rest: &[String], days: u32) -> Result<()> {
    use kmd_core::keystats::{device_name, sync_dir, Compare, Snapshot};
    let db = super::open_db()?;
    let me = device_name();
    let effective = kmd_core::keymap::effective_keymap(&config.launcher.keymap);
    let my_id = kmd_core::keystats::device_id(&kmd_core::Config::default_data_dir());
    let mut snaps = vec![Snapshot::from_db(&db, &me, &my_id, &effective)];

    let paths: Vec<std::path::PathBuf> = if rest.is_empty() {
        sync_dir(&config.launcher.keymap).into_iter().collect()
    } else {
        rest.iter().map(std::path::PathBuf::from).collect()
    };
    if paths.is_empty() {
        eprintln!("다른 기기 파일이 없다 — kmd keymap stats sync <공유 폴더> 를 정하거나 compare <파일|폴더> 로 준다.");
    }
    for p in &paths {
        let (ok, bad) = Snapshot::load(p);
        for (f, why) in bad {
            eprintln!("못 읽음: {} — {why}", f.display());
        }
        // 이 기기 것은 DB 가 더 새롭다
        snaps.extend(ok.into_iter().filter(|s| !s.same_device(&me, &my_id)));
    }
    let since = db.local_date(1 - days.max(1) as i32);
    print!("{}", Compare::new(&snaps, &since, days).markdown());
    Ok(())
}
