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

/// 제안 근거 — 왜 이 폴더를 권하는지. 문구가 달라야 사용자가 납득한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestReason {
    /// 신호 ① 변경 활동 — 최근 [`RECENT_DAYS`]일 내 수정된 문서 수
    RecentActivity(usize),
    /// 신호 ② 실행 이력 — 이 폴더 아래 항목을 연 누적 횟수.
    /// 사용자가 **실제로 쓰는** 폴더라는 더 직접적인 증거다.
    LaunchHistory(usize),
}

/// 제안 1건.
#[derive(Debug, Clone)]
pub struct FolderSuggestion {
    pub path: PathBuf,
    pub reason: SuggestReason,
}

impl FolderSuggestion {
    /// 정렬용 점수. 실행 이력은 "직접 썼다"는 증거라 같은 수치여도 더 높게 친다.
    pub fn score(&self) -> usize {
        match self.reason {
            SuggestReason::RecentActivity(n) => n,
            SuggestReason::LaunchHistory(n) => n.saturating_mul(3),
        }
    }

    /// 제안 이유를 사람이 읽는 문구로.
    pub fn reason_label(&self) -> String {
        match self.reason {
            SuggestReason::RecentActivity(n) => format!("최근 2주 문서 {n}개"),
            SuggestReason::LaunchHistory(n) => format!("여기서 {n}번 열었음"),
        }
    }
}

/// 홈 디렉터리 (HOME → USERPROFILE 폴백).
fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// 제안 후보를 찾을 루트 목록.
///
/// 홈 직계만 보면 **홈 밖에서 일하는 사용자에게는 아무것도 제안하지 못한다.**
/// Windows에서 `D:\\work`처럼 다른 드라이브에 작업 폴더를 두는 구성이 흔한데,
/// 그 경우 기본 검색 경로(Desktop/Documents/Downloads/OneDrive)에도 없고 제안
/// 후보에도 없어 검색이 통째로 비는 것처럼 보인다 (실사용 보고).
/// 그래서 홈과 함께 드라이브·볼륨 루트도 후보 루트로 본다.
fn candidate_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = home_dir() {
        roots.push(home);
    }

    #[cfg(target_os = "windows")]
    {
        for letter in 'A'..='Z' {
            let drive = PathBuf::from(format!("{letter}:\\"));
            if drive.is_dir() {
                roots.push(drive);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(entries) = std::fs::read_dir("/Volumes") {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    roots.push(p);
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        for mp in ["/mnt", "/media"] {
            if let Ok(entries) = std::fs::read_dir(mp) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        roots.push(p);
                    }
                }
            }
        }
    }

    roots.sort();
    roots.dedup();
    roots
}

/// 드라이브 루트 직계에 있는 OS/시스템 폴더 — 제안 후보로 부적절하다.
/// (홈은 별도 루트로 스캔하므로 `Users`도 여기서 거른다)
fn is_system_dir_name(name: &str) -> bool {
    const SYSTEM_DIRS: &[&str] = &[
        "Windows",
        "Program Files",
        "Program Files (x86)",
        "ProgramData",
        "System Volume Information",
        "Recovery",
        "PerfLogs",
        "Users",
        "Library",
        "System",
        "Applications",
        "node_modules",
        "bin",
        "sbin",
        "usr",
        "var",
        "etc",
        "opt",
        "proc",
        "dev",
        "tmp",
    ];
    SYSTEM_DIRS.iter().any(|d| d.eq_ignore_ascii_case(name))
}

/// 실행 이력에서 "자주 쓰는 폴더"를 뽑는다 (신호 ②, docs/15).
///
/// 변경 활동(신호 ①)은 "파일이 자주 바뀌는 곳"을 찾지만, 사용자가 **실제로
/// 여는** 폴더와는 다를 수 있다. 이력은 더 직접적이다 — `:f`로 폴더를 직접
/// 지정해 열었거나 드릴다운으로 들어간 경로가 그대로 남기 때문에, 검색 범위
/// 밖에서 수동으로 파일을 찾아 쓰고 있던 폴더가 정확히 드러난다.
///
/// 파일은 부모 폴더로, 폴더는 자기 자신으로 집계한다.
fn suggest_from_history(
    db: &crate::db::Database,
    launcher: &LauncherConfig,
    max: usize,
) -> Vec<FolderSuggestion> {
    /// 집계에 쓸 이력 상한 — frequency 내림차순이라 상위만 봐도 충분하다.
    const HISTORY_SCAN: usize = 500;
    /// 제안 최소 기준 — 한두 번 연 폴더까지 권하면 소음이 된다.
    const MIN_LAUNCHES: usize = 3;

    let mut by_parent: std::collections::HashMap<PathBuf, usize> = std::collections::HashMap::new();

    for h in db.query_history(HISTORY_SCAN) {
        // 경로가 있는 항목만. item_type 표기가 데스크톱(소문자)과 TUI(대문자)로
        // 갈려 있어 대소문자를 무시한다.
        let kind = h.item_type.to_ascii_lowercase();
        let is_dir = kind == "dir" || kind == "directory";
        if !(is_dir || kind == "file") {
            continue;
        }
        let p = PathBuf::from(&h.value);
        if !p.is_absolute() {
            continue;
        }
        let folder = if is_dir {
            p
        } else {
            match p.parent() {
                Some(parent) => parent.to_path_buf(),
                None => continue,
            }
        };
        if !folder.is_dir() {
            continue; // 지워졌거나 외장이 빠진 경로
        }
        *by_parent.entry(folder).or_insert(0) += h.frequency.max(1) as usize;
    }

    let mut out: Vec<FolderSuggestion> = by_parent
        .into_iter()
        .filter(|(path, n)| *n >= MIN_LAUNCHES && !covered_by_search_paths(launcher, path))
        .map(|(path, n)| FolderSuggestion {
            path,
            reason: SuggestReason::LaunchHistory(n),
        })
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.score()));
    out.truncate(max);
    out
}

