//! "자주 변하는 폴더" 제안 (docs/15 P2).
//!
//! 검색 범위(search_paths) 밖에서 최근 활동이 활발한 폴더를 찾아 사용자에게
//! **제안만** 한다 — 자동 추가는 하지 않는다 (인덱스 범위는 사생활·용량 문제라
//! 사용자 결정). 노출 지점: 런처 `?` 빈 질의 하단 + `kmd index --suggest`.
//!
//! 스캔은 홈 직계 하위 폴더를 후보로, 깊이·엔트리 예산을 걸고 최근 N일 내
//! 수정된 본문 인덱싱 대상 파일 수를 센다. 예산 상한 덕에 최악의 경우에도
//! 수십 ms 수준이며, 결과는 세션 캐시(TTL 10분)로 재사용된다.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use crate::config::{Config, LauncherConfig};
use crate::index::{IndexItem, ItemKind, Source};
use crate::search::SearchResult;

/// Enter로 검색 범위에 추가하는 제안 항목의 keywords 마커.
/// 형식: `kmd:suggest:add:<절대경로>`
pub const SUGGEST_MARKER: &str = "kmd:suggest:add:";

/// "최근"으로 간주하는 수정 시점 (일)
const RECENT_DAYS: u64 = 14;
/// 제안 최소 기준 — 최근 파일이 이보다 적으면 소음으로 본다
const MIN_RECENT_FILES: usize = 5;
/// 후보 폴더 내부 스캔 깊이
const SCAN_DEPTH: usize = 3;
/// 전체 후보 합산 스캔 엔트리 예산 — 키 입력 경로에서 호출되므로 상한 필수
const ENTRY_BUDGET: usize = 12_000;
/// 세션 캐시 TTL
const CACHE_TTL: Duration = Duration::from_secs(600);

/// 제안 1건.
#[derive(Debug, Clone)]
pub struct FolderSuggestion {
    pub path: PathBuf,
    /// 최근 RECENT_DAYS일 내 수정된 인덱싱 대상 파일 수
    pub recent_files: usize,
}

/// 홈 디렉터리 (HOME → USERPROFILE 폴백).
fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// 후보 폴더를 스캔해 제안 목록을 만든다 (실제 홈 기준).
pub fn suggest_folders(launcher: &LauncherConfig, max: usize) -> Vec<FolderSuggestion> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    suggest_folders_in(&home, launcher, SystemTime::now(), max)
}

/// 현재 검색 범위와 겹치는(조상/자손 어느 쪽이든) 폴더인가.
/// 스캔 시점과 캐시 반환 시점 양쪽에서 쓴다 — Enter로 방금 추가한 폴더가
/// TTL이 남은 캐시에서 계속 제안되는 staleness를 막는다.
fn covered_by_search_paths(launcher: &LauncherConfig, path: &Path) -> bool {
    launcher
        .search_paths
        .iter()
        .any(|sp| sp.starts_with(path) || path.starts_with(sp))
}

/// 세션 캐시를 거친 제안 조회 — 런처 키 입력 경로용.
/// 첫 호출만 스캔 비용(예산 상한 내)을 내고, 이후 10분간 캐시를 반환한다.
/// 캐시 반환 시에도 현재 search_paths 기준으로 다시 걸러낸다.
pub fn cached_suggestions(launcher: &LauncherConfig, max: usize) -> Vec<FolderSuggestion> {
    static CACHE: Mutex<Option<(Instant, Vec<FolderSuggestion>)>> = Mutex::new(None);
    let mut guard = match CACHE.lock() {
        Ok(g) => g,
        Err(_) => return suggest_folders(launcher, max),
    };
    if let Some((at, cached)) = guard.as_ref() {
        if at.elapsed() < CACHE_TTL {
            return filter_covered(launcher, cached.iter().cloned(), max);
        }
    }
    let fresh = suggest_folders(launcher, max.max(5));
    let out = filter_covered(launcher, fresh.iter().cloned(), max);
    *guard = Some((Instant::now(), fresh));
    out
}

/// 검색 범위와 겹치는 항목을 걸러내고 max개까지 반환 (순수 함수 — 테스트용 분리).
fn filter_covered(
    launcher: &LauncherConfig,
    items: impl Iterator<Item = FolderSuggestion>,
    max: usize,
) -> Vec<FolderSuggestion> {
    items
        .filter(|s| !covered_by_search_paths(launcher, &s.path))
        .take(max)
        .collect()
}

