//! Index persistence — save/load index to/from disk (JSON + bincode)

use std::path::Path;

use super::Index;

// ── JSON (레거시 호환) ──────────────────────────────────────────────────────

/// JSON 형식으로 인덱스 저장
pub fn save_index(index: &Index, path: &Path) -> Result<(), StoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(StoreError::Io)?;
    }
    let json = serde_json::to_string(index).map_err(StoreError::Json)?;
    write_atomic(path, json.as_bytes()).map_err(StoreError::Io)?;
    Ok(())
}

/// tmp 파일에 쓴 뒤 rename — 데몬이 백그라운드로 캐시를 갱신하는 동안
/// 데스크톱이 같은 파일을 읽어도 잘린(torn) 파일을 보지 않게 한다.
///
/// 인덱스는 사용자의 파일 경로 목록(Desktop/Documents/Downloads 등)을 담으므로
/// 공유 머신에서 다른 사용자가 읽지 못하도록 Unix에서 **0600으로 생성**한다.
/// chmod로 사후 변경하면 그 사이에 다른 사용자가 읽을 수 있어, 처음부터 0600으로
/// 만든다. rename은 권한을 보존하므로 최종 파일도 0600을 유지한다.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    // 프로세스별 고유 tmp — 데몬과 데스크톱이 동시에 캐시를 써도 같은 tmp를
    // 밟지 않는다(기존 고정 tmp 이름은 동시 쓰기 시 파일이 섞일 수 있었다).
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp); // 같은 pid의 이전 크래시 잔여물 제거

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }

    let write_result = (|| -> std::io::Result<()> {
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)
    })();
    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    std::fs::rename(&tmp, path)
}

/// JSON 형식에서 인덱스 로드
pub fn load_index(path: &Path) -> Result<Index, StoreError> {
    let content = std::fs::read_to_string(path).map_err(StoreError::Io)?;
    let index: Index = serde_json::from_str(&content).map_err(StoreError::Json)?;
    Ok(index)
}

// ── Bincode (고속 바이너리) ──────────────────────────────────────────────────

/// bincode 형식으로 인덱스 저장
pub fn save_index_bin(index: &Index, path: &Path) -> Result<(), StoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(StoreError::Io)?;
    }
    let bytes = bincode::serde::encode_to_vec(index, bincode::config::standard())
        .map_err(StoreError::BincodeEncode)?;
    write_atomic(path, &bytes).map_err(StoreError::Io)?;
    Ok(())
}

