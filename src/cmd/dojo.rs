//! `kmd dojo` — 무협 도장 키맵 트레이너 (docs/10)

use color_eyre::Result;

pub fn run(lesson: Option<String>, free: bool) -> Result<()> {
    let config = super::load_config()?;
    // DB 가 없어도 수련은 된다 — 기록만 남지 않는다
    let db = match super::open_db() {
        Ok(db) => Some(db),
        Err(e) => {
            tracing::warn!("dojo: DB 를 열 수 없어 기록 없이 진행한다: {e}");
            None
        }
    };
    crate::tui::dojo::run(&config, db, lesson, free)
}

/// `kmd dojo card` — 결과 카드를 텍스트로
pub fn card() -> Result<()> {
    let config = super::load_config()?;
    let db = super::open_db().ok();
    print!("{}", crate::tui::dojo::card_text(&config, db));
    Ok(())
}

/// `kmd dojo seal export` — 이 기기의 인장 코드
pub fn seal_export() -> Result<()> {
    let db = super::open_db()?;
    let os = std::env::consts::OS;
    let Some(seal) = db.dojo_seals().into_iter().find(|s| s.os == os) else {
        color_eyre::eyre::bail!(
            "이 기기({os})에는 아직 인장이 없다 — kmd dojo 에서 검은띠를 따면 찍힌다"
        );
    };
    let code = kmd_core::dojo::seal_encode(&kmd_core::dojo::SealCode {
        os: seal.os,
        belt: "black".into(),
        at: seal.earned_at,
    });
    println!("{code}");
    eprintln!("다른 기기에서: kmd dojo seal import {code}");
    Ok(())
}

/// `kmd dojo seal import <코드>` — 다른 기기의 인장을 들인다
pub fn seal_import(code: &str) -> Result<()> {
    let db = super::open_db()?;
    let seal = kmd_core::dojo::seal_decode(code).map_err(|e| color_eyre::eyre::eyre!(e))?;
    if db.dojo_add_seal(&seal.os, seal.at, "imported")? {
        println!(
            "인장을 들였다: {} — 인장 {}/3",
            seal.os,
            db.dojo_seals().len()
        );
    } else {
        println!(
            "{} 인장은 이미 있다 — 인장 {}/3",
            seal.os,
            db.dojo_seals().len()
        );
    }
    Ok(())
}