/// 테스트 가능한 내부 구현 — 홈 경로와 현재 시각을 주입받는다.
fn suggest_folders_in(
    home: &Path,
    launcher: &LauncherConfig,
    now: SystemTime,
    max: usize,
) -> Vec<FolderSuggestion> {
    let ignore_set: HashSet<&str> = launcher
        .ignore_patterns
        .iter()
        .map(|s| s.as_str())
        .collect();
    let allowed = crate::content_index::allowed_extensions(&launcher.content_search);
    let recent_cutoff = now - Duration::from_secs(RECENT_DAYS * 24 * 3600);

    let mut budget = ENTRY_BUDGET;
    let mut suggestions: Vec<FolderSuggestion> = Vec::new();

    let Ok(entries) = std::fs::read_dir(home) else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        if budget == 0 {
            break;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // 숨김·시스템·무시 패턴 폴더는 후보에서 제외
        if name.starts_with('.') || name.starts_with('$') || ignore_set.contains(name.as_str()) {
            continue;
        }
        // 이미 검색 범위와 겹치는 폴더는 제외 (조상/자손 어느 쪽이든)
        if covered_by_search_paths(launcher, &path) {
            continue;
        }

        let count = count_recent_files(
            &path,
            &ignore_set,
            &allowed,
            &launcher.content_search,
            recent_cutoff,
            &mut budget,
        );
        if count >= MIN_RECENT_FILES {
            suggestions.push(FolderSuggestion {
                path,
                recent_files: count,
            });
        }
    }

    suggestions.sort_by_key(|s| std::cmp::Reverse(s.recent_files));
    suggestions.truncate(max);
    suggestions
}

/// 후보 폴더 내부에서 최근 수정된 인덱싱 대상 파일 수를 센다 (예산 차감).
fn count_recent_files(
    root: &Path,
    ignore_set: &HashSet<&str>,
    allowed: &HashSet<String>,
    cs: &crate::config::ContentSearchConfig,
    recent_cutoff: SystemTime,
    budget: &mut usize,
) -> usize {
    let mut count = 0usize;
    let walker = walkdir::WalkDir::new(root)
        .max_depth(SCAN_DEPTH)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !crate::index::files::is_ignored_dir(e, ignore_set));
    for entry in walker.flatten() {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_str().unwrap_or("");
        if name.starts_with('.') {
            continue;
        }
        let ext = Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        if !allowed.contains(&ext) {
            continue;
        }
        if crate::content_index::excluded_name(cs, &name.to_lowercase()) {
            continue;
        }
        let recent = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .is_some_and(|t| t >= recent_cutoff);
        if recent {
            count += 1;
        }
    }
    count
}

/// 런처 표시용 제안 항목 — `?` 빈 질의 하단에 붙는다.
/// Enter 시 [`SUGGEST_MARKER`] 처리기가 search_paths에 추가한다.
pub fn suggestion_results(
    launcher: &LauncherConfig,
    use_emoji: bool,
    max: usize,
) -> Vec<SearchResult> {
    cached_suggestions(launcher, max)
        .into_iter()
        .map(|s| {
            let display = display_path(&s.path);
            SearchResult {
                item: IndexItem {
                    name: format!(
                        "{display} 을(를) 검색 범위에 추가 — 최근 2주 문서 {}개",
                        s.recent_files
                    ),
                    path: "Enter로 추가하면 파일명·본문 검색이 이 폴더까지 확장됩니다".to_string(),
                    kind: ItemKind::SystemCommand,
                    source: Source::Plugin,
                    icon: if use_emoji { "\u{2795}" } else { "+ " }.to_string(),
                    keywords: format!("{SUGGEST_MARKER}{}", s.path.to_string_lossy()),
                    icon_path: None,
                },
                score: 0,
            }
        })
        .collect()
}

/// `~` 축약 표시.
fn display_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    if let Some(home) = home_dir() {
        let h = home.to_string_lossy();
        if let Some(rest) = s.strip_prefix(h.as_ref()) {
            return format!("~{rest}");
        }
    }
    s.to_string()
}

/// 제안 항목 Enter 처리 — search_paths에 추가하고 config를 저장한다.
/// 저장까지 성공하면 안내 메시지를 반환한다 (keymap::execute_keymap_action 패턴).
///
/// 저장에 성공한 뒤에만 호출자의 설정을 갱신한다. 예전에는 메모리를 먼저 고치고
/// 저장해서, 실패하면 화면상 추가된 경로가 메모리에만 남았다. 또 디스크의 최신
/// 설정 위에 이 변경만 얹으므로, 그 사이 다른 곳에서 바뀐 값을 되돌리지 않는다.
pub fn execute_suggest_action(config: &mut Config, keywords: &str) -> Option<String> {
    // 먼저 사본에 적용해 "추가할 것이 있는지"와 안내 문구를 얻는다.
    let mut probe = config.clone();
    let msg = apply_suggest_add(&mut probe, keywords)?;

    let Some(path) = config.config_path.clone() else {
        // 저장 경로를 모르면 메모리에만 반영한다 (테스트·임시 설정 경로).
        *config = probe;
        return Some(msg);
    };

    let added = keywords.strip_prefix(SUGGEST_MARKER)?.to_string();
    match Config::update_and_save(&path, |latest| {
        let pb = PathBuf::from(&added);
        if !latest.launcher.search_paths.contains(&pb) {
            latest.launcher.search_paths.push(pb);
        }
    }) {
        Ok(saved) => {
            *config = saved;
            Some(msg)
        }
        Err(e) => Some(format!("설정 저장 실패: {e}")),
    }
}

