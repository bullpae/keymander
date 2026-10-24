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
    Stats { toggle: Option<String>, days: u32 },
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
        Action::Stats { toggle, days } => stats(&mut config, toggle.as_deref(), days)?,
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
fn stats(config: &mut kmd_core::Config, toggle: Option<&str>, days: u32) -> Result<()> {
    match toggle {
        Some("on") | Some("off") => {
            let on = toggle == Some("on");
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
            color_eyre::eyre::bail!("모르는 인자: {other} — on | off | clear 중 하나");
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
    println!();
    println!("  사부의 해석");
    for s in r.insights() {
        println!("    · {s}");
    }
    Ok(())
}
