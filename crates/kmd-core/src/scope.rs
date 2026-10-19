//! 검색 범위 관리 — `:scope` 목록·경로 자동완성·추가·제거 (docs/18)
//!
//! 데스크톱·TUI·CLI가 같은 규칙을 쓰도록 판단은 전부 여기 둔다. UI는 결과 항목을
//! 그리고, Enter([`crate::activation::Activation::ScopeAdd`] 등)가 오면 [`add`]·
//! [`remove`]를 부른 뒤 인덱스를 다시 만든다.
//!
//! 설정 의미 (docs/18 §5): 검색 루트 = OS 기본 사용자 폴더(`include_user_folders`,
//! `hidden_user_folders`로 개별 제외) + `search_paths`. 예전 "대체" 의미로 적힌
//! 레거시 설정은 처음 바꿀 때 [`migrate_legacy`]가 같은 범위를 새 의미로 옮긴다.

use std::path::{Path, PathBuf};

use crate::config::{Config, LauncherConfig};
use crate::fsutil::path_within;
use crate::index::{IndexItem, ItemKind, Source};

/// `:scope` 별칭 — `query_prefix::COMMANDS`와 같은 목록
pub const SCOPE_ALIASES: &[&str] = &[":scope", ":dirs"];
/// 추가 항목 마커 (`kmd:scope:add:<경로>`)
pub const SCOPE_ADD_MARKER: &str = "kmd:scope:add:";
/// 범위 폴더 항목 마커 (`kmd:scope:folder:<경로>`) — Enter 열기, [`REMOVE_KEY_LABEL`] 제거
pub const SCOPE_FOLDER_MARKER: &str = "kmd:scope:folder:";
/// 하위 폴더로 들어가기 마커 (`kmd:scope:browse:<경로>`)
pub const SCOPE_BROWSE_MARKER: &str = "kmd:scope:browse:";
/// `:scope ` 화면으로 가기 (`:set` 첫 행, 결과 0건 안내, 목록의 "폴더 추가…")
pub const SCOPE_OPEN_MARKER: &str = "kmd:scope:open";
/// 안내 전용 (이미 포함됨, 찾을 수 없음)
pub const SCOPE_NOOP_MARKER: &str = "kmd:scope:noop";

/// 범위 폴더 제거 단축키 표기 — macOS 데스크톱은 ⌘D (⌘⌫는 입력창이 함께 받아 글자를 지운다)
pub const REMOVE_KEY_LABEL: &str = if cfg!(target_os = "macos") {
    "\u{2318}D"
} else {
    "Ctrl+Del"
};
/// 제거 되돌리기 단축키 표기
pub const UNDO_KEY_LABEL: &str = if cfg!(target_os = "macos") {
    "\u{2318}Z"
} else {
    "Ctrl+Z"
};

/// 하위 폴더 자동완성 행 상한 — 키 입력마다 그리는 목록이다.
const MAX_BROWSE_ROWS: usize = 40;

/// 범위 목록의 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeEntry {
    pub path: PathBuf,
    /// OS 기본 사용자 폴더(문서·다운로드 등)인가 — 화면에 "기본" 배지
    pub is_user_folder: bool,
}

/// 현재 검색 범위 — 화면 표시 순서(기본 폴더 → 추가 폴더).
///
/// 레거시 설정은 기본 폴더가 `search_paths`에 직접 적혀 있으므로, 경로를 비교해
/// 기본 폴더를 찾아 같은 배지로 묶는다.
pub fn entries(launcher: &LauncherConfig) -> Vec<ScopeEntry> {
    let defaults = crate::config::default_user_folders();
    let is_default = |p: &Path| defaults.iter().any(|d| same_path(d, p));
    let mut out: Vec<ScopeEntry> = launcher
        .user_folders()
        .into_iter()
        .map(|path| ScopeEntry {
            path,
            is_user_folder: true,
        })
        .collect();
    for p in &launcher.search_paths {
        if out.iter().any(|e| same_path(&e.path, p)) {
            continue;
        }
        out.push(ScopeEntry {
            path: p.clone(),
            is_user_folder: launcher.include_user_folders.is_none() && is_default(p),
        });
    }
    // 레거시: 기본 폴더를 앞으로 (화면 순서를 새 의미와 맞춘다)
    out.sort_by_key(|e| !e.is_user_folder);
    out
}

