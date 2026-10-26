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

    /// 열 때 대조 — 바뀐 폴더가 있으면 고친 색인을 돌려주고, 다음 런처도 쓰도록 새 세대로 남긴다
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
        if let Err(e) = snapshot::publish(&snapshot::default_dir(), &b) {
            tracing::warn!("대조 결과를 스냅숏으로 남기지 못함: {e}");
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

/// 전체 스캔 → 새 세대 공개. 색인 재빌드(데몬 주기·`kmd index --rebuild`·`:scope`)에서 부른다.
pub fn rebuild(
    config: &crate::config::LauncherConfig,
) -> Result<scan::ScanStats, snapshot::SnapshotError> {
    let started = std::time::Instant::now();
    let (b, stats) = scan::build_table(&scan::ScanConfig::from_launcher(config));
    let generation = snapshot::publish(&snapshot::default_dir(), &b)?;
    tracing::info!(
        "파일 색인 세대 {generation}: 폴더 {} 파일 {} (빠짐 {}) — {}ms",
        stats.dirs,
        stats.files,
        stats.skipped_files,
        started.elapsed().as_millis()
    );
    Ok(stats)
}
