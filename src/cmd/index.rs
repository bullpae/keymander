//! `kmd index` — manage the search index

use color_eyre::Result;

pub fn run(rebuild: bool, stats: bool, suggest: bool) -> Result<()> {
    let config = super::load_config()?;
    let (bin_path, json_path) = super::index_cache_paths();
    let expected_version = kmd_core::Index::current_version();

    if suggest {
        let suggestions = kmd_core::folder_suggest::suggest_folders(&config.launcher, 5);
        if suggestions.is_empty() {
            println!(
                "제안할 폴더가 없습니다 — 검색 범위 밖에서 최근 활동이 활발한 폴더가 없습니다."
            );
        } else {
            println!("검색 범위에 추가할 만한 폴더 (사용 이력 + 최근 활동 기준):\n");
            for s in &suggestions {
                println!("  {}  — {}", s.path.display(), s.reason_label());
            }
            println!(
                "\n추가: 런처에서 `?` 입력 후 제안 항목에 Enter, 또는 config.toml의\n\
                 launcher.search_paths에 경로를 직접 추가하세요."
            );
        }
        if !rebuild && !stats {
            return Ok(());
        }
    }

    if stats && !rebuild {
        if let Some(index) =
            kmd_core::index::store::try_load_cached(&bin_path, &json_path, expected_version)
        {
            print_stats(&index);
        } else {
            println!("No index found. Run `kmd index --rebuild` to create one.");
        }
        return Ok(());
    }

    if rebuild || (!bin_path.exists() && !json_path.exists()) {
        let provider = kmd_core::index::files::detect_provider(
            &config.launcher.file_search_provider,
            config.launcher.everything_path.as_ref(),
        );
        println!("File provider: {}", provider);

        if !config.launcher.search_paths.is_empty() {
            println!("Search paths:");
            for d in &config.launcher.search_paths {
                println!("  {}", d.display());
            }
        }
        if config.launcher.scan_drives {
            println!(
                "Auto-scan drives: enabled (depth {})",
                config.launcher.drive_scan_depth
            );
        }

        println!("Building index...");
        let start = std::time::Instant::now();
        let index = kmd_core::Index::build(&config.launcher, config.general.emoji_icons);
        let elapsed = start.elapsed();

        kmd_core::index::store::save_both(&index, &bin_path, &json_path);

        println!("Index built in {:.1}ms", elapsed.as_secs_f64() * 1000.0);

        // 본문 인덱스(FTS5)도 함께 갱신 (docs/15) — 데몬 없는 환경의 유일한 갱신 경로
        if config.launcher.content_search.enabled {
            match super::open_db() {
                Ok(db) => {
                    let start = std::time::Instant::now();
                    match kmd_core::content_index::sync(&db, &config.launcher) {
                        Ok(s) => println!(
                            "Content index synced in {:.1}ms (scanned {} / indexed {} / skipped {} / removed {} / failed {})",
                            start.elapsed().as_secs_f64() * 1000.0,
                            s.scanned, s.indexed, s.skipped, s.removed, s.failed
                        ),
                        Err(e) => println!("Content index sync failed: {e}"),
                    }
                }
                Err(e) => println!("Content index DB open failed: {e}"),
            }
        }

        print_stats(&index);
    } else {
        let index = super::load_or_build_index(&config.launcher, config.general.emoji_icons);
        println!("Index loaded from cache.");
        print_stats(&index);
        println!("\nUse `kmd index --rebuild` to refresh.");
    }

    Ok(())
}

fn print_stats(index: &kmd_core::Index) {
    use kmd_core::index::Source;
    use std::collections::HashMap;

    let total = index.len();
    let apps = index
        .items
        .iter()
        .filter(|i| i.source == Source::Apps)
        .count();
    let path_exes = index
        .items
        .iter()
        .filter(|i| i.source == Source::Path)
        .count();
    let files = index
        .items
        .iter()
        .filter(|i| i.source == Source::FileProvider)
        .count();
    let sys_cmds = index
        .items
        .iter()
        .filter(|i| i.source == Source::SystemCommand)
        .count();

    println!("\nIndex Statistics:");
    println!("  Total items:      {}", total);
    println!("  Applications:     {}", apps);
    println!("  PATH executables: {}", path_exes);
    println!("  Files:            {}", files);
    println!("  System commands:  {}", sys_cmds);

    // Show top file extensions
    if files > 0 {
        let mut ext_counts: HashMap<String, usize> = HashMap::new();
        for item in index
            .items
            .iter()
            .filter(|i| i.source == Source::FileProvider)
        {
            let ext = std::path::Path::new(&item.path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("(none)")
                .to_lowercase();
            *ext_counts.entry(ext).or_default() += 1;
        }
        let mut ext_vec: Vec<_> = ext_counts.into_iter().collect();
        ext_vec.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        println!("\n  Top file extensions:");
        for (ext, count) in ext_vec.iter().take(15) {
            println!("    .{:<10} {}", ext, count);
        }
    }

    if let Some(ref ts) = index.last_updated {
        println!("\n  Last updated:     {}", ts);
    }

    // 본문 인덱스(FTS5) 통계 — DB가 없거나 비어 있으면 조용히 생략
    if let Ok(db) = super::open_db() {
        if let Ok((count, bytes)) = kmd_core::content_index::stats(&db) {
            if count > 0 {
                println!(
                    "\n  Content index:    {} files ({:.1} MB source text) — try `kmd grep <query>`",
                    count,
                    bytes as f64 / (1024.0 * 1024.0)
                );
            }
        }
    }
}