/// `path`를 덮는 범위 루트 — 없으면 범위 밖.
pub fn covering_root(launcher: &LauncherConfig, path: &Path) -> Option<PathBuf> {
    launcher
        .effective_search_paths()
        .into_iter()
        .find(|root| path_within(path, root))
}

/// 레거시 "대체" 설정을 같은 범위의 새 의미로 옮긴다 (docs/18 §5.3).
///
/// 적힌 목록 중 기본 폴더는 `include_user_folders`로 대체하고, 목록에 없던 기본
/// 폴더는 `hidden_user_folders`로 남겨 **실제 검색 범위가 바뀌지 않게** 한다.
pub fn migrate_legacy(launcher: &mut LauncherConfig) {
    if launcher.include_user_folders.is_some() {
        return;
    }
    let defaults = crate::config::default_user_folders();
    launcher.hidden_user_folders = defaults
        .iter()
        .filter(|d| !launcher.search_paths.iter().any(|p| same_path(p, d)))
        .cloned()
        .collect();
    launcher
        .search_paths
        .retain(|p| !defaults.iter().any(|d| same_path(d, p)));
    launcher.include_user_folders = Some(true);
}

/// 추가 결과
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddOutcome {
    /// 범위에 새로 들어갔다
    Added,
    /// 숨겼던 기본 폴더를 다시 켰다
    Restored,
    /// 이미 이 루트 아래에 있다 — 바꾼 것 없음
    AlreadyCovered(PathBuf),
}

/// 설정에 폴더를 추가한다 (저장 없음 — [`add`]가 감싼다).
pub fn add_to(launcher: &mut LauncherConfig, path: &Path) -> AddOutcome {
    // 숨긴 기본 폴더 복원이 먼저 — 상위 기본 폴더가 덮고 있어도(바탕 화면이
    // OneDrive 안에 있는 경우) 숨김 표시는 지워야 목록에 다시 보인다
    if launcher
        .hidden_user_folders
        .iter()
        .any(|h| same_path(h, path))
    {
        launcher.hidden_user_folders.retain(|h| !same_path(h, path));
        return AddOutcome::Restored;
    }
    if let Some(root) = covering_root(launcher, path) {
        return AddOutcome::AlreadyCovered(root);
    }
    migrate_legacy(launcher);
    launcher.search_paths.push(path.to_path_buf());
    AddOutcome::Added
}

/// 설정에서 폴더를 뺀다 (저장 없음). 범위에 없던 경로면 false.
pub fn remove_from(launcher: &mut LauncherConfig, path: &Path) -> bool {
    migrate_legacy(launcher);
    let before = launcher.search_paths.len();
    launcher.search_paths.retain(|p| !same_path(p, path));
    if launcher.search_paths.len() != before {
        return true;
    }
    if launcher.user_folders().iter().any(|u| same_path(u, path)) {
        launcher.hidden_user_folders.push(path.to_path_buf());
        return true;
    }
    false
}

/// 폴더를 범위에 추가하고 저장한다. 성공 시 안내 문구.
///
/// 디스크의 최신 설정 위에 적용한다(`Config::update_and_save`) — 다른 곳에서 바꾼
/// 설정을 덮지 않는다. 저장에 성공해야만 호출자의 메모리 설정을 바꾼다.
pub fn add(config: &mut Config, path: &Path) -> Result<(AddOutcome, String), String> {
    if !path.is_dir() {
        return Err(format!("폴더가 아닙니다: {}", path.display()));
    }
    let mut outcome = AddOutcome::Added;
    save_with(config, |l| outcome = add_to(l, path))?;
    let msg = match &outcome {
        AddOutcome::Added | AddOutcome::Restored => {
            format!("검색 범위에 추가됨: {}", path.display())
        }
        AddOutcome::AlreadyCovered(root) => {
            format!("이미 검색 범위에 포함됨: {}", root.display())
        }
    };
    Ok((outcome, msg))
}

