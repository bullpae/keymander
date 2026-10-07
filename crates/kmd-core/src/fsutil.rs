//! 파일시스템 판정 규칙 중 **OS마다 다른 것**을 한 곳에 모은다.
//!
//! 예전에는 "숨김" 판정이 색인·본문 색인·폴더 검색·폴더 제안·TUI 드릴다운에
//! 각자 `name.starts_with('.')`로 흩어져 있었다. 그건 macOS·Linux의 규칙일 뿐이다.
//! Windows는 이름이 아니라 **숨김 속성**으로 숨긴다 — `desktop.ini`(거의 모든
//! 폴더에 있음), `Thumbs.db`, `NTUSER.DAT`, `AppData`가 검색 결과에 섞였다.

use std::path::Path;

/// Windows `FILE_ATTRIBUTE_HIDDEN`.
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;

/// 이름 규칙(모든 OS): `.`으로 시작하면 숨김.
pub fn is_hidden_name(name: &str) -> bool {
    name.len() > 1 && name.starts_with('.')
}

/// Windows 속성 규칙. 탐색기 기본값과 같은 기준 — 숨김 속성만 본다.
///
/// 시스템 속성(`0x4`)은 보지 않는다. 아이콘을 바꾼 사용자 폴더에 시스템 속성이
/// 붙는 경우가 있어, 그걸 숨김으로 보면 사용자 폴더가 통째로 빠진다. 보호된 OS
/// 파일(`desktop.ini` 등)은 숨김+시스템이라 숨김 속성만으로 걸러진다.
pub fn is_hidden_attr(attributes: u32) -> bool {
    attributes & FILE_ATTRIBUTE_HIDDEN != 0
}

/// `std::fs::read_dir` 항목이 숨김인가.
///
/// Windows에서 `DirEntry::metadata()`는 디렉터리 열거가 이미 받아 둔 정보라
/// 추가 시스템 호출이 없다. 다른 OS에선 속성을 볼 일이 없으므로 부르지 않는다.
pub fn is_hidden_entry(entry: &std::fs::DirEntry) -> bool {
    if is_hidden_name(&entry.file_name().to_string_lossy()) {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if let Ok(meta) = entry.metadata() {
            return is_hidden_attr(meta.file_attributes());
        }
    }
    false
}

/// `walkdir` 항목이 숨김인가 — 규칙은 [`is_hidden_entry`]와 같다.
pub fn is_hidden_walk_entry(entry: &walkdir::DirEntry) -> bool {
    if is_hidden_name(&entry.file_name().to_string_lossy()) {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if let Ok(meta) = entry.metadata() {
            return is_hidden_attr(meta.file_attributes());
        }
    }
    false
}

/// 이 컴퓨터의 드라이브·외장 볼륨 루트 (홈은 포함하지 않는다).
///
/// - Windows: `C:\` ~ `Z:\` 중 존재하는 것. A·B는 플로피 자리라 조회가 멈출 수 있어 뺀다
/// - macOS: `/Volumes/*` — 단 부팅 디스크 별칭(`/Volumes/Macintosh HD -> /`)은 뺀다
/// - Linux: `/mnt/*`, `/media/*`
///
/// 예전엔 색인(`index/files.rs`)과 폴더 제안(`folder_suggest.rs`)에 두 벌이 있었고,
/// 부팅 디스크 필터는 폴더 제안 쪽에만 들어가 있었다(9dbe15c) — 색인에서
/// `scan_drives`를 켜면 macOS 시스템 디스크 전체를 훑었다.
pub fn volume_roots() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();

    #[cfg(target_os = "windows")]
    for letter in 'C'..='Z' {
        let drive = std::path::PathBuf::from(format!("{letter}:\\"));
        if drive.is_dir() {
            roots.push(drive);
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let parents: &[&str] = if cfg!(target_os = "macos") {
            &["/Volumes"]
        } else {
            &["/mnt", "/media"]
        };
        for parent in parents {
            let Ok(entries) = std::fs::read_dir(parent) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() && !is_boot_volume_alias(&p) {
                    roots.push(p);
                }
            }
        }
    }

    roots
}

