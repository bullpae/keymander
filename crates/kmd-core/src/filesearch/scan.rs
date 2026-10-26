//! 검색 폴더를 걸어 [`TableBuilder`] 를 만든다 (docs/22 §3.8)
//!
//! 무시 규칙은 기존 파일 색인과 같은 함수(`index::files`)를 쓴다 — 숨김·무시 폴더·제외
//! 확장자·macOS 패키지. 다른 점은 할당량이 없다는 것(안전 상한 `max_files` 만).
//! 폴더는 상한과 무관하게 모두 담는다 — 폴더 이름이 있어야 그 아래로 찾아 들어간다.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use walkdir::WalkDir;

use super::table::{Kind, RootInfo, TableBuilder, NO_PARENT};
use crate::config::LauncherConfig;
use crate::index::files::{is_ignored_dir, is_ignored_file, is_walk_candidate};
use crate::index::package;

/// 검색 폴더가 없을 때(마운트 해제 등)의 시각 표시
pub const OFFLINE: i64 = 0;

/// 스캔 설정 — `LauncherConfig` 에서 뽑는다
#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub roots: Vec<PathBuf>,
    pub depth: usize,
    pub ignore: Vec<String>,
    pub ignore_ext: HashSet<String>,
    pub max_files: usize,
}

impl ScanConfig {
    pub fn from_launcher(c: &LauncherConfig) -> Self {
        Self {
            roots: c.effective_search_paths(),
            depth: c.search_depth,
            ignore: c.effective_ignore_patterns(),
            ignore_ext: c.ignored_extension_set(),
            max_files: c.file_search.max_files,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanStats {
    pub dirs: usize,
    pub files: usize,
    /// `max_files` 를 넘어 빠진 파일 — 0 이 아니면 "색인 불완전"
    pub skipped_files: usize,
}

/// 전체 스캔
pub fn build_table(cfg: &ScanConfig) -> (TableBuilder, ScanStats) {
    let mut b = TableBuilder::new();
    let mut stats = ScanStats::default();
    for root in &cfg.roots {
        // 없는 검색 폴더(마운트 해제된 VeraCrypt·외장 볼륨)도 표에 남긴다 — 시각 0 = 오프라인.
        // 빼 버리면 다시 마운트해도 다음 전체 재빌드까지 검색되지 않는다. 열 때 대조가
        // 다시 나타난 것을 보고 그 아래를 걷는다.
        let online = root.is_dir();
        let root_id = b.add_root(RootInfo {
            path: root.as_os_str().as_encoded_bytes().to_vec(),
            mtime: if online { mtime_of(root) } else { OFFLINE },
            mtime_reliable: online && mtime_reliable(root),
        });
        if online {
            walk_into(&mut b, cfg, root, root_id, NO_PARENT, 0, &mut stats);
        }
    }
    if stats.skipped_files > 0 {
        tracing::warn!(
            "파일 색인 불완전 — 파일 {}개가 상한 {}을 넘어 빠짐",
            stats.skipped_files,
            cfg.max_files
        );
    }
    (b, stats)
}

/// `dir` 아래를 걸어 `parent` 의 자식으로 붙인다. `dir_depth` 는 `dir` 자신의 깊이
/// (검색 폴더 = 0). 열 때 대조가 새로 생긴 폴더를 붙일 때도 쓴다.
pub(crate) fn walk_into(
    b: &mut TableBuilder,
    cfg: &ScanConfig,
    dir: &Path,
    root_id: u16,
    parent: u32,
    dir_depth: usize,
    stats: &mut ScanStats,
) {
    if dir_depth >= cfg.depth {
        return;
    }
    let ignore: HashSet<&str> = cfg.ignore.iter().map(String::as_str).collect();
    let walker = WalkDir::new(dir)
        .max_depth(cfg.depth - dir_depth)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !(is_ignored_dir(e, &ignore)
                    || is_ignored_file(e, &cfg.ignore_ext)
                    || package::parent_is_package(e.path()))
        });

    // 깊이별 마지막 폴더 행 — 걷기는 폴더를 내용보다 먼저 내놓으므로 부모가 늘 앞에 있다
    let mut stack: Vec<u32> = vec![parent];
    for entry in walker {
        let Ok(entry) = entry else { continue };
        if !is_walk_candidate(&entry) {
            continue;
        }
        let d = entry.depth();
        let is_dir = entry.file_type().is_dir();
        // stats.files 는 표에 든 파일 수 (열 때 대조는 기존 표의 파일 수로 시작한다)
        if !is_dir && stats.files >= cfg.max_files {
            stats.skipped_files += 1;
            continue;
        }
        stack.truncate(d);
        let Some(&p) = stack.last() else { continue };
        let mtime = entry_mtime(&entry, is_dir);
        let kind = if is_dir { Kind::Dir } else { Kind::File };
        let row = b.push(
            entry.file_name(),
            p,
            root_id,
            kind,
            (dir_depth + d) as u32,
            mtime,
        );
        if is_dir {
            stats.dirs += 1;
            stack.push(row);
        } else {
            stats.files += 1;
        }
    }
}

/// 폴더 바로 아래 한 단계 — 스캔과 같은 규칙으로 거른다 (열 때 대조가 바뀐 폴더를 다시 읽을 때)
pub(crate) fn list_children(cfg: &ScanConfig, dir: &Path) -> std::io::Result<Vec<Child>> {
    // 폴더가 사라졌으면 walkdir 는 오류 항목 하나만 낸다 — 호출자가 삭제로 다루도록 먼저 확인
    std::fs::metadata(dir)?;
    let ignore: HashSet<&str> = cfg.ignore.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    for entry in WalkDir::new(dir)
        .min_depth(1)
        .max_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            !(is_ignored_dir(e, &ignore)
                || is_ignored_file(e, &cfg.ignore_ext)
                || package::parent_is_package(e.path()))
        })
        .flatten()
    {
        if !is_walk_candidate(&entry) {
            continue;
        }
        out.push(Child {
            name: entry.file_name().to_os_string(),
            is_dir: entry.file_type().is_dir(),
            mtime: entry_mtime(&entry, entry.file_type().is_dir()),
        });
    }
    Ok(out)
}