/// 폴더를 범위에서 빼고 저장한다.
pub fn remove(config: &mut Config, path: &Path) -> Result<String, String> {
    let mut removed = false;
    save_with(config, |l| removed = remove_from(l, path))?;
    if removed {
        // 상위 폴더가 범위에 남아 있으면 그 아래라서 계속 검색된다 — 알려 준다
        Ok(match covering_root(&config.launcher, path) {
            Some(root) => format!(
                "검색 범위에서 제거됨: {} — 단, 상위 폴더 {} 범위라 계속 검색됩니다",
                path.display(),
                root.display()
            ),
            None => format!("검색 범위에서 제거됨: {}", path.display()),
        })
    } else {
        Err(format!("검색 범위에 없는 폴더입니다: {}", path.display()))
    }
}

fn save_with(config: &mut Config, edit: impl FnOnce(&mut LauncherConfig)) -> Result<(), String> {
    let Some(file) = config.config_path.clone() else {
        // 저장 경로를 모르면 메모리에만 반영한다 (테스트·임시 설정)
        edit(&mut config.launcher);
        return Ok(());
    };
    match Config::update_and_save(&file, |latest| edit(&mut latest.launcher)) {
        Ok(saved) => {
            *config = saved;
            Ok(())
        }
        Err(e) => Err(format!("설정 저장 실패: {e}")),
    }
}

/// 범위를 바꾼 뒤 인덱스를 지금 다시 만든다 — 결과는 공유 캐시에 쓰인다.
///
/// 데몬이 있으면 데몬에 맡긴다: 데몬은 메모리에 검색 엔진을 따로 들고 있어서
/// 캐시만 바꾸면 데몬 쪽 검색이 옛 범위에 머문다(예전엔 리빌드 후 데몬 재시작이
/// 필요했다). 데몬이 없으면 직접 빌드한다. 성공 시 `true` = 데몬이 처리.
pub fn rebuild_index_now(timeout: std::time::Duration) -> Result<bool, String> {
    use crate::ipc::{send_request_with_timeout, Request, Response};
    match send_request_with_timeout(&Request::RebuildIndex, timeout) {
        Ok(Response::Ok { .. }) => return Ok(true),
        Ok(Response::Error { message }) => return Err(message),
        Ok(other) => tracing::warn!("리빌드 응답 예상 밖: {other:?} — 직접 빌드"),
        Err(e) => tracing::info!("데몬 리빌드 불가({e}) — 직접 빌드"),
    }
    let config = Config::load(&Config::default_config_dir()).map_err(|e| e.to_string())?;
    let index = crate::Index::build(&config.launcher, config.general.emoji_icons);
    let data_dir = Config::default_data_dir();
    crate::index::store::save_both(
        &index,
        &data_dir.join(crate::INDEX_CACHE_BIN_FILENAME),
        &data_dir.join(crate::INDEX_CACHE_FILENAME),
    );
    Ok(false)
}

/// `:scope` 뒤 입력을 경로로 해석 — `~`, `%VAR%`, `d:`(드라이브만) 지원.
pub fn parse_input(raw: &str) -> PathBuf {
    let trimmed = raw.trim().trim_matches('"');
    let expanded = expand_env_vars(trimmed);
    // `d:`만 치면 드라이브 루트 (`d:`는 Windows에서 "그 드라이브의 현재 폴더"라 혼동)
    let expanded = if cfg!(windows) && expanded.len() == 2 && expanded.ends_with(':') {
        format!("{expanded}\\")
    } else {
        expanded
    };
    crate::folder_search::resolve_dir(&expanded)
}