/// `/Volumes/Macintosh HD` 같은 **부팅 디스크 별칭**인가.
///
/// macOS는 부팅 볼륨을 `/Volumes/<이름> -> /` 심볼릭 링크로 둔다. 외장 디스크는
/// 실제 마운트 지점(디렉터리)이다. 이걸 외장 볼륨으로 보고 훑으면 시스템 디스크
/// 전체가 후보가 되어 `/private`(OS가 계속 파일을 쓰는 곳)를 "검색 범위에
/// 추가하라"고 제안했다(2026-10-07 실사례).
pub fn is_boot_volume_alias(p: &Path) -> bool {
    let is_link = std::fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    is_link || p.canonicalize().is_ok_and(|c| c == Path::new("/"))
}

/// `path`가 `root` 아래(또는 같은 곳)인가.
///
/// Windows 경로는 대소문자를 구분하지 않는데 `Path::starts_with`는 구분한다.
/// config에 `c:\users\me\work`로 적고 스캔 결과가 `C:\Users\Me\work`면 "범위 밖"이
/// 되어, 이미 검색 중인 폴더를 다시 추가하라고 제안하는 식의 오판이 생긴다.
/// 구성요소 단위로 비교하므로 `/`·`\` 혼용도 같은 경로로 본다.
pub fn path_within(path: &Path, root: &Path) -> bool {
    if cfg!(windows) {
        path_within_ignore_case(path, root)
    } else {
        path.starts_with(root)
    }
}

fn path_within_ignore_case(path: &Path, root: &Path) -> bool {
    let mut p = path.components();
    for r in root.components() {
        match p.next() {
            Some(c)
                if c.as_os_str().to_string_lossy().to_lowercase()
                    == r.as_os_str().to_string_lossy().to_lowercase() => {}
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 이름_규칙() {
        assert!(is_hidden_name(".git"));
        assert!(is_hidden_name(".DS_Store"));
        assert!(!is_hidden_name("."), "자기 자신은 숨김이 아니다");
        assert!(!is_hidden_name("notes.txt"));
        assert!(
            !is_hidden_name("desktop.ini"),
            "이름만으로는 Windows 숨김을 모른다"
        );
    }

    #[test]
    fn 속성_규칙() {
        assert!(is_hidden_attr(0x2)); // HIDDEN
        assert!(is_hidden_attr(0x2 | 0x4)); // HIDDEN | SYSTEM — desktop.ini
        assert!(
            !is_hidden_attr(0x4),
            "시스템만 붙은 사용자 폴더는 숨기지 않는다"
        );
        assert!(
            !is_hidden_attr(0x1 | 0x10),
            "읽기 전용 폴더(아이콘 바꾼 폴더)는 보인다"
        );
    }

    #[test]
    fn 대소문자_무시_포함_판정() {
        assert!(path_within_ignore_case(
            Path::new("C:/Users/Me/work/a.txt"),
            Path::new("c:/users/me/work")
        ));
        assert!(path_within_ignore_case(
            Path::new("/a/B"),
            Path::new("/a/b")
        ));
        assert!(
            !path_within_ignore_case(Path::new("/a/workspace"), Path::new("/a/work")),
            "접두 문자열이 아니라 구성요소 단위로 비교한다"
        );
        assert!(!path_within_ignore_case(Path::new("/a"), Path::new("/a/b")));
    }

    #[test]
    fn 숨김_파일_실제_판정() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".hidden"), b"").unwrap();
        std::fs::write(dir.path().join("shown.txt"), b"").unwrap();
        let mut hidden: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(is_hidden_entry)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        hidden.sort();
        assert_eq!(hidden, vec![".hidden"]);
    }

    /// Windows CI에서 실제 숨김 속성을 붙여 본다 — 속성 경로는 다른 OS에서 돌지 않는다.
    #[cfg(windows)]
    #[test]
    fn 윈도우_숨김_속성_파일은_숨김이다() {
        let dir = tempfile::tempdir().unwrap();
        let ini = dir.path().join("desktop.ini");
        std::fs::write(&ini, b"").unwrap();
        std::fs::write(dir.path().join("shown.txt"), b"").unwrap();
        let ok = std::process::Command::new("attrib")
            .arg("+h")
            .arg(&ini)
            .status()
            .unwrap()
            .success();
        assert!(ok, "attrib +h 실패");

        let hidden: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(is_hidden_entry)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(hidden, vec!["desktop.ini"]);

        let walk_hidden: Vec<String> = walkdir::WalkDir::new(dir.path())
            .min_depth(1)
            .into_iter()
            .flatten()
            .filter(is_hidden_walk_entry)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(walk_hidden, vec!["desktop.ini"]);
    }
}
