//! `:f /경로 쿼리` — 특정 폴더 내 즉석 파일 검색 (TUI/데스크톱 공용)

use crate::index::{IndexItem, ItemKind, Source};
use crate::search::SearchResult;

/// 한 번의 검색에서 살펴볼 디렉터리 항목 상한.
///
/// 이 검색은 **키 입력마다** UI 스레드에서 돈다. 상한이 없으면 항목 수만큼
/// 시간이 늘고, 외장·네트워크 볼륨이면 수 초까지 멈춘다 — 폴더 제안
/// (`folder_suggest`)이 이미 "검색 한 번이 14초"를 겪고 예산을 넣은 문제다.
/// 5천 개면 로컬 디스크에서 수 ms 안쪽이고, 그보다 큰 폴더는 검색어로 좁히는
/// 게 맞다(잘렸다는 안내 항목을 붙인다).
pub const MAX_ENTRIES: usize = 5_000;

/// 폴더 한 단계 안의 항목.
pub struct ListedEntry {
    pub name: String,
    pub path: std::path::PathBuf,
    pub is_dir: bool,
}

/// 폴더 한 단계를 나열한 결과.
pub struct DirListing {
    pub entries: Vec<ListedEntry>,
    /// `max_entries`에 걸려 뒷부분을 보지 않았다.
    pub truncated: bool,
}

/// 폴더 한 단계를 나열한다 — `:f`와 TUI 드릴다운이 공유한다.
///
/// 예전엔 TUI 드릴다운이 이 루프의 복사본을 들고 있어서 상한·숨김 규칙·stat
/// 절약 같은 개선이 한쪽에만 들어갔다. OS마다 다른 부분은 여기서 처리한다:
/// - 숨김: `.` 이름 + Windows 숨김 속성 ([`crate::fsutil::is_hidden_entry`])
/// - 폴더 판별: `DirEntry::file_type`은 열거가 이미 준 정보라 추가 stat이 없다
///   (`path.is_dir()`는 항목마다 stat — 네트워크 볼륨에서 가장 비싼 부분).
///   심볼릭 링크만 대상을 따라가 폴더 링크를 폴더로 본다
pub fn list_dir(dir: &std::path::Path, max_entries: usize) -> std::io::Result<DirListing> {
    let mut entries = Vec::new();
    let mut truncated = false;
    for (seen, entry) in std::fs::read_dir(dir)?.flatten().enumerate() {
        if seen >= max_entries {
            truncated = true;
            break;
        }
        if crate::fsutil::is_hidden_entry(&entry) {
            continue;
        }
        let path = entry.path();
        let is_dir = match entry.file_type() {
            Ok(ft) if ft.is_symlink() => path.is_dir(),
            Ok(ft) => ft.is_dir(),
            Err(_) => path.is_dir(),
        };
        entries.push(ListedEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path,
            is_dir,
        });
    }
    Ok(DirListing { entries, truncated })
}

/// 잘렸다는 안내 항목 — `:f`와 TUI 드릴다운이 같은 문구를 쓴다.
pub fn truncated_notice(dir_display: &str) -> SearchResult {
    truncated_item(dir_display, MAX_ENTRIES)
}

/// `:f` 이후의 입력을 파싱해 폴더 내 검색 결과를 만든다.
///
/// 형식: `:f /path/to/dir 검색어` 또는 `:f ~/dir 검색어`
/// - 입력이 비어 있으면 사용법 안내 항목을 반환
/// - 경로가 없으면 오류 안내 항목을 반환
/// - 검색어 없이 경로만 있으면 최상위 목록을 반환
pub fn folder_search_results(query: &str, use_emoji: bool) -> Vec<SearchResult> {
    folder_search_with_budget(query, use_emoji, MAX_ENTRIES)
}