/// `%USERPROFILE%` 같은 Windows식 환경 변수를 펼친다 (모든 OS에서 허용).
fn expand_env_vars(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => match std::env::var(&after[..end]) {
                Ok(val) => {
                    out.push_str(&val);
                    rest = &after[end + 1..];
                }
                Err(_) => {
                    out.push('%');
                    rest = after;
                }
            },
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 입력이 절대 경로처럼 보이는가 — 일반 검색에서 "범위에 추가" 행을 띄울지(E1).
/// 키 입력마다 불리므로 디스크를 보기 전에 모양으로 거른다.
fn looks_like_dir_path(q: &str) -> bool {
    let b = q.as_bytes();
    let drive =
        b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/');
    drive
        || q.starts_with("~/")
        || q.starts_with("~\\")
        || (cfg!(not(windows)) && q.starts_with('/') && q.len() > 1)
}

/// 일반 검색 입력이 범위 밖의 실제 폴더 경로면 맨 위에 둘 "범위에 추가" 항목 (E1).
pub fn path_add_suggestion(
    launcher: &LauncherConfig,
    query: &str,
    use_emoji: bool,
) -> Option<IndexItem> {
    let q = query.trim();
    if !looks_like_dir_path(q) {
        return None;
    }
    let path = parse_input(q);
    if !path.is_dir() || covering_root(launcher, &path).is_some() {
        return None;
    }
    Some(add_item(
        &path,
        "Enter: 검색 범위에 추가 — 바로 인덱싱합니다",
        use_emoji,
    ))
}

/// `:scope` 결과 항목.
///
/// - `rest`가 비면 범위 목록: "폴더 추가…" + 폴더마다 한 줄 + 제외 규칙 요약
/// - `rest`가 있으면 경로 자동완성: 그 폴더(또는 접두사가 맞는 폴더)의 추가 행 +
///   하위 폴더(Enter = 들어가기)
///
/// `count_under`는 인덱스에서 그 루트 아래 항목 수 — UI가 엔진으로 센다.
pub fn results(
    launcher: &LauncherConfig,
    rest: &str,
    count_under: &dyn Fn(&Path) -> usize,
    use_emoji: bool,
) -> Vec<IndexItem> {
    if rest.trim().is_empty() {
        list_results(launcher, count_under, use_emoji)
    } else {
        browse_results(launcher, rest, use_emoji)
    }
}

/// `:scope` 입력에서 명령 뒤의 나머지를 떼어 낸다.
pub fn strip_command(query: &str) -> &str {
    for alias in SCOPE_ALIASES {
        if let Some(rest) = query.strip_prefix(alias) {
            if rest.is_empty() || rest.starts_with(' ') {
                return rest.trim_start();
            }
        }
    }
    query
}

fn list_results(
    launcher: &LauncherConfig,
    count_under: &dyn Fn(&Path) -> usize,
    use_emoji: bool,
) -> Vec<IndexItem> {
    let mut items = vec![info_item(
        "폴더 추가…",
        "경로를 입력하세요 — 예: :scope D:\\Work  (Tab/Enter로 하위 폴더 탐색)",
        icon(use_emoji, "\u{2795}", "[+]"),
        SCOPE_OPEN_MARKER,
    )];
    for e in entries(launcher) {
        let exists = e.path.is_dir();
        let badge = if e.is_user_folder { "기본" } else { "추가" };
        let detail = if exists {
            format!(
                "{badge} · {}개 · Enter 열기 · {REMOVE_KEY_LABEL} 제거",
                group_digits(count_under(&e.path))
            )
        } else {
            format!("{badge} · 지금 없는 폴더 (외장 드라이브?) · {REMOVE_KEY_LABEL} 제거")
        };
        items.push(IndexItem {
            name: e.path.display().to_string(),
            path: detail,
            kind: ItemKind::SystemCommand,
            source: Source::Plugin,
            icon: if exists {
                icon(use_emoji, "\u{1F4C1}", "[DIR]")
            } else {
                icon(use_emoji, "\u{26A0}\u{FE0F}", "[!]")
            },
            keywords: format!("{SCOPE_FOLDER_MARKER}{}", e.path.display()),
            icon_path: None,
        });
    }
    let ignores = launcher.effective_ignore_patterns().len();
    let exts = launcher.ignored_extension_set().len();
    items.push(info_item(
        &format!("제외 규칙: 폴더 이름 {ignores}개 · 확장자 {exts}개"),
        "Enter: 설정 파일에서 편집 (ignore_patterns · ignore_extensions)",
        icon(use_emoji, "\u{2699}\u{FE0F}", "[CFG]"),
        "kmd:settings:config",
    ));
    items
}

fn browse_results(launcher: &LauncherConfig, rest: &str, use_emoji: bool) -> Vec<IndexItem> {
    let typed = parse_input(rest);
    let ends_with_sep = rest.ends_with(['\\', '/']);

    // 입력한 경로 자체가 폴더면 그 폴더 + 하위, 아니면 부모에서 접두사로 고른다
    let (base, prefix): (PathBuf, String) = if typed.is_dir() && (ends_with_sep || is_root(&typed))
    {
        (typed.clone(), String::new())
    } else {
        match (typed.parent(), typed.file_name()) {
            (Some(parent), Some(name)) if parent.is_dir() => {
                (parent.to_path_buf(), name.to_string_lossy().to_lowercase())
            }
            _ if typed.is_dir() => (typed.clone(), String::new()),
            _ => {
                return vec![info_item(
                    &format!("폴더를 찾을 수 없습니다: {}", typed.display()),
                    "경로를 확인하세요 — 예: D:\\Util,  ~/Documents,  %USERPROFILE%\\Work",
                    icon(use_emoji, "\u{26A0}\u{FE0F}", "[!]"),
                    SCOPE_NOOP_MARKER,
                )]
            }
        }
    };

    let mut children: Vec<PathBuf> =
        crate::folder_search::list_dir(&base, crate::folder_search::MAX_ENTRIES)
            .map(|l| {
                l.entries
                    .into_iter()
                    .filter(|e| e.is_dir)
                    .map(|e| e.path)
                    .collect()
            })
            .unwrap_or_default();
    if !prefix.is_empty() {
        children.retain(|c| {
            c.file_name()
                .map(|n| n.to_string_lossy().to_lowercase().starts_with(&prefix))
                .unwrap_or(false)
        });
    }
    children.sort_by_key(|c| c.file_name().map(|n| n.to_string_lossy().to_lowercase()));

    // 추가 후보: 접두사가 없으면 기준 폴더, 있으면 정확히 같은 이름 → 첫 후보
    let candidate = if prefix.is_empty() {
        Some(base.clone())
    } else {
        children
            .iter()
            .find(|c| {
                c.file_name()
                    .is_some_and(|n| n.to_string_lossy().to_lowercase() == prefix)
            })
            .or(children.first())
            .cloned()
    };

    let mut items = Vec::new();
    if let Some(cand) = &candidate {
        items.push(match covering_root(launcher, cand) {
            Some(root) => info_item(
                &format!("이미 검색 범위에 포함됨: {}", cand.display()),
                &format!("{} 범위 안입니다", root.display()),
                icon(use_emoji, "\u{2705}", "[OK]"),
                SCOPE_NOOP_MARKER,
            ),
            None => add_item(cand, &add_hint(cand), use_emoji),
        });
    }
    for child in children.iter().take(MAX_BROWSE_ROWS) {
        items.push(IndexItem {
            name: child.display().to_string(),
            path: "Enter/Tab: 하위 폴더 보기".to_string(),
            kind: ItemKind::SystemCommand,
            source: Source::Plugin,
            icon: icon(use_emoji, "\u{1F4C2}", "[>]"),
            keywords: format!("{SCOPE_BROWSE_MARKER}{}", child.display()),
            icon_path: None,
        });
    }
    if items.is_empty() {
        items.push(info_item(
            &format!("'{prefix}'로 시작하는 폴더가 없습니다"),
            &format!("{} 안을 찾았습니다", base.display()),
            icon(use_emoji, "\u{26A0}\u{FE0F}", "[!]"),
            SCOPE_NOOP_MARKER,
        ));
    }
    items
}

/// 추가 후 결과 문구 — 데스크톱·TUI·CLI 공용.
///
/// 0개면 이유를 알려 준다. macOS는 인덱싱을 데몬(launchd)이 하는데, 외장 볼륨·
/// iCloud 같은 보호 위치는 데몬에 파일 접근 권한이 없으면 **조용히 0개**로 끝난다
/// (백그라운드라 권한 요청 창도 안 뜰 수 있다).
pub fn added_message(path: &Path, count: usize) -> String {
    if count > 0 {
        return format!("{} 추가됨 — {count}개 검색 가능", path.display());
    }
    if cfg!(target_os = "macos") {
        format!(
            "{} 추가됨 — 0개: 폴더 접근 권한이 없을 수 있습니다 (시스템 설정 › 개인정보 보호 및 \
             보안 › 전체 디스크 접근 권한에서 kmd-daemon 허용)",
            path.display()
        )
    } else {
        format!(
            "{} 추가됨 — 0개: 폴더가 비었거나 제외 규칙에 모두 걸렸습니다",
            path.display()
        )
    }
}

/// 하위 폴더로 들어갈 때 검색창에 채울 입력 (`:scope <경로>\`).
pub fn browse_seed(path: &str) -> String {
    let sep = if cfg!(windows) { '\\' } else { '/' };
    if path.ends_with(['\\', '/']) {
        format!(":scope {path}")
    } else {
        format!(":scope {path}{sep}")
    }
}

/// 추가 행 설명 — 실행 파일이 많으면 프로그램 폴더라고 알려 준다.
fn add_hint(path: &Path) -> String {
    let exes = count_executables(path);
    if exes > 0 {
        format!("Enter: 검색 범위에 추가 — 실행 파일 {exes}개 보임")
    } else {
        "Enter: 검색 범위에 추가".to_string()
    }
}

/// 추가 전에 가볍게 살피는 실행 파일 수 (2단계, 항목 상한) — 키 입력마다 불린다.
///
/// macOS 앱은 파일이 아니라 `.app` **폴더**(번들)다 — 번들 하나를 실행 파일 하나로
/// 세고 그 안으로는 내려가지 않는다(번들 내부 파일 수천 개가 예산을 다 먹는다).
fn count_executables(path: &Path) -> usize {
    const BUDGET: usize = 2_000;
    let mut seen = 0usize;
    let mut exes = 0usize;
    let mut walker = walkdir::WalkDir::new(path).max_depth(2).into_iter();
    while let Some(Ok(entry)) = walker.next() {
        seen += 1;
        if seen > BUDGET {
            break;
        }
        if entry.depth() > 0 && is_app_bundle(entry.path()) {
            exes += 1;
            walker.skip_current_dir();
            continue;
        }
        if entry.file_type().is_file() {
            let ext = entry
                .path()
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase());
            if matches!(
                ext.as_deref(),
                Some("exe" | "lnk" | "bat" | "cmd" | "command")
            ) {
                exes += 1;
            }
        }
    }
    exes
}

