//! 파일 이름 검색 엔진 — 구조 배열 색인 + 불변 스냅숏(mmap) (docs/22)
//!
//! 런처가 같은 프로세스에서 직접 검색한다. 데몬(또는 데몬이 없을 때 런처)이 색인을
//! 스냅숏 파일로 쓰고, 런처는 그 파일을 매핑만 한다 — 검색 요청이 프로세스 경계를
//! 넘지 않으므로 접속 창구(포트·파이프)가 없다.

pub mod query;
pub mod reconcile;
pub mod scan;
pub mod snapshot;
pub mod table;

use std::path::Path;

use crate::index::{IndexItem, ItemKind, Source};
use crate::search::SearchResult;

pub use query::Boost;
use table::{Kind, Table, TableBuilder};

/// 한 프로세스 안에서 새 세대를 만드는 일(주기 갱신·감시 반영·전체 스캔)을 한 번에 하나씩 —
/// 둘이 같은 세대를 읽고 따로 고쳐 쓰면 나중 것이 먼저 것의 변경을 덮는다.
/// 다른 프로세스(런처의 열 때 대조)와는 세대 파일을 `create_new` 로 써서 겹쳐 쓰지 않는다.
static PUBLISH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn publish_guard() -> std::sync::MutexGuard<'static, ()> {
    PUBLISH_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// 열 때 대조 결과를 새 세대로 남기는 최소 간격
pub const PUBLISH_MIN_AGE: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// 런처가 검색에 쓰는 색인 — 매핑한 스냅숏, 또는 열 때 대조로 고친 메모리 표
pub enum FileIndex {
    Mapped(snapshot::Snapshot),
    Memory(TableBuilder),
}

impl std::fmt::Debug for FileIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mapped(s) => write!(
                f,
                "FileIndex::Mapped(gen {}, {} 행)",
                s.generation(),
                s.table().len()
            ),
            Self::Memory(b) => write!(f, "FileIndex::Memory({} 행)", b.len()),
        }
    }
}

impl FileIndex {
    /// 기본 위치의 현재 세대를 연다 — 없거나 깨졌으면 None (호출자는 옛 엔진만 쓴다)
    pub fn open_default() -> Option<Self> {
        match snapshot::Snapshot::open_current(&snapshot::default_dir()) {
            Ok(s) => Some(Self::Mapped(s)),
            Err(e) => {
                tracing::info!("파일 색인 스냅숏을 쓰지 않음: {e}");
                None
            }
        }
    }

    pub fn table(&self) -> Table<'_> {
        match self {
            Self::Mapped(s) => s.table(),
            Self::Memory(b) => b.table(),
        }
    }

    /// 열 때 대조 — 바뀐 폴더가 있으면 고친 색인을 돌려준다.
    ///
    /// 새 세대로 남기는 것(다음 런처가 이어 씀)은 현재 세대가 [`PUBLISH_MIN_AGE`] 보다 오래됐을 때만.
    /// 40만 개면 세대 하나가 약 28MB 라, 파일 하나 바뀔 때마다 쓰면 런처를 열 때마다 디스크를
    /// 쓴다. 그 사이의 런처는 같은 대조를 다시 하는데, 대조는 수 ms 다 (docs/22 §3.8).
    pub fn reconciled(&self, cfg: &scan::ScanConfig) -> Option<Self> {
        let started = std::time::Instant::now();
        let (b, stats) = reconcile::reconcile(&self.table(), cfg)?;
        tracing::info!(
            "열 때 대조: 폴더 {}개 확인, {}개 바뀜, +{} −{} ({}ms)",
            stats.checked_dirs,
            stats.changed_dirs,
            stats.added,
            stats.removed,
            started.elapsed().as_millis()
        );
        let dir = snapshot::default_dir();
        let stale = snapshot::current_age(&dir).is_none_or(|age| age >= PUBLISH_MIN_AGE);
        if stale {
            if let Err(e) = snapshot::publish(&dir, &b) {
                tracing::warn!("대조 결과를 스냅숏으로 남기지 못함: {e}");
            }
        }
        Some(Self::Memory(b))
    }

    /// 질의 → 결과 항목. 점수는 [`crate::rank`] 공식에서 실행 이력만 뺀 값 — [`crate::rank::merge`] 로 합친다.
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        boost: &Boost,
        weights: &crate::config::KindWeights,
        use_emoji: bool,
    ) -> Vec<SearchResult> {
        let t = self.table();
        let now = scan::to_ns(std::time::SystemTime::now());
        query::search(&t, query, limit, now, boost)
            .into_iter()
            .map(|hit| {
                let item = hit_item(&t, hit.row as usize, use_emoji);
                // 질의 점수(품질·최근·깊이)에 종류 가중을 더해 rank::merge 와 같은 척도로
                let score = hit.score + crate::rank::kind_points(item.kind, weights);
                SearchResult { item, score }
            })
            .collect()
    }
}