fn folder_search_with_budget(
    query: &str,
    use_emoji: bool,
    max_entries: usize,
) -> Vec<SearchResult> {
    let after_prefix = query.strip_prefix(":f").unwrap_or(query).trim();

    if after_prefix.is_empty() {
        return vec![help_item()];
    }

    let (dir_part, name_query) = split_dir_and_query(after_prefix, |p| resolve_dir(p).is_dir());
    let dir_buf = resolve_dir(dir_part);
    let dir = dir_buf.as_path();
    if !dir.is_dir() {
        return vec![not_found_item(dir_part)];
    }

    // 폴더 내 항목 열거 (1단계)
    // 검색어·파일명 모두 유니코드 소문자로 — 예전엔 검색어만 유니코드, 파일명은
    // ASCII 소문자라 `Ä` 같은 문자에서 어긋났다.
    let query_lower = name_query.to_lowercase();
    let listing = list_dir(dir, max_entries).unwrap_or(DirListing {
        entries: Vec::new(),
        truncated: false,
    });
    let truncated = listing.truncated;
    let mut results: Vec<SearchResult> = Vec::new();

    for entry in listing.entries {
        let name_lower = entry.name.to_lowercase();
        if !query_lower.is_empty() && !name_lower.contains(query_lower.as_str()) {
            continue;
        }
        let (kind, icon) = match (entry.is_dir, use_emoji) {
            (true, true) => (ItemKind::Directory, "\u{1F4C1}"),
            (true, false) => (ItemKind::Directory, "D/"),
            (false, true) => (ItemKind::File, "\u{1F4C4}"),
            (false, false) => (ItemKind::File, "F "),
        };
        // 검색어와 얼마나 일치하는지 점수 부여 (이름 접두사 일치 우대)
        let score: u32 = if query_lower.is_empty() {
            0
        } else if name_lower.starts_with(query_lower.as_str()) {
            20
        } else {
            10
        };
        results.push(SearchResult {
            item: IndexItem {
                name: entry.name,
                path: entry.path.to_string_lossy().into_owned(),
                kind,
                source: Source::FileProvider,
                icon: icon.to_string(),
                keywords: String::new(),
                icon_path: None,
            },
            score,
        });
    }

    // 점수 내림차순 → 같은 점수는 폴더 우선 → 이름 오름차순
    results.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| {
                let a_dir = a.item.kind == ItemKind::Directory;
                let b_dir = b.item.kind == ItemKind::Directory;
                b_dir.cmp(&a_dir)
            })
            .then(a.item.name.cmp(&b.item.name))
    });

    if results.is_empty() && !truncated {
        results.push(no_match_item(dir_part, name_query));
    }
    // 잘렸으면 반드시 알린다 — "없다"와 "앞부분만 봤다"는 다른 상황이다.
    if truncated {
        results.push(truncated_item(dir_part, max_entries));
    }

    results
}

fn truncated_item(dir: &str, max_entries: usize) -> SearchResult {
    SearchResult {
        item: IndexItem {
            name: format!("항목이 많아 앞의 {max_entries}개만 살펴봤습니다"),
            path: format!("{dir} — 검색어를 더 입력하거나 하위 폴더로 좁혀 보세요"),
            kind: ItemKind::SystemCommand,
            source: Source::Plugin,
            icon: "\u{2139}\u{FE0F}".to_string(),
            keywords: "kmd:folder_search:truncated".to_string(),
            icon_path: None,
        },
        score: 0,
    }
}

/// 공백 경계를 몇 군데까지 경로 후보로 시험할지 — 키 입력마다 stat이 이 수만큼 돈다.
const MAX_PATH_CANDIDATES: usize = 8;

/// `:f` 뒤의 입력을 (폴더, 검색어)로 나눈다.
///
/// 예전에는 **첫 공백**에서 잘랐다. Windows에선 `D:\My Projects`,
/// `OneDrive - 회사`처럼 공백 있는 폴더가 흔해 `D:\My`만 보고 "없는 폴더"가 됐다.
/// - `"..."`로 감싸면 따옴표 안이 경로다(닫는 따옴표 전이면 끝까지 — 타이핑 중).
/// - 아니면 공백 경계 중 **가장 긴, 실제로 존재하는 폴더**를 경로로 본다.
///   `D:\My`와 `D:\My Projects`가 둘 다 있으면 긴 쪽이 이긴다.
/// - 어느 것도 없으면 첫 토큰을 경로로 돌려준다 — "찾을 수 없음"에 그 값이 뜬다.
fn split_dir_and_query(input: &str, is_dir: impl Fn(&str) -> bool) -> (&str, &str) {
    let input = input.trim();
    if let Some(rest) = input.strip_prefix('"') {
        return match rest.find('"') {
            Some(end) => (&rest[..end], rest[end + 1..].trim()),
            None => (rest, ""),
        };
    }

    // 후보: 입력 전체, 그다음 공백 경계를 뒤에서부터.
    let mut cuts: Vec<usize> = input.match_indices(' ').map(|(i, _)| i).collect();
    cuts.push(input.len());
    for &cut in cuts.iter().rev().take(MAX_PATH_CANDIDATES) {
        let candidate = input[..cut].trim_end();
        if !candidate.is_empty() && is_dir(candidate) {
            return (candidate, input[cut..].trim());
        }
    }

    match input.find(' ') {
        Some(pos) => (&input[..pos], input[pos + 1..].trim()),
        None => (input, ""),
    }
}