/// bincode 형식에서 인덱스 로드
///
/// bincode 1.x 시절 캐시는 디코딩에 실패하며, 호출부(try_load_cached)가
/// JSON 캐시 → 전체 리빌드로 폴백하므로 업그레이드 시 자동 재생성된다.
pub fn load_index_bin(path: &Path) -> Result<Index, StoreError> {
    let bytes = std::fs::read(path).map_err(StoreError::Io)?;
    let (index, _len): (Index, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
            .map_err(StoreError::BincodeDecode)?;
    Ok(index)
}

// ── 공용 캐시 헬퍼 ──────────────────────────────────────────────────────────

/// bincode + JSON 캐시 동시 저장 (개별 실패는 경고 로그)
pub fn save_both(index: &Index, bin_path: &Path, json_path: &Path) {
    if let Err(e) = save_index_bin(index, bin_path) {
        tracing::warn!("Failed to save bincode cache: {e}");
    }
    if let Err(e) = save_index(index, json_path) {
        tracing::warn!("Failed to save JSON cache: {e}");
    }
}

/// 캐시에서 인덱스 로드 시도 (bincode → JSON fallback).
/// `expected_version`과 일치해야 반환. JSON 히트 시 bincode 캐시를 자동 생성.
pub fn try_load_cached(bin_path: &Path, json_path: &Path, expected_version: &str) -> Option<Index> {
    try_load_cached_with_max_age(bin_path, json_path, expected_version, None)
}

/// freshness 기반 캐시 로드. `max_age`가 Some이면 파일 수정 시각 기준으로
/// 오래된 캐시를 무효화하여 새로 설치된 앱/CLI가 반영되도록 한다.
pub fn try_load_cached_with_max_age(
    bin_path: &Path,
    json_path: &Path,
    expected_version: &str,
    max_age: Option<std::time::Duration>,
) -> Option<Index> {
    if bin_path.exists() && !is_file_too_old(bin_path, max_age) {
        match load_index_bin(bin_path) {
            Ok(idx) if idx.version == expected_version => return Some(idx),
            Ok(_) => tracing::info!("Bincode cache version mismatch, skipping"),
            Err(e) => tracing::warn!("Failed to read bincode cache: {e}"),
        }
    }
    if json_path.exists() && !is_file_too_old(json_path, max_age) {
        match load_index(json_path) {
            Ok(idx) if idx.version == expected_version => {
                let _ = save_index_bin(&idx, bin_path);
                return Some(idx);
            }
            Ok(_) => tracing::info!("JSON cache version mismatch, skipping"),
            Err(e) => tracing::warn!("Failed to read JSON cache: {e}"),
        }
    }
    None
}

/// 캐시 로드 → (실패 시) 빌드 → 저장. CLI·TUI·데스크톱이 각자 반복하던
/// 3단 절차를 한 곳에 둔다.
///
/// 호출자마다 다른 것은 정책뿐이라 인자로 받는다 — 캐시 경로 쌍, `max_age`
/// (None이면 나이 무시), 그리고 캐시 미스일 때 무엇을 빌드할지(`build`).
/// 반환값의 `bool`은 "빌드했는가" — 호출자가 로깅이나 후처리를 구분할 때 쓴다.
pub fn load_cached_or_build(
    bin_path: &Path,
    json_path: &Path,
    max_age: Option<std::time::Duration>,
    build: impl FnOnce() -> Index,
) -> (Index, bool) {
    let expected_version = Index::current_version();
    if let Some(cached) =
        try_load_cached_with_max_age(bin_path, json_path, expected_version, max_age)
    {
        return (cached, false);
    }
    let index = build();
    save_both(&index, bin_path, json_path);
    (index, true)
}

fn is_file_too_old(path: &Path, max_age: Option<std::time::Duration>) -> bool {
    let Some(max) = max_age else {
        return false;
    };
    let Ok(meta) = std::fs::metadata(path) else {
        return true;
    };
    let Ok(modified) = meta.modified() else {
        return true;
    };
    let age = std::time::SystemTime::now()
        .duration_since(modified)
        .unwrap_or(std::time::Duration::from_secs(u64::MAX));
    age > max
}

// ── 에러 타입 ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("I/O error: {0}")]
    Io(std::io::Error),
    #[error("JSON error: {0}")]
    Json(serde_json::Error),
    #[error("Bincode encode error: {0}")]
    BincodeEncode(bincode::error::EncodeError),
    #[error("Bincode decode error: {0}")]
    BincodeDecode(bincode::error::DecodeError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{IndexItem, ItemKind, Source};

    fn sample_index() -> Index {
        let mut index = Index::new();
        index.items.push(IndexItem {
            name: "test.rs".to_string(),
            path: "/home/user/test.rs".to_string(),
            kind: ItemKind::File,
            source: Source::FileProvider,
            icon: "Rs".to_string(),
            keywords: "/home/user/test.rs".to_string(),
            icon_path: None,
        });
        index.items.push(IndexItem {
            name: "docs".to_string(),
            path: "/home/user/docs".to_string(),
            kind: ItemKind::Directory,
            source: Source::FileProvider,
            icon: ">>".to_string(),
            keywords: "/home/user/docs".to_string(),
            icon_path: None,
        });
        index
    }

    /// 캐시로 저장·재로드될 수 있는 인덱스 — `Index::new()`는 version이 비어 있어
    /// 그대로 쓰면 버전 불일치로 항상 거부된다.
    fn versioned_sample_index() -> Index {
        let mut index = sample_index();
        index.version = Index::current_version().to_string();
        index
    }

    #[test]
    fn 캐시가_없으면_빌드하고_저장한다() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("index.bin");
        let json = dir.path().join("index.json");

        let (index, built) = load_cached_or_build(&bin, &json, None, versioned_sample_index);
        assert!(built, "캐시가 없으므로 빌드해야 한다");
        assert_eq!(index.items.len(), 2);
        assert!(
            bin.exists() && json.exists(),
            "빌드 결과를 두 형식으로 저장"
        );
    }

    #[test]
    fn 캐시가_있으면_빌드하지_않는다() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("index.bin");
        let json = dir.path().join("index.json");
        save_both(&versioned_sample_index(), &bin, &json);

        let (index, built) = load_cached_or_build(&bin, &json, None, || {
            panic!("캐시 히트 시 build를 부르면 안 된다")
        });
        assert!(!built);
        assert_eq!(index.items.len(), 2);
    }

    #[test]
    fn 버전이_다른_캐시는_무시하고_다시_빌드한다() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("index.bin");
        let json = dir.path().join("index.json");
        let mut stale = sample_index();
        stale.version = "v0-ancient".to_string();
        save_both(&stale, &bin, &json);

        let (_, built) = load_cached_or_build(&bin, &json, None, versioned_sample_index);
        assert!(built, "스키마 버전이 다르면 캐시를 버린다");
    }

    #[test]
    fn 캐시가_max_age보다_오래되면_다시_빌드한다() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("index.bin");
        let json = dir.path().join("index.json");
        save_both(&versioned_sample_index(), &bin, &json);

        // 나이 0 = 방금 쓴 캐시도 이미 낡은 것으로 본다
        let (_, built) = load_cached_or_build(
            &bin,
            &json,
            Some(std::time::Duration::from_secs(0)),
            versioned_sample_index,
        );
        assert!(built, "max_age를 넘긴 캐시는 무시하고 새로 빌드한다");
    }

    #[test]
    fn test_json_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.json");
        let index = sample_index();

        save_index(&index, &path).unwrap();
        let loaded = load_index(&path).unwrap();

        assert_eq!(loaded.items.len(), 2);
        assert_eq!(loaded.items[0].name, "test.rs");
        assert_eq!(loaded.items[1].kind, ItemKind::Directory);
        assert_eq!(loaded.version, index.version);
    }

    #[test]
    fn test_bincode_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.bin");
        let index = sample_index();

        save_index_bin(&index, &path).unwrap();
        let loaded = load_index_bin(&path).unwrap();

        assert_eq!(loaded.items.len(), 2);
        assert_eq!(loaded.items[0].name, "test.rs");
        assert_eq!(loaded.items[1].kind, ItemKind::Directory);
        assert_eq!(loaded.version, index.version);
    }

    #[test]
    fn test_bincode_smaller_than_json() {
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("index.json");
        let bin_path = dir.path().join("index.bin");
        let index = sample_index();

        save_index(&index, &json_path).unwrap();
        save_index_bin(&index, &bin_path).unwrap();

        let json_size = std::fs::metadata(&json_path).unwrap().len();
        let bin_size = std::fs::metadata(&bin_path).unwrap().len();
        assert!(
            bin_size < json_size,
            "bincode({bin_size}) >= json({json_size})"
        );
    }

    #[test]
    fn test_load_nonexistent_file_returns_error() {
        let result = load_index(std::path::Path::new("/nonexistent/path/index.json"));
        assert!(result.is_err());
        let result = load_index_bin(std::path::Path::new("/nonexistent/path/index.bin"));
        assert!(result.is_err());
    }
}