pub(crate) struct Child {
    pub name: std::ffi::OsString,
    pub is_dir: bool,
    pub mtime: i64,
}

/// 걷기 항목의 수정 시각. 폴더는 [`mtime_of`] 로 따로 읽는다 — Windows 에서 걷기가 주는 시각은
/// 부모 폴더 목록(FindNextFile)에 적힌 값이라 폴더는 늦게 갱신될 수 있고, 열 때 대조는
/// `fs::metadata` 로 확인하므로 둘이 어긋나면 매번 "바뀜"으로 보인다.
fn entry_mtime(entry: &walkdir::DirEntry, is_dir: bool) -> i64 {
    if is_dir {
        return mtime_of(entry.path());
    }
    entry
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .map_or(0, to_ns)
}

pub(crate) fn mtime_of(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map_or(0, to_ns)
}

pub(crate) fn to_ns(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos().min(i64::MAX as u128) as i64)
}

/// 폴더 수정 시각이 하위 항목 추가·삭제·이름 변경을 믿을 만하게 반영하는 볼륨인지.
///
/// NTFS·APFS·ext4 등 로컬 디스크는 그렇다. FAT32·exFAT 는 하위 변경에 폴더 시각을
/// 갱신하지 않고, SMB·NFS 는 클라이언트 캐시로 늦다 — 이런 볼륨은 열 때 대조에서 뺀다.
pub fn mtime_reliable(path: &Path) -> bool {
    !volume_is_unreliable(path)
}

#[cfg(target_os = "macos")]
fn volume_is_unreliable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return true;
    };
    // SAFETY: statfs 는 넘긴 구조체만 채운다. c 는 NUL 로 끝나는 유효한 경로다.
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return true;
    }
    let name: Vec<u8> = st
        .f_fstypename
        .iter()
        .take_while(|&&ch| ch != 0)
        .map(|&ch| ch as u8)
        .collect();
    matches!(
        name.as_slice(),
        b"msdos" | b"exfat" | b"smbfs" | b"nfs" | b"afpfs" | b"webdav" | b"cifs"
    )
}

