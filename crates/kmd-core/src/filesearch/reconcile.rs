//! 열 때 대조 — 런처를 여는 순간의 파일 상태로 검색되게 (docs/22 §3.8)
//!
//! 파일이 생기거나 지워지거나 이름이 바뀌면 그 파일이 든 **폴더의 수정 시각**이 바뀐다
//! (NTFS·APFS·ext4 공통). 그래서 파일 40만 개가 아니라 폴더 행만 stat 해 바뀐 폴더를
//! 찾고, 그 폴더만 다시 읽어 새 표를 만든다. 폴더 2만 개 확인이 4스레드 11ms 다.
//!
//! 폴더 시각을 믿을 수 없는 볼륨(FAT·네트워크, [`RootInfo::mtime_reliable`])은 열 때
//! 대조에서 건너뛴다. 데몬의 주기 갱신([`relist`])만 그 볼륨의 폴더를 전부 다시 읽는다 —
//! exFAT 는 파일을 넣고·바꾸고·지워도 폴더 시각이 그대로다(2026-10-27 Windows 실측).
//! 파일 **내용**만 바뀐 경우는 폴더 시각이 그대로라 감지되지 않는다 — 이름 검색에는
//! 영향이 없고 "최근 수정" 가중만 늦다.

use std::collections::{HashMap, HashSet};

use super::scan::{
    list_children, mtime_of, mtime_reliable, walk_into, ScanConfig, ScanStats, OFFLINE,
};
use super::table::{Kind, Table, TableBuilder, NO_PARENT};

