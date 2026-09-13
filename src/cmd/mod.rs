pub mod config;
pub mod daemon;
pub mod emoji;
pub mod grep;
pub mod history;
pub mod index;
pub mod keymap;
pub mod launch;
pub mod plugin;
pub mod portable;
pub mod prompt;
pub mod search;
pub mod version;

use std::path::PathBuf;

/// 인덱스 로드 순서: bincode → JSON fallback → 새로 빌드.
/// 빌드 후에는 bincode + JSON 둘 다 저장.
pub fn load_or_build_index(
    launcher_config: &kmd_core::config::LauncherConfig,
    use_emoji: bool,
) -> kmd_core::Index {
    let (bin_path, json_path) = index_cache_paths();
    // CLI는 나이를 따지지 않는다 — 데스크톱만 데몬 부재 폴백용 freshness를 건다.
    let (index, _built) =
        kmd_core::index::store::load_cached_or_build(&bin_path, &json_path, None, || {
            kmd_core::Index::build(launcher_config, use_emoji)
        });
    index
}

/// (bincode, json) 인덱스 캐시 경로 쌍 반환
pub(crate) fn index_cache_paths() -> (PathBuf, PathBuf) {
    let data_dir = kmd_core::Config::default_data_dir();
    (
        data_dir.join(kmd_core::INDEX_CACHE_BIN_FILENAME),
        data_dir.join(kmd_core::INDEX_CACHE_FILENAME),
    )
}

/// Open the database
pub fn open_db() -> color_eyre::Result<kmd_core::Database> {
    let db_path = kmd_core::Config::default_data_dir().join(kmd_core::DB_FILENAME);
    let db = kmd_core::Database::open(&db_path)?;
    Ok(db)
}

/// Load config
pub fn load_config() -> color_eyre::Result<kmd_core::Config> {
    let config_dir = kmd_core::Config::default_config_dir();
    let config = kmd_core::Config::load(&config_dir)?;
    Ok(config)
}

/// Create a search engine loaded with the full index.
/// Shared helper used by `search` and `launch` subcommands.
pub fn create_search_engine(config: &kmd_core::Config) -> kmd_core::SearchEngine {
    let index = load_or_build_index(&config.launcher, config.general.emoji_icons);
    let mut engine = kmd_core::SearchEngine::new();
    engine.set_kind_weights(config.launcher.kind_weights.clone());
    engine.load(index.items);
    engine
}