/// 순수 적용부 (저장 없음) — 테스트용 분리.
fn apply_suggest_add(config: &mut Config, keywords: &str) -> Option<String> {
    let path = keywords.strip_prefix(SUGGEST_MARKER)?;
    if path.is_empty() {
        return None;
    }
    let pb = PathBuf::from(path);
    if !config.launcher.search_paths.contains(&pb) {
        config.launcher.search_paths.push(pb.clone());
    }
    Some(format!(
        "검색 범위에 추가됨: {} — 다음 인덱스 갱신부터 반영됩니다",
        display_path(&pb)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_file(dir: &Path, name: &str, content: &str) {
        let mut f = std::fs::File::create(dir.join(name)).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    fn launcher_with_paths(paths: Vec<PathBuf>) -> LauncherConfig {
        LauncherConfig {
            search_paths: paths,
            ..Default::default()
        }
    }

    #[test]
    fn 활동_폴더_제안과_임계값() {
        let home = tempfile::tempdir().unwrap();
        // 활발한 폴더: md 6개 (방금 생성 = 최근)
        let busy = home.path().join("projects");
        std::fs::create_dir(&busy).unwrap();
        for i in 0..6 {
            write_file(&busy, &format!("n{i}.md"), "notes");
        }
        // 소음 폴더: 2개뿐 → 임계값 미달
        let quiet = home.path().join("misc");
        std::fs::create_dir(&quiet).unwrap();
        write_file(&quiet, "a.md", "x");
        write_file(&quiet, "b.md", "x");

        let s = suggest_folders_in(
            home.path(),
            &launcher_with_paths(vec![]),
            SystemTime::now(),
            5,
        );
        assert_eq!(s.len(), 1, "임계값(5) 이상만 제안: {s:?}");
        assert!(s[0].path.ends_with("projects"));
        assert_eq!(s[0].recent_files, 6);
    }

    #[test]
    fn 검색_범위와_겹치면_제외() {
        let home = tempfile::tempdir().unwrap();
        let covered = home.path().join("docs");
        std::fs::create_dir(&covered).unwrap();
        for i in 0..8 {
            write_file(&covered, &format!("n{i}.md"), "notes");
        }
        let s = suggest_folders_in(
            home.path(),
            &launcher_with_paths(vec![covered.clone()]),
            SystemTime::now(),
            5,
        );
        assert!(s.is_empty(), "이미 search_paths에 있으면 제안 안 함");

        // 하위 폴더가 이미 범위에 있어도 조상 폴더는 제안하지 않는다
        let sub = covered.join("sub");
        let s2 = suggest_folders_in(
            home.path(),
            &launcher_with_paths(vec![sub]),
            SystemTime::now(),
            5,
        );
        assert!(s2.is_empty());
    }

    #[test]
    fn 숨김·무시_폴더는_후보_제외() {
        let home = tempfile::tempdir().unwrap();
        for name in [".hidden", "node_modules", "Library"] {
            let d = home.path().join(name);
            std::fs::create_dir(&d).unwrap();
            for i in 0..8 {
                write_file(&d, &format!("n{i}.md"), "notes");
            }
        }
        let s = suggest_folders_in(
            home.path(),
            &launcher_with_paths(vec![]),
            SystemTime::now(),
            5,
        );
        assert!(s.is_empty(), "{s:?}");
    }

    #[test]
    fn 캐시_반환도_현재_검색범위로_필터() {
        let a = PathBuf::from("/tmp/kmd-suggest-test-a");
        let items = vec![FolderSuggestion {
            path: a.clone(),
            recent_files: 9,
        }];
        let empty = launcher_with_paths(vec![]);
        assert_eq!(
            filter_covered(&empty, items.clone().into_iter(), 5).len(),
            1
        );

        // Enter로 방금 추가된 폴더는 TTL이 남은 캐시에서도 제안이 사라져야 한다
        let covering = launcher_with_paths(vec![a]);
        assert!(filter_covered(&covering, items.into_iter(), 5).is_empty());
    }

    #[test]
    fn 제안_항목_변환과_추가_적용() {
        let home = tempfile::tempdir().unwrap();
        let busy = home.path().join("work");
        std::fs::create_dir(&busy).unwrap();

        // suggestion_results 대신 마커 적용 로직을 직접 검증 (실제 config 저장 없음)
        let mut config = Config::default();
        let before = config.launcher.search_paths.len();
        let kw = format!("{SUGGEST_MARKER}{}", busy.to_string_lossy());
        let msg = apply_suggest_add(&mut config, &kw).expect("적용 성공");
        assert!(msg.contains("추가됨"));
        assert_eq!(config.launcher.search_paths.len(), before + 1);
        assert_eq!(config.launcher.search_paths.last().unwrap(), &busy);

        // 중복 적용해도 한 번만
        apply_suggest_add(&mut config, &kw);
        assert_eq!(config.launcher.search_paths.len(), before + 1);

        // 무관한 마커는 None
        assert!(apply_suggest_add(&mut config, "kmd:help:entry").is_none());
    }
}