/// macOS 앱 번들인가 — 인덱서와 같은 판정(`Contents/Info.plist` 필수, docs/18 §9).
fn is_app_bundle(path: &Path) -> bool {
    crate::index::package::classify(path) == Some(crate::index::package::PackageKind::App)
}

fn add_item(path: &Path, hint: &str, use_emoji: bool) -> IndexItem {
    IndexItem {
        name: format!("{} 을(를) 검색 범위에 추가", path.display()),
        path: hint.to_string(),
        kind: ItemKind::SystemCommand,
        source: Source::Plugin,
        icon: icon(use_emoji, "\u{2795}", "[+]"),
        keywords: format!("{SCOPE_ADD_MARKER}{}", path.display()),
        icon_path: None,
    }
}

fn info_item(name: &str, desc: &str, icon: String, keywords: &str) -> IndexItem {
    IndexItem {
        name: name.to_string(),
        path: desc.to_string(),
        kind: ItemKind::SystemCommand,
        source: Source::Plugin,
        icon,
        keywords: keywords.to_string(),
        icon_path: None,
    }
}

/// `:set` 첫 행 (E3) — 범위 요약, Enter로 `:scope` 진입.
pub fn settings_row(launcher: &LauncherConfig, use_emoji: bool) -> IndexItem {
    let n = entries(launcher).len();
    info_item(
        &format!("검색 범위: 폴더 {n}개"),
        "Enter: :scope — 폴더 추가·제거",
        icon(use_emoji, "\u{1F4C1}", "[DIR]"),
        SCOPE_OPEN_MARKER,
    )
}