/// 행 하나를 결과 항목으로 — 옛 파일 색인 항목과 같은 모양 (`index::files`)
pub fn hit_item(t: &Table<'_>, row: usize, use_emoji: bool) -> IndexItem {
    let path = t.path(row);
    let path_str = path.to_string_lossy().into_owned();
    let is_dir = t.kind(row) == Kind::Dir;
    let mut item = IndexItem {
        name: String::from_utf8_lossy(t.raw_name(row)).into_owned(),
        path: path_str.clone(),
        kind: if is_dir {
            ItemKind::Directory
        } else {
            ItemKind::File
        },
        source: Source::FileProvider,
        icon: if is_dir {
            crate::index::files::dir_icon(use_emoji)
        } else {
            crate::index::files::icon_for_path(&path, use_emoji)
        },
        keywords: path_str,
        icon_path: None,
    };
    if is_dir {
        if let Some(kind) = crate::index::package::detect(Path::new(&item.path)) {
            crate::index::files::apply_package_kind(&mut item, kind, use_emoji);
        }
    }
    item
}

/// 데몬 주기 갱신 — 폴더 시각으로 바뀐 폴더만 다시 읽어 새 세대를 공개한다(공개 간격 제한 없음).
/// 폴더 시각을 믿을 수 없는 볼륨은 폴더를 전부 다시 나열하고, 달라진 것이 있을 때만 공개한다.
/// 스냅숏이 없거나 검색 폴더 목록이 바뀌었으면(`:scope`) 전체 스캔. 런처를 오래 열지 않아도
/// 색인이 뒤처지지 않게 하고, 런처의 열 때 대조가 할 일을 줄인다.
pub fn refresh(
    config: &crate::config::LauncherConfig,
) -> Result<Option<reconcile::ReconcileStats>, snapshot::SnapshotError> {
    let _guard = publish_guard();
    refresh_in(
        &snapshot::default_dir(),
        &scan::ScanConfig::from_launcher(config),
    )
}

fn refresh_in(
    dir: &Path,
    cfg: &scan::ScanConfig,
) -> Result<Option<reconcile::ReconcileStats>, snapshot::SnapshotError> {
    let Ok(snap) = snapshot::Snapshot::open_current(dir) else {
        rebuild_in(dir, cfg)?;
        return Ok(None);
    };
    let roots: Vec<_> = snap.table().roots.iter().map(|r| r.path_buf()).collect();
    if roots != cfg.roots {
        drop(snap);
        rebuild_in(dir, cfg)?;
        return Ok(None);
    }
    // 데몬은 런처와 달리 창 표시를 막지 않으니, 폴더 시각을 믿을 수 없는 볼륨(exFAT·네트워크)도
    // 여기서 다시 나열한다 — 아니면 그 볼륨은 6시간 전체 재빌드 때만 새 파일이 보였다
    let Some((b, stats)) = reconcile::relist(&snap.table(), cfg) else {
        return Ok(None);
    };
    drop(snap);
    snapshot::publish(dir, &b)?;
    Ok(Some(stats))
}

/// 전체 스캔 → 새 세대 공개. 색인 재빌드(데몬 주기·`kmd index --rebuild`·`:scope`)에서 부른다.
/// 데몬 감시 반영 (F3) — 이벤트가 난 폴더만 다시 읽는다. 이름이 생기거나 지워졌을 때만 새 세대를
/// 쓴다: 내려받는 파일처럼 수정 시각만 계속 바뀌는 경우 몇 초마다 수십 MB 를 다시 쓰지 않게
/// (시각은 주기 갱신이 반영한다). 스냅숏이 없거나 검색 폴더 목록이 바뀌었으면 [`refresh`] 와 같이 전체 스캔.
pub fn apply_dirty(
    config: &crate::config::LauncherConfig,
    dirs: &[std::path::PathBuf],
) -> Result<Option<reconcile::ReconcileStats>, snapshot::SnapshotError> {
    let _guard = publish_guard();
    apply_dirty_in(
        &snapshot::default_dir(),
        &scan::ScanConfig::from_launcher(config),
        dirs,
    )
}

fn apply_dirty_in(
    dir: &Path,
    cfg: &scan::ScanConfig,
    dirs: &[std::path::PathBuf],
) -> Result<Option<reconcile::ReconcileStats>, snapshot::SnapshotError> {
    let Ok(snap) = snapshot::Snapshot::open_current(dir) else {
        rebuild_in(dir, cfg)?;
        return Ok(None);
    };
    let roots: Vec<_> = snap.table().roots.iter().map(|r| r.path_buf()).collect();
    if roots != cfg.roots {
        drop(snap);
        rebuild_in(dir, cfg)?;
        return Ok(None);
    }
    let Some((b, stats)) = reconcile::relist_dirs(&snap.table(), cfg, dirs) else {
        return Ok(None);
    };
    if stats.added == 0 && stats.removed == 0 {
        return Ok(None);
    }
    drop(snap);
    snapshot::publish(dir, &b)?;
    Ok(Some(stats))
}