#[cfg(target_os = "linux")]
fn volume_is_unreliable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return true;
    };
    // SAFETY: statfs 는 넘긴 구조체만 채운다. c 는 NUL 로 끝나는 유효한 경로다.
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return true;
    }
    // msdos·exfat·nfs·cifs·smb2·fuse(네트워크 마운트가 흔함)
    const UNRELIABLE: [i64; 6] = [
        0x4d44,
        0x2011_BAB0,
        0x6969,
        0xFF53_4D42,
        0xFE53_4D42,
        0x6573_5546,
    ];
    UNRELIABLE.contains(&(st.f_type as i64))
}

#[cfg(windows)]
fn volume_is_unreliable(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetDriveTypeW, GetVolumeInformationW, GetVolumePathNameW,
    };
    const DRIVE_REMOTE: u32 = 4;

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut root = [0u16; 512];
    // SAFETY: wide 는 NUL 로 끝나고, root 는 넘긴 길이만큼의 쓰기 버퍼다
    if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return true;
    }
    // 매핑된 네트워크 드라이브(Z:)와 UNC 공유
    // SAFETY: root 는 위에서 채운 NUL 종료 경로
    if unsafe { GetDriveTypeW(root.as_ptr()) } == DRIVE_REMOTE {
        return true;
    }
    let mut fs_name = [0u16; 64];
    // SAFETY: 이름 버퍼 외의 출력 포인터는 쓰지 않으므로 null, fs_name 은 넘긴 길이만큼의 버퍼
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs_name.as_mut_ptr(),
            fs_name.len() as u32,
        )
    };
    if ok == 0 {
        return true;
    }
    let end = fs_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(fs_name.len());
    let name = String::from_utf16_lossy(&fs_name[..end]);
    // FAT·FAT32·exFAT 는 하위 항목이 바뀌어도 폴더 수정 시각을 갱신하지 않는다
    name.to_ascii_uppercase().contains("FAT")
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn volume_is_unreliable(_path: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(root: &Path, max_files: usize) -> ScanConfig {
        ScanConfig {
            roots: vec![root.to_path_buf()],
            depth: 6,
            ignore: vec!["node_modules".into()],
            ignore_ext: HashSet::new(),
            max_files,
        }
    }

    fn tree() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        for year in 2020..=2026 {
            let p = tmp.path().join(format!("{year}년")).join("프로젝트");
            std::fs::create_dir_all(&p).unwrap();
            for n in 0..20 {
                std::fs::write(p.join(format!("doc{n}.txt")), "").unwrap();
            }
        }
        std::fs::create_dir_all(tmp.path().join("node_modules/x")).unwrap();
        std::fs::write(tmp.path().join(".hidden"), "").unwrap();
        tmp
    }

    #[test]
    fn 할당량_없이_전부_담고_부모가_먼저() {
        let tmp = tree();
        let (b, stats) = build_table(&cfg(tmp.path(), 1_000_000));
        assert_eq!(stats.files, 140);
        assert_eq!(stats.dirs, 14);
        assert_eq!(stats.skipped_files, 0);
        let t = b.table();
        for row in 0..t.len() {
            let p = t.parent[row];
            assert!(p == NO_PARENT || (p as usize) < row);
            assert!(t.path(row).exists(), "{:?}", t.path(row));
        }
        let names: Vec<_> = (0..t.len())
            .map(|r| String::from_utf8_lossy(t.raw_name(r)).into_owned())
            .collect();
        assert!(!names.iter().any(|n| n == "node_modules" || n == ".hidden"));
    }

    #[test]
    fn 상한을_넘으면_파일만_빠지고_폴더는_남는다() {
        let tmp = tree();
        let (b, stats) = build_table(&cfg(tmp.path(), 30));
        assert_eq!(stats.files, 30);
        assert_eq!(stats.skipped_files, 110);
        assert_eq!(stats.dirs, 14, "폴더는 상한과 무관");
        assert_eq!(b.len(), 44);
    }
}