/// 검색 결과가 없을 때 안내 (E4).
pub fn empty_result_hint(use_emoji: bool) -> IndexItem {
    info_item(
        "찾는 게 없나요? 그 폴더를 검색 범위에 추가하세요 — :scope",
        "Enter: :scope 열기 · 또는 폴더 경로를 그대로 입력",
        icon(use_emoji, "\u{2795}", "[+]"),
        SCOPE_OPEN_MARKER,
    )
}

fn icon(use_emoji: bool, emoji: &str, ascii: &str) -> String {
    if use_emoji { emoji } else { ascii }.to_string()
}

fn is_root(p: &Path) -> bool {
    p.parent().is_none()
}

fn same_path(a: &Path, b: &Path) -> bool {
    path_within(a, b) && path_within(b, a)
}

fn group_digits(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launcher(paths: Vec<PathBuf>, include: Option<bool>) -> LauncherConfig {
        LauncherConfig {
            search_paths: paths,
            include_user_folders: include,
            ..Default::default()
        }
    }

    #[test]
    fn 추가는_합산_의미로_저장되고_포함된_경로는_막는다() {
        let tmp = tempfile::tempdir().unwrap();
        let util = tmp.path().join("Util");
        std::fs::create_dir_all(util.join("VeraCrypt")).unwrap();
        let mut l = launcher(vec![], Some(false));

        assert_eq!(add_to(&mut l, &util), AddOutcome::Added);
        assert_eq!(l.search_paths, vec![util.clone()]);
        // 하위 폴더는 이미 포함
        assert_eq!(
            add_to(&mut l, &util.join("VeraCrypt")),
            AddOutcome::AlreadyCovered(util.clone())
        );
        assert!(remove_from(&mut l, &util));
        assert!(l.search_paths.is_empty());
        assert!(!remove_from(&mut l, &util), "없는 경로 제거는 false");
    }

    #[test]
    fn 레거시_설정은_첫_변경에서_같은_범위로_옮겨진다() {
        let defaults = crate::config::default_user_folders();
        let Some(first) = defaults.first().cloned() else {
            return; // 기본 폴더가 하나도 없는 CI 환경
        };
        let tmp = tempfile::tempdir().unwrap();
        // 레거시: 기본 폴더 하나 + 사용자 폴더를 직접 적었다 (나머지 기본 폴더는 빠짐)
        let mut l = launcher(vec![first.clone(), tmp.path().to_path_buf()], None);
        let before = l.effective_search_paths();

        let extra = tmp.path().join("new");
        std::fs::create_dir(&extra).unwrap();
        let other = tempfile::tempdir().unwrap();
        assert_eq!(add_to(&mut l, other.path()), AddOutcome::Added);

        assert_eq!(l.include_user_folders, Some(true));
        assert!(
            !l.search_paths.iter().any(|p| same_path(p, &first)),
            "기본 폴더는 스위치로"
        );
        let mut after = l.effective_search_paths();
        after.retain(|p| !same_path(p, other.path()));
        assert_eq!(after.len(), before.len(), "추가한 것 외에는 범위가 그대로");
        for p in &before {
            assert!(after.iter().any(|a| same_path(a, p)));
        }
    }

    #[test]
    fn 기본_폴더_제거는_숨김으로_다시_추가하면_복원() {
        let defaults = crate::config::default_user_folders();
        let Some(first) = defaults.first().cloned() else {
            return;
        };
        let mut l = launcher(vec![], Some(true));
        assert!(remove_from(&mut l, &first));
        assert!(l.hidden_user_folders.iter().any(|h| same_path(h, &first)));
        assert!(!l
            .effective_search_paths()
            .iter()
            .any(|p| same_path(p, &first)));
        assert_eq!(add_to(&mut l, &first), AddOutcome::Restored);
        assert!(l.hidden_user_folders.is_empty());
    }

    #[test]
    fn 경로_입력_자동완성은_접두사로_고르고_추가_행을_먼저() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("Util")).unwrap();
        std::fs::create_dir(tmp.path().join("Utility_old")).unwrap();
        std::fs::create_dir(tmp.path().join("Work")).unwrap();
        let l = launcher(vec![], Some(false));

        let typed = format!("{}", tmp.path().join("ut").display());
        let items = browse_results(&l, &typed, false);
        assert!(items[0].keywords.starts_with(SCOPE_ADD_MARKER));
        assert!(
            items[0].keywords.ends_with("Util"),
            "첫 후보 = 정렬상 첫 폴더"
        );
        let browse: Vec<_> = items
            .iter()
            .filter(|i| i.keywords.starts_with(SCOPE_BROWSE_MARKER))
            .collect();
        assert_eq!(browse.len(), 2, "Work는 접두사가 달라 빠진다");
    }

    #[test]
    fn 범위_안_경로는_추가_대신_포함_안내() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("A")).unwrap();
        let l = launcher(vec![tmp.path().to_path_buf()], Some(false));
        let typed = format!(
            "{}{}",
            tmp.path().join("A").display(),
            std::path::MAIN_SEPARATOR
        );
        let items = browse_results(&l, &typed, false);
        assert_eq!(items[0].keywords, SCOPE_NOOP_MARKER);
        assert!(
            path_add_suggestion(&l, &tmp.path().join("A").display().to_string(), false).is_none()
        );
    }

    #[test]
    fn 일반_검색의_경로_입력은_범위_밖일_때만_추가_행() {
        let tmp = tempfile::tempdir().unwrap();
        let l = launcher(vec![], Some(false));
        let q = tmp.path().display().to_string();
        let item = path_add_suggestion(&l, &q, false).expect("범위 밖 실제 폴더");
        assert!(item.keywords.starts_with(SCOPE_ADD_MARKER));
        assert!(
            path_add_suggestion(&l, "vera", false).is_none(),
            "경로 모양이 아니면 디스크를 안 본다"
        );
    }

    #[test]
    fn 앱_번들은_하나로_세고_안으로_내려가지_않는다() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Foo.app").join("Contents").join("MacOS");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("Foo"), "").unwrap();
        std::fs::write(
            tmp.path()
                .join("Foo.app")
                .join("Contents")
                .join("Info.plist"),
            "<plist/>",
        )
        .unwrap();
        // Info.plist 없는 `web.app`은 앱이 아니다 (개발 폴더 이름)
        std::fs::create_dir(tmp.path().join("web.app")).unwrap();
        std::fs::write(tmp.path().join("tool.exe"), "").unwrap();
        std::fs::write(tmp.path().join("run.command"), "").unwrap();
        std::fs::write(tmp.path().join("readme.txt"), "").unwrap();
        assert_eq!(count_executables(tmp.path()), 3);
    }

    #[test]
    fn 입력_해석_환경변수와_명령_떼기() {
        std::env::set_var("KMD_SCOPE_TEST_DIR", "zz");
        assert!(expand_env_vars("%KMD_SCOPE_TEST_DIR%/x").starts_with("zz"));
        assert_eq!(expand_env_vars("100%"), "100%");
        assert_eq!(strip_command(":scope D:\\Util"), "D:\\Util");
        assert_eq!(strip_command(":dirs"), "");
        assert_eq!(group_digits(1234567), "1,234,567");
    }
}
