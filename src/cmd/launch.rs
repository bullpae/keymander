//! `kmd launch <target>` — launch a program, file, or URL

use color_eyre::Result;
use kmd_core::action;

pub fn run(target: &str) -> Result<()> {
    let config = super::load_config()?;

    // Check if it's a URL
    if target.starts_with("http://") || target.starts_with("https://") || target.starts_with("www.")
    {
        let url = if target.starts_with("www.") {
            format!("https://{}", target)
        } else {
            target.to_string()
        };
        match action::open_url(&url) {
            action::ActionResult::OpenedUrl(u) => println!("Opened: {}", u),
            action::ActionResult::Error(e) => eprintln!("Error: {}", e),
            _ => {}
        }
        return Ok(());
    }

    // Check if it's a file path
    let path = std::path::Path::new(target);
    if path.exists() {
        match action::open_with_system(target) {
            action::ActionResult::Launched => println!("Launched: {}", target),
            action::ActionResult::Error(e) => eprintln!("Error: {}", e),
            _ => {}
        }

        // Record in history — 절대 경로로 남겨야 폴더 제안(이력 신호)이 쓸 수 있다
        if let Ok(db) = super::open_db() {
            let kind = if path.is_dir() {
                kmd_core::ItemKind::Directory
            } else {
                kmd_core::ItemKind::File
            };
            let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            kmd_core::history::record_launch(&db, kind, &abs.to_string_lossy(), None);
        }

        return Ok(());
    }

    // Search for the target
    let mut engine = super::create_search_engine(&config);

    let (_mode, results) = engine.search(target, 1);

    if let Some(result) = results.first() {
        println!("Launching: {} ({})", result.item.name, result.item.path);
        match action::execute(result) {
            action::ActionResult::Launched => {
                if let Ok(db) = super::open_db() {
                    kmd_core::history::record_launch(
                        &db,
                        result.item.kind,
                        &result.item.path,
                        Some(&result.item.name),
                    );
                }
            }
            action::ActionResult::OpenedUrl(url) => println!("Opened: {}", url),
            action::ActionResult::NeedsConfirmation(name) => {
                eprintln!("'{}' requires confirmation. Use --force to skip.", name);
            }
            action::ActionResult::Error(e) => eprintln!("Error: {}", e),
        }
    } else {
        eprintln!("No match found for: {}", target);
    }

    Ok(())
}