/// 사용자가 친 폴더 문자열을 실제 경로로 바꾼다 (`~` 확장 + 구분자 정리).
fn resolve_dir(part: &str) -> std::path::PathBuf {
    let expanded = match part.strip_prefix('~') {
        // `~`, `~/x`, `~\x`만 홈으로 본다 (`~user`는 건드리지 않는다).
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => match dirs::home_dir() {
            Some(home) => format!("{}{rest}", home.display()),
            None => part.to_string(),
        },
        _ => part.to_string(),
    };
    std::path::PathBuf::from(native_separators(&expanded))
}

/// Windows에선 `/`를 `\`로 바꾼다.
///
/// 결과 항목의 경로는 이 폴더를 기준으로 만들어진다. `~/Documents`가
/// `C:\Users\me/Documents`처럼 섞인 채로 들어가면, 탐색기(`explorer.exe`)는
/// 그런 경로를 알아보지 못하고 **기본 폴더를 연다** — 어떤 항목을 골라도 같은
/// 창이 뜨던 원인이다. 다른 OS에서는 `\`가 파일명 문자일 수 있어 건드리지 않는다.
fn native_separators(s: &str) -> String {
    if cfg!(windows) {
        to_windows_separators(s)
    } else {
        s.to_string()
    }
}

/// `/`→`\` 변환 + 드라이브 상대 경로를 루트 기준으로 보정한다.
///
/// `d:`는 Windows에서 "D 드라이브의 **현재 폴더**"라는 뜻이라, `:f d:`의 하위
/// 항목이 `d:My Data`처럼 만들어졌다. 탐색기는 이런 경로를 알아보지 못하고
/// 기본 폴더(문서)를 열어, 어떤 폴더를 골라도 엉뚱한 곳이 열렸다. 런처에는
/// 드라이브별 현재 폴더라는 개념이 의미 없으니 `d:` → `d:\`, `d:temp` →
/// `d:\temp`로 본다.
fn to_windows_separators(s: &str) -> String {
    let s = s.replace('/', "\\");
    let b = s.as_bytes();
    let drive_relative =
        b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && b.get(2) != Some(&b'\\');
    if drive_relative {
        format!("{}\\{}", &s[..2], &s[2..])
    } else {
        s
    }
}

fn help_item() -> SearchResult {
    SearchResult {
        item: IndexItem {
            name: ":f /경로  또는  :f ~/경로 검색어".to_string(),
            path: "Enter로 폴더를 열거나, 경로 뒤에 검색어를 입력해 파일을 찾으세요".to_string(),
            kind: ItemKind::SystemCommand,
            source: Source::Plugin,
            icon: "\u{1F4C2}".to_string(),
            keywords: "kmd:folder_search:hint".to_string(),
            icon_path: None,
        },
        score: 0,
    }
}

fn not_found_item(dir: &str) -> SearchResult {
    SearchResult {
        item: IndexItem {
            name: format!("폴더를 찾을 수 없음: {dir}"),
            path: "경로를 확인하세요 — 공백이 있으면 따옴표로 감싸도 됩니다: :f \"D:\\My Folder\" 검색어"
                .to_string(),
            kind: ItemKind::SystemCommand,
            source: Source::Plugin,
            icon: "\u{26A0}\u{FE0F}".to_string(),
            keywords: "kmd:folder_search:error".to_string(),
            icon_path: None,
        },
        score: 0,
    }
}

