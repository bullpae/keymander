//! `kmd scope` — 검색 범위(인덱싱 폴더) 보기·추가·제거 (docs/18)

use std::path::Path;
use std::time::Duration;

use color_eyre::Result;

pub enum Action {
    List,
    Add(String),
    Remove(String),
}

pub fn run(action: Action) -> Result<()> {
    let mut config = super::load_config()?;
    match action {
        Action::List => {
            println!("검색 범위:");
            for e in kmd_core::scope::entries(&config.launcher) {
                let badge = if e.is_user_folder { "기본" } else { "추가" };
                let missing = if e.path.is_dir() {
                    ""
                } else {
                    "  (지금 없음)"
                };
                println!("  [{badge}] {}{missing}", e.path.display());
            }
            let ignores = config.launcher.effective_ignore_patterns().len();
            let exts = config.launcher.ignored_extension_set().len();
            println!("\n제외 규칙: 폴더 이름 {ignores}개 · 확장자 {exts}개");
            println!("추가: kmd scope add <폴더>   제거: kmd scope rm <폴더>");
            return Ok(());
        }
        Action::Add(raw) => {
            let path = kmd_core::scope::parse_input(&raw);
            match kmd_core::scope::add(&mut config, &path) {
                Ok((kmd_core::scope::AddOutcome::AlreadyCovered(_), msg)) => {
                    println!("{msg}");
                    return Ok(());
                }
                Ok((_, msg)) => println!("{msg}"),
                Err(e) => color_eyre::eyre::bail!(e),
            }
            reindex(Some(&path))?;
        }
        Action::Remove(raw) => {
            let path = kmd_core::scope::parse_input(&raw);
            match kmd_core::scope::remove(&mut config, &path) {
                Ok(msg) => println!("{msg}"),
                Err(e) => color_eyre::eyre::bail!(e),
            }
            reindex(None)?;
        }
    }
    Ok(())
}

/// 바로 다시 인덱싱 — 데몬이 있으면 데몬 메모리까지 교체된다.
fn reindex(added: Option<&Path>) -> Result<()> {
    print!("인덱싱 중… ");
    let started = std::time::Instant::now();
    let via_daemon = kmd_core::scope::rebuild_index_now(Duration::from_secs(120))
        .map_err(|e| color_eyre::eyre::eyre!(e))?;
    let (bin, json) = super::index_cache_paths();
    let index =
        kmd_core::index::store::try_load_cached(&bin, &json, kmd_core::Index::current_version());
    let total = index.as_ref().map_or(0, |i| i.items.len());
    println!(
        "완료 ({}, {:.1}s, 전체 {total}개 항목)",
        if via_daemon {
            "데몬"
        } else {
            "직접 빌드"
        },
        started.elapsed().as_secs_f64()
    );
    if let (Some(path), Some(index)) = (added, index) {
        let n = index
            .items
            .iter()
            .filter(|i| kmd_core::fsutil::path_within(Path::new(&i.path), path))
            .count();
        println!("{} — {n}개 검색 가능", path.display());
    }
    Ok(())
}