/// 기본 위치의 이력 DB를 열어 신호 ②를 구한다 (실패하면 빈 목록).
fn history_suggestions(launcher: &LauncherConfig, max: usize) -> Vec<FolderSuggestion> {
    let path = crate::Config::default_data_dir().join(crate::DB_FILENAME);
    if !path.exists() {
        return Vec::new();
    }
    match crate::db::Database::open(&path) {
        Ok(db) => suggest_from_history(&db, launcher, max),
        Err(e) => {
            tracing::debug!("이력 기반 폴더 제안 건너뜀: {e}");
            Vec::new()
        }
    }
}

/// 후보 폴더를 스캔해 제안 목록을 만든다 (홈 + 드라이브/볼륨 루트).
///
/// 스캔 예산([`ENTRY_BUDGET`])은 **전체 루트가 공유**한다 — 루트가 늘어도
/// 키 입력 경로의 최악 비용이 그대로 유지된다.
pub fn suggest_folders(launcher: &LauncherConfig, max: usize) -> Vec<FolderSuggestion> {
    let now = SystemTime::now();
    let mut budget = ENTRY_BUDGET;

    // 신호 ② 실행 이력 — 사용자가 실제로 연 폴더. 더 직접적인 증거라 먼저 둔다.
    let mut all: Vec<FolderSuggestion> = history_suggestions(launcher, max.max(5));
    let seen: HashSet<PathBuf> = all.iter().map(|s| s.path.clone()).collect();

    // 신호 ① 변경 활동 — 아직 열어본 적 없지만 파일이 활발히 바뀌는 폴더.
    for root in candidate_roots() {
        if budget == 0 {
            break;
        }
        for s in scan_root(&root, launcher, now, &mut budget) {
            // 같은 폴더가 양쪽에 잡히면 이력 쪽을 남긴다 (문구가 더 설득력 있다)
            if !seen.contains(&s.path) {
                all.push(s);
            }
        }
    }

    all.sort_by_key(|s| std::cmp::Reverse(s.score()));
    all.truncate(max);
    all
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
    let mut guard = match SUGGEST_CACHE.lock() {
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

/// 제안 세션 캐시 (스캔 결과, 마지막 계산 시각).
static SUGGEST_CACHE: Mutex<Option<(Instant, Vec<FolderSuggestion>)>> = Mutex::new(None);
/// 백그라운드 워밍이 이미 돌고 있는지 — 매 키 입력마다 스레드를 띄우지 않는다.
static WARMING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// **UI 경로 전용** — 캐시에 있으면 돌려주고, 없으면 백그라운드로 채운 뒤
/// 이번에는 빈 목록을 반환한다.
///
/// 제안 계산은 파일시스템 스캔이라 키 입력 경로에서 동기로 돌리면 안 된다.
/// 후보 루트에 외장·네트워크 볼륨이 들어가면 한 번의 검색이 수 초씩 멈춘다
/// (실측: 이 함수를 만들기 전 테스트에서 검색 한 번이 14초까지 늘어졌다).
/// 제안은 다음 검색부터 나타나며, 그 지연은 사용자에게 무해하다.
pub fn cached_suggestions_nonblocking(
    launcher: &LauncherConfig,
    max: usize,
) -> Vec<FolderSuggestion> {
    use std::sync::atomic::Ordering;

    if let Ok(guard) = SUGGEST_CACHE.lock() {
        if let Some((at, cached)) = guard.as_ref() {
            if at.elapsed() < CACHE_TTL {
                return filter_covered(launcher, cached.iter().cloned(), max);
            }
        }
    }

    // 캐시가 없거나 낡았다 — 한 번만 백그라운드 워밍을 띄운다.
    if !WARMING.swap(true, Ordering::AcqRel) {
        let cfg = launcher.clone();
        std::thread::spawn(move || {
            let fresh = suggest_folders(&cfg, 5);
            if let Ok(mut g) = SUGGEST_CACHE.lock() {
                *g = Some((Instant::now(), fresh));
            }
            WARMING.store(false, Ordering::Release);
        });
    }
    Vec::new()
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
#[cfg(test)]
fn suggest_folders_in(
    home: &Path,
    launcher: &LauncherConfig,
    now: SystemTime,
    max: usize,
) -> Vec<FolderSuggestion> {
    let mut budget = ENTRY_BUDGET;
    let mut s = scan_root(home, launcher, now, &mut budget);
    s.sort_by_key(|x| std::cmp::Reverse(x.score()));
    s.truncate(max);
    s
}

/// 루트 하나의 **직계 하위 폴더**를 후보로 스캔한다.
/// 정렬·절단은 호출자가 한다 (여러 루트 결과를 합쳐야 하므로).
/// `budget`은 루트 간 공유된다.
fn scan_root(
    root: &Path,
    launcher: &LauncherConfig,
    now: SystemTime,
    budget: &mut usize,
) -> Vec<FolderSuggestion> {
    let ignore_set: HashSet<&str> = launcher
        .ignore_patterns
        .iter()
        .map(|s| s.as_str())
        .collect();
    let allowed = crate::content_index::allowed_extensions(&launcher.content_search);
    let recent_cutoff = now - Duration::from_secs(RECENT_DAYS * 24 * 3600);

    let mut suggestions: Vec<FolderSuggestion> = Vec::new();

    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        if *budget == 0 {
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
        // 드라이브 루트 직계의 OS 폴더(Windows, Program Files, Users…)는 후보가 아니다
        if is_system_dir_name(&name) {
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
            budget,
        );
        if count >= MIN_RECENT_FILES {
            suggestions.push(FolderSuggestion {
                path,
                reason: SuggestReason::RecentActivity(count),
            });
        }
    }

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
    // UI 경로 — 절대 블로킹하지 않는다 (cached_suggestions_nonblocking 참조)
    cached_suggestions_nonblocking(launcher, max)
        .into_iter()
        .map(|s| {
            let display = display_path(&s.path);
            SearchResult {
                item: IndexItem {
                    name: format!("{display} 을(를) 검색 범위에 추가 — {}", s.reason_label()),
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

    // ── 신호 ② 실행 이력: 자주 연 폴더를 권한다 ───────────────────────
    //
    // docs/15에서 "후속 여지"로 남겨둔 신호. 변경 활동(①)은 파일이 바뀌는 곳을
    // 찾지만, 사용자가 실제로 **여는** 폴더는 이력에만 드러난다.

    #[test]
    fn 자주_연_폴더를_이력에서_뽑는다() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir(&work).unwrap();
        write_file(&work, "a.md", "x");
        let file = work.join("a.md");

        let db = crate::db::Database::open_in_memory().unwrap();
        // 같은 파일을 세 번 열었다 → 부모 폴더가 후보
        for _ in 0..3 {
            db.record_launch("file", &file.to_string_lossy(), Some("a.md"))
                .unwrap();
        }

        let got = suggest_from_history(&db, &launcher_with_paths(vec![]), 5);
        assert_eq!(got.len(), 1, "부모 폴더가 제안돼야 한다: {got:?}");
        assert_eq!(got[0].path, work);
        assert!(
            matches!(got[0].reason, SuggestReason::LaunchHistory(n) if n >= 3),
            "이력 신호여야 한다: {:?}",
            got[0].reason
        );
    }

    #[test]
    fn 한두번_연_폴더는_제안하지_않는다() {
        let dir = tempfile::tempdir().unwrap();
        let rare = dir.path().join("rare");
        std::fs::create_dir(&rare).unwrap();
        write_file(&rare, "b.md", "x");

        let db = crate::db::Database::open_in_memory().unwrap();
        db.record_launch("file", &rare.join("b.md").to_string_lossy(), None)
            .unwrap();

        let got = suggest_from_history(&db, &launcher_with_paths(vec![]), 5);
        assert!(got.is_empty(), "1회 실행은 소음이다: {got:?}");
    }

    #[test]
    fn 이미_검색범위인_폴더는_이력에서도_제외된다() {
        let dir = tempfile::tempdir().unwrap();
        let covered = dir.path().join("docs");
        std::fs::create_dir(&covered).unwrap();
        write_file(&covered, "c.md", "x");

        let db = crate::db::Database::open_in_memory().unwrap();
        for _ in 0..5 {
            db.record_launch("file", &covered.join("c.md").to_string_lossy(), None)
                .unwrap();
        }

        let got = suggest_from_history(&db, &launcher_with_paths(vec![covered.clone()]), 5);
        assert!(got.is_empty(), "이미 검색되는 폴더는 권할 필요가 없다");
    }

    #[test]
    fn 앱_웹_항목은_폴더_집계에서_빠진다() {
        let db = crate::db::Database::open_in_memory().unwrap();
        for _ in 0..9 {
            db.record_launch("app", "firefox", Some("Firefox")).unwrap();
            db.record_launch("Web", "https://example.com", None)
                .unwrap();
        }
        let got = suggest_from_history(&db, &launcher_with_paths(vec![]), 5);
        assert!(got.is_empty(), "경로가 아닌 항목이 섞였다: {got:?}");
    }

    #[test]
    fn item_type_대소문자가_달라도_집계된다() {
        // 데스크톱은 "file", TUI는 "File"로 기록한다 — 둘 다 잡아야 한다
        let dir = tempfile::tempdir().unwrap();
        let w = dir.path().join("mixed");
        std::fs::create_dir(&w).unwrap();
        write_file(&w, "d.md", "x");
        let f = w.join("d.md").to_string_lossy().to_string();

        let db = crate::db::Database::open_in_memory().unwrap();
        db.record_launch("file", &f, None).unwrap();
        db.record_launch("File", &f, None).unwrap();
        db.record_launch("FILE", &f, None).unwrap();

        let got = suggest_from_history(&db, &launcher_with_paths(vec![]), 5);
        assert_eq!(got.len(), 1, "대소문자 때문에 누락됐다: {got:?}");
    }

    #[test]
    fn 이력_신호가_활동_신호보다_우선한다() {
        let a = FolderSuggestion {
            path: PathBuf::from("/tmp/by-history"),
            reason: SuggestReason::LaunchHistory(4),
        };
        let b = FolderSuggestion {
            path: PathBuf::from("/tmp/by-activity"),
            reason: SuggestReason::RecentActivity(10),
        };
        assert!(
            a.score() > b.score(),
            "실제로 연 폴더가 먼저 와야 한다 ({} vs {})",
            a.score(),
            b.score()
        );
    }

    // ── 홈 밖 루트도 후보가 된다 (Windows D:\work 같은 구성) ──────────
    //
    // 예전에는 read_dir(home) 하나만 봐서, 홈 밖에 작업 폴더를 둔 사용자에게는
    // 제안이 영원히 비어 있었다 — 윈도우에서 검색이 안 된다는 보고의 원인.

    #[test]
    fn 시스템_폴더는_후보에서_제외된다() {
        let root = tempfile::tempdir().unwrap();
        // 드라이브 루트 직계의 OS 폴더를 흉내 — 문서가 많아도 제안하면 안 된다
        for sys in ["Windows", "Program Files", "Users"] {
            let d = root.path().join(sys);
            std::fs::create_dir(&d).unwrap();
            for i in 0..8 {
                write_file(&d, &format!("f{i}.md"), "x");
            }
        }
        // 진짜 작업 폴더
        let work = root.path().join("work");
        std::fs::create_dir(&work).unwrap();
        for i in 0..8 {
            write_file(&work, &format!("n{i}.md"), "x");
        }

        let mut budget = ENTRY_BUDGET;
        let got = scan_root(
            root.path(),
            &launcher_with_paths(vec![]),
            SystemTime::now(),
            &mut budget,
        );
        let names: Vec<String> = got
            .iter()
            .map(|s| s.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            names,
            vec!["work".to_string()],
            "시스템 폴더가 섞였다: {names:?}"
        );
    }

    #[test]
    fn 예산은_루트_사이에_공유된다() {
        let a = tempfile::tempdir().unwrap();
        let busy = a.path().join("docs");
        std::fs::create_dir(&busy).unwrap();
        for i in 0..8 {
            write_file(&busy, &format!("n{i}.md"), "x");
        }

        // 예산이 0이면 더 스캔하지 않는다 — 루트가 늘어도 최악 비용이 고정된다
        let mut budget = 0usize;
        let got = scan_root(
            a.path(),
            &launcher_with_paths(vec![]),
            SystemTime::now(),
            &mut budget,
        );
        assert!(got.is_empty(), "예산 소진 후에는 스캔하지 않아야 한다");
    }

    #[test]
    fn 후보_루트에_홈이_포함된다() {
        let roots = candidate_roots();
        assert!(!roots.is_empty(), "후보 루트가 비어 있다");
        if let Some(home) = home_dir() {
            assert!(roots.contains(&home), "홈이 후보 루트에 없다: {roots:?}");
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
        assert_eq!(s[0].reason, SuggestReason::RecentActivity(6));
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
            reason: SuggestReason::RecentActivity(9),
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
