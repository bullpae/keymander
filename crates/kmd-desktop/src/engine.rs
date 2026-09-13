//! Search engine bootstrap — config loading + index management.
//!
//! Extracted so that `app.rs` focuses only on Elm state/update/view,
//! and this module handles all kmd-core integration concerns.
use std::time::{Duration, Instant};

use kmd_core::{QUICK_INDEX_CACHE_BIN_FILENAME, QUICK_INDEX_CACHE_FILENAME};

/// 캐시 신선도 폴백 한계. 평상시에는 데몬 리프레셔가
/// `launcher.index_refresh_minutes` 주기로 캐시를 갱신하므로 항상 히트하고,
/// 이 한계는 데몬이 꺼져 있을 때만 재빌드를 유발한다 (부팅 후 비동기 경로라
/// 창 표시는 지연되지 않음).
const INDEX_FRESHNESS_SECS: u64 = 24 * 60 * 60;

/// Load the user configuration, falling back to defaults on failure.
pub fn load_config() -> kmd_core::Config {
    let config_dir = kmd_core::Config::default_config_dir();
    match kmd_core::Config::load(&config_dir) {
        Ok(config) => config,
        Err(e) => {
            tracing::warn!(
                "Failed to load config from {}: {e} — using defaults",
                config_dir.display()
            );
            kmd_core::Config::default()
        }
    }
}

/// Build a search engine loaded with the full index.
///
/// Tries loading a cached index first; rebuilds and saves when stale or missing.
pub fn create_search_engine(config: &kmd_core::Config) -> kmd_core::SearchEngine {
    let started = Instant::now();
    let index = load_or_build_index(config);

    tracing::info!("Loaded {} items into search engine", index.items.len());

    let mut engine = kmd_core::SearchEngine::new();
    engine.set_kind_weights(config.launcher.kind_weights.clone());
    engine.load(index.items);
    tracing::info!(
        "Full search engine ready in {} ms",
        started.elapsed().as_millis()
    );
    engine
}

/// Build a lightweight engine for instant first interaction.
///
/// Includes fast sources (apps/PATH/system commands) and skips file crawling.
/// The full engine is loaded asynchronously and replaces this shortly after boot.
pub fn create_quick_search_engine(config: &kmd_core::Config) -> kmd_core::SearchEngine {
    let started = Instant::now();

    let index = load_or_build_quick_index(config.general.emoji_icons);
    let count = index.items.len();

    let mut engine = kmd_core::SearchEngine::new();
    engine.set_kind_weights(config.launcher.kind_weights.clone());
    engine.load(index.items);
    tracing::info!(
        "Quick search engine ready in {} ms ({} items)",
        started.elapsed().as_millis(),
        count
    );
    engine
}

fn load_or_build_quick_index(use_emoji: bool) -> kmd_core::Index {
    let desktop_dir = kmd_core::Config::default_data_dir().join("desktop");
    load_index_logged(
        "Quick index",
        &desktop_dir.join(QUICK_INDEX_CACHE_BIN_FILENAME),
        &desktop_dir.join(QUICK_INDEX_CACHE_FILENAME),
        || kmd_core::Index::build_quick(use_emoji),
    )
}

/// 인덱스 로드: bincode → JSON fallback → 새로 빌드.
/// 캐시가 24시간보다 오래되면 새로 빌드하여 새 앱/CLI를 반영한다.
fn load_or_build_index(config: &kmd_core::Config) -> kmd_core::Index {
    let data_dir = kmd_core::Config::default_data_dir();
    load_index_logged(
        "Index",
        &data_dir.join(kmd_core::INDEX_CACHE_BIN_FILENAME),
        &data_dir.join(kmd_core::INDEX_CACHE_FILENAME),
        || kmd_core::Index::build(&config.launcher, config.general.emoji_icons),
    )
}

/// 데스크톱 공통 인덱스 로드 — 코어의 캐시/빌드 절차에 소요 시간 로깅만 얹는다.
///
/// 두 캐시 모두 데몬 리프레셔가 갱신하므로 freshness 한계를 둔다 —
/// 데몬이 없을 때 낡은 캐시에 갇히지 않기 위한 폴백이다.
fn load_index_logged(
    label: &str,
    bin_path: &std::path::Path,
    json_path: &std::path::Path,
    build: impl FnOnce() -> kmd_core::Index,
) -> kmd_core::Index {
    let started = Instant::now();
    let max_age = Some(Duration::from_secs(INDEX_FRESHNESS_SECS));
    let (index, built) =
        kmd_core::index::store::load_cached_or_build(bin_path, json_path, max_age, build);
    let elapsed = started.elapsed().as_millis();
    if built {
        tracing::info!("{label} rebuilt from source in {elapsed} ms");
    } else {
        tracing::info!("{label} cache hit in {elapsed} ms");
    }
    index
}
