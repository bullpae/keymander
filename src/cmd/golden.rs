//! `kmd index --golden` — 실행 이력으로 옛 엔진과 새 파일 검색 엔진의 순위를 비교한다 (docs/22 §3.7)
//!
//! 실제로 연 항목마다 사람이 칠 법한 질의(이름 앞 세 글자·첫 단어·머리글자)를 만들고,
//! 그 항목이 몇 위에 오는지 두 엔진에서 잰다. 새 엔진을 켜지 않았어도 돌아간다 —
//! 스냅숏이 없으면 메모리에서 한 번 스캔한다.

use std::collections::HashSet;

use color_eyre::Result;
use kmd_core::filesearch::{scan, Boost, FileIndex};
use kmd_core::SearchResult;

const LIMIT: usize = 50;
const MAX_ITEMS: usize = 150;

/// 한 엔진의 순위를 고르는 함수
type Pick = fn(&Row) -> Option<usize>;

/// 질의 하나의 결과 — 그 항목이 두 엔진에서 몇 위인지 (없으면 None)
struct Row {
    query: String,
    path: String,
    old: Option<usize>,
    new: Option<usize>,
}

pub fn run() -> Result<()> {
    let config = super::load_config()?;
    let weights = &config.launcher.kind_weights;
    let emoji = config.general.emoji_icons;

    // 이력: 런처 두 곳(데스크톱·TUI/CLI)을 합친다
    let mut entries = Vec::new();
    for role in kmd_core::DbRole::ALL {
        if let Ok(db) = kmd_core::Database::open_default(role) {
            entries.extend(db.query_history(500));
        }
    }
    let mut frecency = kmd_core::history::FrecencyMap::new();
    for role in kmd_core::DbRole::ALL {
        if let Ok(db) = kmd_core::Database::open_default(role) {
            for (k, v) in kmd_core::history::load_boost_map(&db) {
                frecency.entry(k).or_insert(v);
            }
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.frequency));
    let mut seen = HashSet::new();
    // Windows 는 대소문자만 다른 이력(`d:\util\x` · `D:\Util\x`)을 한 항목으로 센다
    entries.retain(|e| {
        std::path::Path::new(&e.value).exists()
            && seen.insert(kmd_core::history::path_key(&e.value).into_owned())
    });
    entries.truncate(MAX_ITEMS);
    if entries.is_empty() {
        println!("실행 이력이 없습니다 — 런처로 몇 가지를 열어 본 뒤 다시 실행하세요.");
        return Ok(());
    }

    println!("엔진 준비 중…");
    let mut legacy = super::create_search_engine(&config);
    legacy.set_fuzzy_wait(std::time::Duration::from_secs(5));
    let index = match FileIndex::open_default() {
        Some(i) => i,
        None => {
            let (b, stats) = scan::build_table(&scan::ScanConfig::from_launcher(&config.launcher));
            println!(
                "(스냅숏이 없어 메모리에서 스캔: 폴더 {} 파일 {})",
                stats.dirs, stats.files
            );
            FileIndex::Memory(b)
        }
    };
    let boost = Boost::new(&frecency);

    let mut rows: Vec<Row> = Vec::new();
    for e in &entries {
        let name = std::path::Path::new(&e.value)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| e.display.clone());
        for q in queries_for(&name) {
            let (_, mut old) = legacy.search(&q, LIMIT);
            kmd_core::history::boost_results_with_map(&mut old, &frecency);
            let (_, base) = legacy.search(&q, LIMIT);
            let files = index.search(&q, LIMIT, &boost, weights, emoji);
            let new = kmd_core::rank::merge(base, files, &q, weights, &frecency);
            rows.push(Row {
                old: position(&old, &e.value),
                new: position(&new, &e.value),
                query: q,
                path: e.value.clone(),
            });
        }
    }

    let total = rows.len();
    let pct = |pick: Pick, k: usize| {
        let n = rows
            .iter()
            .filter(|r| pick(r).is_some_and(|p| p < k))
            .count();
        n as f64 * 100.0 / total as f64
    };
    println!("\n항목 {}개, 질의 {}개", entries.len(), total);
    println!("              1위      3위 안    50위 안");
    let engines: [(&str, Pick); 2] = [("옛 엔진", |r| r.old), ("새 엔진", |r| r.new)];
    for (label, pick) in engines {
        println!(
            "  {label}  {:>5.1}%   {:>5.1}%   {:>5.1}%",
            pct(pick, 1),
            pct(pick, 3),
            pct(pick, LIMIT)
        );
    }

    let lower = |r: &Row| {
        r.new
            .unwrap_or(LIMIT)
            .saturating_sub(r.old.unwrap_or(LIMIT))
    };
    let mut worse: Vec<&Row> = rows.iter().filter(|r| lower(r) > 0).collect();
    worse.sort_by_key(|r| std::cmp::Reverse(lower(r)));
    if !worse.is_empty() {
        println!("\n새 엔진에서 더 아래로 간 것 (최대 15):");
        for r in worse.into_iter().take(15) {
            println!(
                "  {:<14} 옛 {:>4}  새 {:>4}  {}",
                r.query,
                rank_label(r.old),
                rank_label(r.new),
                r.path
            );
        }
    }
    Ok(())
}

/// 사람이 칠 법한 질의 — 앞 세 글자, 첫 단어(확장자 뗀), 여러 단어면 머리글자
fn queries_for(name: &str) -> Vec<String> {
    let stem = name
        .rsplit_once('.')
        .map_or(name, |(s, _)| s)
        .to_lowercase();
    let mut qs = Vec::new();
    let head: String = stem.chars().take(3).collect();
    if head.chars().count() == 3 {
        qs.push(head);
    }
    let words: Vec<&str> = stem
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    if let Some(w) = words.first() {
        if w.chars().count() > 3 {
            qs.push((*w).to_string());
        }
    }
    if words.len() >= 2 {
        let initials: String = words.iter().filter_map(|w| w.chars().next()).collect();
        if initials.is_ascii() {
            qs.push(initials);
        }
    }
    qs.dedup();
    qs
}

fn position(results: &[SearchResult], path: &str) -> Option<usize> {
    results
        .iter()
        .position(|r| kmd_core::history::same_path(&r.item.path, path))
}

fn rank_label(p: Option<usize>) -> String {
    p.map_or("없음".into(), |p| format!("{}", p + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 질의_만들기() {
        assert_eq!(
            queries_for("Visual Studio Code.app"),
            vec!["vis", "visual", "vsc"]
        );
        assert_eq!(queries_for("보고서_최종.hwp"), vec!["보고서"]);
        assert_eq!(queries_for("README.md"), vec!["rea", "readme"]);
    }
}