fn no_match_item(dir: &str, query: &str) -> SearchResult {
    SearchResult {
        item: IndexItem {
            name: format!("'{query}'에 해당하는 파일이 없습니다"),
            path: format!("검색 위치: {dir}"),
            kind: ItemKind::SystemCommand,
            source: Source::Plugin,
            icon: "\u{1F50D}".to_string(),
            keywords: "kmd:folder_search:empty".to_string(),
            icon_path: None,
        },
        score: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with_files(n: usize) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..n {
            std::fs::write(dir.path().join(format!("file{i:03}.txt")), b"").unwrap();
        }
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        dir
    }

    fn query_for(dir: &tempfile::TempDir, q: &str) -> String {
        format!(":f {} {q}", dir.path().display())
            .trim_end()
            .to_string()
    }

    fn is_marker(r: &SearchResult, marker: &str) -> bool {
        r.item.keywords == format!("kmd:folder_search:{marker}")
    }

    // ── 경로/검색어 분리 (Windows에서 "있는 폴더를 없다고" 하던 문제) ─────

    fn exists_in<'a>(dirs: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |p| dirs.contains(&p)
    }

    #[test]
    fn 공백있는_폴더는_가장_긴_존재하는_경로로_본다() {
        let is_dir = exists_in(&[r"D:\My", r"D:\My Projects"]);
        assert_eq!(
            split_dir_and_query(r"D:\My Projects report", &is_dir),
            (r"D:\My Projects", "report")
        );
        assert_eq!(
            split_dir_and_query(r"D:\My Projects", &is_dir),
            (r"D:\My Projects", "")
        );
        // 짧은 쪽만 맞으면 짧은 쪽 + 나머지가 검색어
        assert_eq!(
            split_dir_and_query(r"D:\My notes", &is_dir),
            (r"D:\My", "notes")
        );
    }

    #[test]
    fn 따옴표로_감싼_경로() {
        let never = |_: &str| false;
        assert_eq!(
            split_dir_and_query(r#""C:\My Folder" rep"#, never),
            (r"C:\My Folder", "rep")
        );
        // 닫는 따옴표 전(타이핑 중)이면 끝까지가 경로
        assert_eq!(
            split_dir_and_query(r#""C:\My Fo"#, never),
            (r"C:\My Fo", "")
        );
    }

    #[test]
    fn 어떤_후보도_없으면_첫_토큰을_경로로() {
        let never = |_: &str| false;
        assert_eq!(
            split_dir_and_query("/no/such report x", never),
            ("/no/such", "report x")
        );
    }

    #[test]
    fn 실제_공백폴더를_찾는다() {
        let dir = tempfile::tempdir().unwrap();
        let spaced = dir.path().join("my projects");
        std::fs::create_dir(&spaced).unwrap();
        std::fs::write(spaced.join("report.txt"), b"").unwrap();

        let q = format!(":f {} report", spaced.display());
        let results = folder_search_with_budget(&q, false, 100);
        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(results[0].item.name, "report.txt");
    }

    #[test]
    fn 윈도우_구분자_변환() {
        // explorer.exe는 섞인 경로를 알아보지 못해 기본 폴더를 연다.
        assert_eq!(
            to_windows_separators(r"C:\Users\me/Documents/x"),
            r"C:\Users\me\Documents\x"
        );
    }

    #[test]
    fn 드라이브만_쓰면_루트로_본다() {
        // `d:`는 "D의 현재 폴더"라 하위 항목이 `d:My Data`가 되어 탐색기가
        // 기본 폴더(문서)를 열던 회귀 방지
        assert_eq!(to_windows_separators("d:"), r"d:\");
        assert_eq!(to_windows_separators("D:temp"), r"D:\temp");
        assert_eq!(to_windows_separators("d:/Temp"), r"d:\Temp");
        assert_eq!(to_windows_separators(r"d:\Temp"), r"d:\Temp");
        // 드라이브 문자가 아닌 것은 그대로
        assert_eq!(to_windows_separators(r"\\server\share"), r"\\server\share");
        assert_eq!(to_windows_separators("~"), "~");
    }

    #[cfg(windows)]
    #[test]
    fn 드라이브_루트의_하위_항목은_절대경로() {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let results = folder_search_results(&format!(":f {drive}"), false);
        let dir = results
            .iter()
            .find(|r| r.item.kind == ItemKind::Directory)
            .expect("시스템 드라이브 루트에 폴더가 있어야 한다");
        assert!(
            dir.item.path.starts_with(&format!("{drive}\\")),
            "드라이브 상대 경로가 되면 안 됨: {}",
            dir.item.path
        );
    }

    #[test]
    fn 물결표는_홈으로_펼친다() {
        let home = dirs::home_dir().expect("테스트 환경에 홈이 있어야 한다");
        assert_eq!(resolve_dir("~"), home);
        assert_eq!(
            resolve_dir("~/sub"),
            std::path::PathBuf::from(native_separators(&format!("{}/sub", home.display())))
        );
        // `~user`는 다른 사용자의 홈이라는 뜻일 수 있어 건드리지 않는다
        assert_eq!(resolve_dir("~other"), std::path::PathBuf::from("~other"));
    }

    #[test]
    fn 상한_이내면_전부_나오고_잘림_안내_없음() {
        let dir = dir_with_files(5);
        let results = folder_search_with_budget(&query_for(&dir, ""), false, 100);
        assert_eq!(results.len(), 6, "파일 5 + 폴더 1");
        assert!(!results.iter().any(|r| is_marker(r, "truncated")));
    }

    #[test]
    fn 상한을_넘으면_멈추고_잘림을_알린다() {
        let dir = dir_with_files(20);
        let results = folder_search_with_budget(&query_for(&dir, ""), false, 10);
        let entries = results
            .iter()
            .filter(|r| r.item.keywords.is_empty())
            .count();
        assert_eq!(entries, 10, "상한만큼만 살펴본다");
        assert!(
            results.last().is_some_and(|r| is_marker(r, "truncated")),
            "잘렸으면 마지막에 안내 항목이 있어야 한다"
        );
    }

    #[test]
    fn 잘렸는데_매치가_없으면_없음_대신_잘림만_알린다() {
        // "파일이 없습니다"는 거짓이다 — 뒷부분은 보지 않았다.
        let dir = dir_with_files(20);
        let results = folder_search_with_budget(&query_for(&dir, "zzz"), false, 5);
        assert!(!results.iter().any(|r| is_marker(r, "empty")));
        assert!(results.iter().any(|r| is_marker(r, "truncated")));
    }

    #[test]
    fn 폴더와_파일을_구분한다() {
        let dir = dir_with_files(1);
        let results = folder_search_with_budget(&query_for(&dir, ""), false, 100);
        let sub = results.iter().find(|r| r.item.name == "subdir").unwrap();
        assert_eq!(sub.item.kind, ItemKind::Directory);
        let file = results
            .iter()
            .find(|r| r.item.name == "file000.txt")
            .unwrap();
        assert_eq!(file.item.kind, ItemKind::File);
        // 같은 점수면 폴더 우선
        assert_eq!(results[0].item.name, "subdir");
    }

    #[cfg(unix)]
    #[test]
    fn 폴더_심볼릭링크는_폴더로_본다() {
        // file_type()만 보면 링크는 링크로 나온다 — 기존 is_dir() 의미를 지킨다.
        let dir = dir_with_files(0);
        std::os::unix::fs::symlink(dir.path().join("subdir"), dir.path().join("link")).unwrap();
        let results = folder_search_with_budget(&query_for(&dir, "link"), false, 100);
        assert_eq!(results[0].item.name, "link");
        assert_eq!(results[0].item.kind, ItemKind::Directory);
    }

    #[test]
    fn 검색어로_거른다() {
        let dir = dir_with_files(12);
        let results = folder_search_with_budget(&query_for(&dir, "file01"), false, 100);
        assert_eq!(results.len(), 2, "file010, file011");
    }

    #[test]
    fn 없는_경로는_오류_항목() {
        let results = folder_search_results(":f /no/such/dir/kmd-test", false);
        assert_eq!(results.len(), 1);
        assert!(is_marker(&results[0], "error"));
    }
}