/// 자식을 가진 쪽 — 검색 폴더 자신 또는 폴더 행
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Container {
    Root(u16),
    Dir(u32),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReconcileStats {
    pub checked_dirs: usize,
    pub changed_dirs: usize,
    pub added: usize,
    pub removed: usize,
}

/// 바뀐 것이 없으면 `None`. 있으면 바뀐 폴더를 반영한 새 표.
pub fn reconcile(t: &Table<'_>, cfg: &ScanConfig) -> Option<(TableBuilder, ReconcileStats)> {
    reconcile_with(t, cfg, false)
}

/// [`reconcile`] + 폴더 시각을 믿을 수 없는 볼륨의 폴더를 시각과 무관하게 전부 다시 읽는다.
/// 폴더 수만큼 디렉터리를 나열하므로 런처를 열 때가 아니라 데몬의 주기 갱신에서만 부른다.
pub fn relist(t: &Table<'_>, cfg: &ScanConfig) -> Option<(TableBuilder, ReconcileStats)> {
    reconcile_with(t, cfg, true)
}

fn reconcile_with(
    t: &Table<'_>,
    cfg: &ScanConfig,
    relist_unreliable: bool,
) -> Option<(TableBuilder, ReconcileStats)> {
    let mut stats = ReconcileStats::default();
    let (changed, new_mtime) = changed_containers(t, cfg, &mut stats, relist_unreliable);
    if changed.is_empty() {
        return None;
    }

    // 바뀐 폴더의 지금 자식 목록
    let mut listings: HashMap<Container, HashMap<Vec<u8>, super::scan::Child>> = HashMap::new();
    for &c in &changed {
        let path = match c {
            Container::Root(r) => t.roots[r as usize].path_buf(),
            Container::Dir(row) => t.path(row as usize),
        };
        // 사라진 폴더는 부모 쪽 목록에서 빠지며 지워진다 — 여기서는 빈 목록
        let children = list_children(cfg, &path).unwrap_or_default();
        listings.insert(
            c,
            children
                .into_iter()
                .map(|ch| (ch.name.as_encoded_bytes().to_vec(), ch))
                .collect(),
        );
    }

    // 기존 자식과 비교 — 없어진 것은 지우고, 남은 것은 시각 갱신, 새 이름은 나중에 붙인다
    let n = t.len();
    let mut removed = vec![false; n];
    let mut mtime_update: HashMap<usize, i64> = HashMap::new();
    let mut seen: HashMap<Container, HashSet<Vec<u8>>> = HashMap::new();
    // 자식이 실제로 빠지거나 늘어난 폴더 — "폴더 n개 바뀜" 진단용
    let mut touched: HashSet<Container> = HashSet::new();
    for (row, gone) in removed.iter_mut().enumerate() {
        let p = t.parent[row];
        let c = if p == NO_PARENT {
            Container::Root(t.root_id[row])
        } else {
            Container::Dir(p)
        };
        let Some(list) = listings.get(&c) else {
            continue;
        };
        let name = t.raw_name(row);
        match list.get(name) {
            Some(ch) if ch.is_dir == (t.kind(row) == Kind::Dir) => {
                seen.entry(c).or_default().insert(name.to_vec());
                if ch.mtime != t.mtime[row] && t.kind(row) == Kind::File {
                    mtime_update.insert(row, ch.mtime);
                }
            }
            // 사라졌거나 파일↔폴더가 바뀜 → 지우고(바뀐 것은 새로 붙는다)
            _ => {
                *gone = true;
                touched.insert(c);
            }
        }
    }
    for (&c, &m) in &new_mtime {
        if let Container::Dir(row) = c {
            mtime_update.insert(row as usize, m);
        }
    }

    // 새 표 — 지운 행과 그 아래는 빼고, 부모 번호를 다시 매긴다 (부모가 먼저라 한 번에)
    let mut b = TableBuilder::new();
    b.roots = t.roots.to_vec();
    for (&c, &m) in &new_mtime {
        if let Container::Root(r) = c {
            let root = &mut b.roots[r as usize];
            if root.mtime == m {
                continue; // 시각 그대로 다시 나열만 한 검색 폴더 — 볼륨 판별도 그대로
            }
            root.mtime = m;
            // 다시 마운트되면 볼륨이 바뀌었을 수 있다(같은 드라이브 문자에 다른 볼륨)
            root.mtime_reliable = m != OFFLINE && mtime_reliable(&root.path_buf());
        }
    }
    let mut new_index = vec![NO_PARENT; n];
    let mut scan_stats = ScanStats::default();
    for row in 0..n {
        let p = t.parent[row];
        let parent_gone = p != NO_PARENT && new_index[p as usize] == NO_PARENT;
        if removed[row] || parent_gone {
            stats.removed += 1;
            continue;
        }
        let new_parent = if p == NO_PARENT {
            NO_PARENT
        } else {
            new_index[p as usize]
        };
        let mtime = mtime_update.get(&row).copied().unwrap_or(t.mtime[row]);
        new_index[row] = b.push_copied(t, row, new_parent, mtime);
        match t.kind(row) {
            Kind::Dir => scan_stats.dirs += 1,
            Kind::File => scan_stats.files += 1,
        }
    }

    // 새로 생긴 이름 — 폴더면 그 아래까지 걸어 붙인다
    let before = b.len();
    for &c in &changed {
        let Some(list) = listings.get(&c) else {
            continue;
        };
        let known = seen.get(&c);
        let (root_id, parent, depth, dir_path) = match c {
            Container::Root(r) => (r, NO_PARENT, 0usize, t.roots[r as usize].path_buf()),
            Container::Dir(row) => {
                let np = new_index[row as usize];
                if np == NO_PARENT {
                    continue; // 이 폴더 자신이 지워졌다
                }
                (
                    t.root_id[row as usize],
                    np,
                    t.depth(row as usize) as usize,
                    t.path(row as usize),
                )
            }
        };
        if depth + 1 > cfg.depth {
            continue;
        }
        let mut fresh: Vec<_> = list
            .iter()
            .filter(|(name, _)| known.is_none_or(|k| !k.contains(*name)))
            .collect();
        fresh.sort_by(|a, b| a.0.cmp(b.0)); // 결과가 실행마다 같도록
        if !fresh.is_empty() {
            touched.insert(c);
        }
        for (_, ch) in fresh {
            if !ch.is_dir && scan_stats.files >= cfg.max_files {
                continue;
            }
            let kind = if ch.is_dir { Kind::Dir } else { Kind::File };
            let row = b.push(
                &ch.name,
                parent,
                root_id,
                kind,
                (depth + 1) as u32,
                ch.mtime,
            );
            if ch.is_dir {
                scan_stats.dirs += 1;
                walk_into(
                    &mut b,
                    cfg,
                    &dir_path.join(&ch.name),
                    root_id,
                    row,
                    depth + 1,
                    &mut scan_stats,
                );
            } else {
                scan_stats.files += 1;
            }
        }
    }
    stats.added = b.len() - before;

    // 다시 읽었지만 실제로 달라진 것이 없으면(믿을 수 없는 볼륨을 다시 나열한 경우) 새 세대를
    // 쓰지 않는다 — 5분마다 수십 MB 를 다시 쓰지 않게
    let mtime_changed = mtime_update.iter().any(|(&row, &m)| m != t.mtime[row]);
    let roots_changed = b
        .roots
        .iter()
        .zip(t.roots.iter())
        .any(|(a, b)| a.mtime != b.mtime || a.mtime_reliable != b.mtime_reliable);
    if stats.added == 0 && stats.removed == 0 && !mtime_changed && !roots_changed {
        return None;
    }
    stats.changed_dirs = changed
        .iter()
        .filter(|&&c| {
            touched.contains(&c)
                || match c {
                    Container::Root(r) => new_mtime.get(&c) != Some(&t.roots[r as usize].mtime),
                    Container::Dir(row) => new_mtime.get(&c) != Some(&t.mtime[row as usize]),
                }
        })
        .count();
    Some((b, stats))
}

/// 시각이 바뀐 검색 폴더·폴더 행과 그 새 시각. `relist_unreliable` 이면 폴더 시각을 믿을 수
/// 없는 볼륨의 검색 폴더·폴더 행은 시각과 무관하게 모두 "바뀜"으로 낸다(다시 나열 대상).
fn changed_containers(
    t: &Table<'_>,
    cfg: &ScanConfig,
    stats: &mut ReconcileStats,
    relist_unreliable: bool,
) -> (Vec<Container>, HashMap<Container, i64>) {
    let mut changed = Vec::new();
    let mut new_mtime = HashMap::new();
    let mut relist_roots = vec![false; t.roots.len()];
    for (r, root) in t.roots.iter().enumerate() {
        let path = root.path_buf();
        let online = path.is_dir();
        // 오프라인이던 검색 폴더가 다시 나타남(VeraCrypt·외장 볼륨 다시 마운트) — 볼륨 종류와
        // 상관없이 통째로 다시 읽는다. 사라진 폴더도 마찬가지로 자식을 모두 지운다.
        let came_back = root.mtime == OFFLINE && online;
        let went_away = root.mtime != OFFLINE && !online;
        let relist = relist_unreliable && !root.mtime_reliable && online && !came_back;
        relist_roots[r] = relist;
        if !(came_back || went_away || root.mtime_reliable || relist) {
            continue;
        }
        let m = if online { mtime_of(&path) } else { OFFLINE };
        if m != root.mtime || relist {
            changed.push(Container::Root(r as u16));
            new_mtime.insert(Container::Root(r as u16), m);
        }
    }

    // 다시 나열할 볼륨의 폴더 행 — 시각을 보지 않고 모두
    for row in 0..t.len() {
        if t.kind(row) == Kind::Dir
            && relist_roots[t.root_id[row] as usize]
            && (t.depth(row) as usize) < cfg.depth
        {
            let c = Container::Dir(row as u32);
            changed.push(c);
            new_mtime.insert(c, mtime_of(&t.path(row)));
        }
    }

    let dirs: Vec<u32> = (0..t.len())
        .filter(|&row| {
            t.kind(row) == Kind::Dir
                && t.roots[t.root_id[row] as usize].mtime_reliable
                && (t.depth(row) as usize) < cfg.depth
        })
        .map(|row| row as u32)
        .collect();
    stats.checked_dirs = dirs.len();
    let threads = if dirs.len() < 2_000 {
        1
    } else {
        std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).clamp(1, 4))
    };
    let chunk = dirs.len().div_ceil(threads).max(1);
    let found: Vec<(u32, i64)> = std::thread::scope(|s| {
        let handles: Vec<_> = dirs
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    part.iter()
                        .filter_map(|&row| {
                            let m = std::fs::metadata(t.path(row as usize))
                                .and_then(|md| md.modified())
                                .ok()
                                .map(super::scan::to_ns)?;
                            (m != t.mtime[row as usize]).then_some((row, m))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    for (row, m) in found {
        changed.push(Container::Dir(row));
        new_mtime.insert(Container::Dir(row), m);
    }
    (changed, new_mtime)
}

#[cfg(test)]
mod tests {
    use super::super::scan::build_table;
    use super::*;
    use std::path::Path;

    fn cfg(root: &Path) -> ScanConfig {
        ScanConfig {
            roots: vec![root.to_path_buf()],
            depth: 6,
            ignore: Vec::new(),
            ignore_ext: HashSet::new(),
            max_files: 1_000_000,
        }
    }

    fn names(b: &TableBuilder) -> Vec<String> {
        let t = b.table();
        let mut v: Vec<String> = (0..t.len())
            .map(|r| t.path(r).to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    /// 폴더 시각이 확실히 바뀌게 — 파일시스템 시각 해상도(1초 이하)를 피해 직접 민다
    fn bump(dir: &Path) {
        let m = std::fs::metadata(dir).unwrap().modified().unwrap();
        let later = filetime::FileTime::from_system_time(m + std::time::Duration::from_secs(5));
        filetime::set_file_mtime(dir, later).unwrap();
    }

    #[test]
    fn 마운트_해제된_검색_폴더는_오프라인으로_남고_다시_나타나면_색인된다() {
        // VeraCrypt·외장 볼륨: 색인 재빌드 때 마운트가 풀려 있어도 다시 마운트하면 열 때 대조로 돌아온다
        let tmp = tempfile::tempdir().unwrap();
        let vol = tmp.path().join("vol");
        let c = cfg(&vol);
        let (b, stats) = build_table(&c);
        assert_eq!((b.len(), stats.files), (0, 0));
        assert_eq!(b.roots.len(), 1, "없는 검색 폴더도 표에 남는다");
        assert_eq!(b.roots[0].mtime, OFFLINE);

        std::fs::create_dir_all(vol.join("비밀")).unwrap();
        std::fs::write(vol.join("비밀/계약서.hwp"), "").unwrap();
        let (mounted, _) = reconcile(&b.table(), &c).expect("다시 나타남");
        assert_eq!(names(&mounted), names(&build_table(&c).0));
        assert_ne!(mounted.roots[0].mtime, OFFLINE);

        std::fs::remove_dir_all(&vol).unwrap();
        let (gone, _) = reconcile(&mounted.table(), &c).expect("사라짐");
        assert_eq!(gone.len(), 0, "마운트 해제되면 결과에서 빠진다");
        assert_eq!(gone.roots[0].mtime, OFFLINE);
        assert!(reconcile(&gone.table(), &c).is_none());
    }

    #[test]
    fn 폴더_시각을_못_믿는_볼륨은_다시_나열해야_보이고_안_바뀌었으면_none() {
        // exFAT(VeraCrypt O:) 는 파일을 넣어도 폴더 시각이 그대로다 (2026-10-27 실측)
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        std::fs::create_dir_all(r.join("2026/자료")).unwrap();
        std::fs::write(r.join("2026/자료/보고서.hwp"), "").unwrap();
        let c = cfg(r);
        let (mut base, _) = build_table(&c);
        base.roots[0].mtime_reliable = false;

        assert!(
            relist(&base.table(), &c).is_none(),
            "바뀐 것 없으면 새 세대 없음"
        );

        // 폴더 시각을 되돌려 exFAT 처럼 — 열 때 대조로는 못 본다
        let keep = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
        let (m_root, m_sub) = (keep(r), keep(&r.join("2026/자료")));
        std::fs::write(r.join("2026/자료/새파일.txt"), "").unwrap();
        std::fs::remove_file(r.join("2026/자료/보고서.hwp")).unwrap();
        std::fs::create_dir(r.join("2027")).unwrap();
        filetime::set_file_mtime(r, filetime::FileTime::from_system_time(m_root)).unwrap();
        let sub = r.join("2026/자료");
        filetime::set_file_mtime(&sub, filetime::FileTime::from_system_time(m_sub)).unwrap();
        assert!(
            reconcile(&base.table(), &c).is_none(),
            "런처의 열 때 대조는 건너뛴다"
        );

        let (after, stats) = relist(&base.table(), &c).expect("다시 나열하면 보인다");
        let mut fresh = build_table(&c).0;
        fresh.roots[0].mtime_reliable = false;
        assert_eq!(names(&after), names(&fresh));
        assert_eq!((stats.added, stats.removed), (2, 1), "{stats:?}");
        assert_eq!(
            stats.changed_dirs, 2,
            "검색 폴더(2027 추가)·자료(파일 교체)"
        );
        assert!(relist(&after.table(), &c).is_none());
    }

    #[test]
    fn 바뀐_것이_없으면_none() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        std::fs::write(tmp.path().join("a/b/x.txt"), "").unwrap();
        let c = cfg(tmp.path());
        let (b, _) = build_table(&c);
        assert!(reconcile(&b.table(), &c).is_none());
    }

    #[test]
    fn 추가_삭제_이름변경_폴더통째_삭제를_반영하면_새로_스캔한_것과_같다() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        for y in ["2025년", "2026년"] {
            std::fs::create_dir_all(r.join(y).join("자료")).unwrap();
            std::fs::write(r.join(y).join("자료/보고서.hwp"), "").unwrap();
        }
        std::fs::write(r.join("old.txt"), "").unwrap();
        let c = cfg(r);
        let (base, _) = build_table(&c);

        // 파일 추가, 이름 변경, 폴더 통째 삭제, 새 폴더(하위 포함)
        std::fs::write(r.join("2026년/자료/새파일.txt"), "").unwrap();
        std::fs::rename(r.join("old.txt"), r.join("new.txt")).unwrap();
        std::fs::remove_dir_all(r.join("2025년")).unwrap();
        std::fs::create_dir_all(r.join("2027년/계획")).unwrap();
        std::fs::write(r.join("2027년/계획/초안.md"), "").unwrap();
        bump(r);
        bump(&r.join("2026년/자료"));

        let (after, stats) = reconcile(&base.table(), &c).expect("바뀐 것 있음");
        let (fresh, _) = build_table(&c);
        assert_eq!(names(&after), names(&fresh));
        assert!(stats.added >= 4 && stats.removed >= 4, "{stats:?}");
        // 부모가 먼저 — 스냅숏 검증과 같은 불변식
        let t = after.table();
        assert!(
            (0..t.len()).all(|row| t.parent[row] == NO_PARENT || (t.parent[row] as usize) < row)
        );
        // 한 번 반영한 뒤에는 다시 바뀐 것이 없다
        assert!(reconcile(&t, &c).is_none());
    }
}
