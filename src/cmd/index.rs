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
                "\n추가: 런처에서 `?` 입력 후 제안 항목에 Enter, `:scope <폴더>`,\n\
                 또는 `kmd scope add <폴더>`."
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

    // 데몬이 떠 있으면 데몬에 맡긴다 — 여기서 캐시만 새로 쓰면 데몬 메모리의
    // 엔진은 옛 인덱스 그대로라, 예전엔 리빌드 후 데몬을 재시작해야 했다.
    if rebuild {
        if let Ok(kmd_core::ipc::Response::Ok { message }) =
            kmd_core::ipc::send_request_with_timeout(
                &kmd_core::ipc::Request::RebuildIndex,
                std::time::Duration::from_secs(120),
            )
        {
            println!("데몬이 인덱스를 다시 만들었습니다 — {message}");
            println!("(본문 인덱스는 데몬이 이어서 갱신합니다)");
            if let Some(index) =
                kmd_core::index::store::try_load_cached(&bin_path, &json_path, expected_version)
            {
                print_stats(&index);
            }
            return Ok(());
        }
    }

    if rebuild || (!bin_path.exists() && !json_path.exists()) {
        let provider = kmd_core::index::files::detect_provider(
            &config.launcher.file_search_provider,
            config.launcher.everything_path.as_ref(),
        );
        println!("File provider: {}", provider);

        let roots = config.launcher.effective_search_paths();
        if !roots.is_empty() {
            println!("Search paths:");
            for d in &roots {
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

/// 새 파일 검색 엔진 스냅숏 (docs/22) — 세대·항목 수·검색 폴더별 상태
fn print_file_index_stats() {
    use kmd_core::filesearch::{snapshot, table::Kind};
    let dir = snapshot::default_dir();
    let Ok(snap) = snapshot::Snapshot::open_current(&dir) else {
        return;
    };
    let t = snap.table();
    let dirs = (0..t.len()).filter(|&r| t.kind(r) == Kind::Dir).count();
    println!(
        "\n  File index (docs/22): 세대 {}, 폴더 {}, 파일 {} — {}",
        snap.generation(),
        dirs,
        t.len() - dirs,
        snap.path().display()
    );
    for root in t.roots {
        let note = if root.mtime_reliable {
            "열 때 대조"
        } else {
            "폴더 시각을 믿을 수 없는 볼륨 — 주기 재빌드로만 갱신"
        };
        println!("    {}  ({note})", root.path_buf().display());
    }
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
    print_file_index_stats();

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