/// 색인이 쓰는 데이터 폴더 — 감시가 자기 쓰기(스냅숏·로그)에 다시 반응하지 않게 거른다
pub fn data_dirs() -> Vec<std::path::PathBuf> {
    let mut v = vec![crate::Config::standard_data_dir()];
    if let Some(parent) = snapshot::default_dir().parent() {
        v.push(parent.to_path_buf());
    }
    v
}

pub fn rebuild(
    config: &crate::config::LauncherConfig,
) -> Result<scan::ScanStats, snapshot::SnapshotError> {
    let _guard = publish_guard();
    rebuild_in(
        &snapshot::default_dir(),
        &scan::ScanConfig::from_launcher(config),
    )
}

fn rebuild_in(
    dir: &Path,
    cfg: &scan::ScanConfig,
) -> Result<scan::ScanStats, snapshot::SnapshotError> {
    let started = std::time::Instant::now();
    let (b, stats) = scan::build_table(cfg);
    let generation = snapshot::publish(dir, &b)?;
    tracing::info!(
        "파일 색인 세대 {generation}: 폴더 {} 파일 {} (빠짐 {}) — {}ms",
        stats.dirs,
        stats.files,
        stats.skipped_files,
        started.elapsed().as_millis()
    );
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 감시_반영은_이름이_생기거나_지워질_때만_새_세대() {
        let data = tempfile::tempdir().unwrap();
        let tree = tempfile::tempdir().unwrap();
        let r = tree.path();
        std::fs::create_dir_all(r.join("a")).unwrap();
        std::fs::write(r.join("a/x.txt"), "").unwrap();
        let cfg = scan::ScanConfig {
            roots: vec![r.to_path_buf()],
            depth: 6,
            ignore: Vec::new(),
            ignore_ext: Default::default(),
            max_files: 1_000_000,
        };
        let dirty = [r.join("a")];
        assert!(apply_dirty_in(data.path(), &cfg, &dirty).unwrap().is_none());
        assert_eq!(
            snapshot::current_generation(data.path()),
            Some(1),
            "없으면 만든다"
        );

        // 내용만 바뀜(수정 시각) — 새 세대를 쓰지 않는다
        let x = r.join("a/x.txt");
        let m = std::fs::metadata(&x).unwrap().modified().unwrap();
        filetime::set_file_mtime(
            &x,
            filetime::FileTime::from_system_time(m + std::time::Duration::from_secs(60)),
        )
        .unwrap();
        assert!(apply_dirty_in(data.path(), &cfg, &dirty).unwrap().is_none());
        assert_eq!(snapshot::current_generation(data.path()), Some(1));

        std::fs::write(r.join("a/y.txt"), "").unwrap();
        let stats = apply_dirty_in(data.path(), &cfg, &dirty)
            .unwrap()
            .expect("새 이름");
        assert_eq!((stats.added, stats.removed), (1, 0));
        assert_eq!(snapshot::current_generation(data.path()), Some(2));
    }

    #[test]
    fn 주기_갱신은_없으면_만들고_바뀌면_새_세대_검색_폴더가_바뀌면_다시_스캔() {
        let data = tempfile::tempdir().unwrap();
        let tree = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tree.path().join("a")).unwrap();
        let mut cfg = scan::ScanConfig {
            roots: vec![tree.path().to_path_buf()],
            depth: 6,
            ignore: Vec::new(),
            ignore_ext: Default::default(),
            max_files: 1_000_000,
        };
        assert!(refresh_in(data.path(), &cfg).unwrap().is_none());
        assert_eq!(snapshot::current_generation(data.path()), Some(1));
        assert!(
            refresh_in(data.path(), &cfg).unwrap().is_none(),
            "바뀐 것 없음"
        );

        std::fs::write(tree.path().join("a/new.txt"), "").unwrap();
        let a = tree.path().join("a");
        let m = std::fs::metadata(&a).unwrap().modified().unwrap();
        filetime::set_file_mtime(
            &a,
            filetime::FileTime::from_system_time(m + std::time::Duration::from_secs(5)),
        )
        .unwrap();
        let stats = refresh_in(data.path(), &cfg).unwrap().expect("바뀜");
        assert_eq!(stats.added, 1);
        assert_eq!(snapshot::current_generation(data.path()), Some(2));

        let other = tempfile::tempdir().unwrap();
        cfg.roots.push(other.path().to_path_buf());
        assert!(refresh_in(data.path(), &cfg).unwrap().is_none());
        let snap = snapshot::Snapshot::open_current(data.path()).unwrap();
        assert_eq!(snap.table().roots.len(), 2, "검색 폴더가 늘면 전체 스캔");
    }
}
