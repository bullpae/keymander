//! kmd-core — keymander core library
//!
//! Provides indexing, search, configuration, database, and action execution
//! for the keymander keyboard launcher.

pub mod action;
pub mod config;
pub mod content_index;
pub mod db;
pub mod folder_search;
pub mod folder_suggest;
pub mod fsutil;
pub mod hangul;
pub mod history;
pub mod index;
pub mod ipc;
pub mod keymap;
pub mod plugin;
pub mod portable;
pub mod process;
pub mod prompt;
pub mod query_prefix;
pub mod search;
pub mod single_instance;
pub mod textenc;
pub mod transform;
pub mod web;

/// Re-export commonly used types
pub use config::Config;
pub use db::Database;
pub use index::{Index, IndexItem, ItemKind, Source};
pub use search::{SearchEngine, SearchMode, SearchResult};

// ── Well-known file names ────────────────────────────────────────────────────
/// Configuration file name.
pub const CONFIG_FILENAME: &str = "config.toml";
/// SQLite database file name.
pub const DB_FILENAME: &str = "kmd.db";
/// Index cache file name (JSON, legacy fallback).
pub const INDEX_CACHE_FILENAME: &str = "index.json";
/// Index cache file name (bincode, fast binary format).
pub const INDEX_CACHE_BIN_FILENAME: &str = "index.bin";
/// Quick index cache file name (JSON) — `<data_dir>/desktop/` 하위.
/// 데스크톱 부팅 즉시 응답용 경량 인덱스로, 데몬 백그라운드 리프레시도
/// 같은 경로를 갱신한다.
pub const QUICK_INDEX_CACHE_FILENAME: &str = "quick-index.json";
/// Quick index cache file name (bincode) — `<data_dir>/desktop/` 하위.
pub const QUICK_INDEX_CACHE_BIN_FILENAME: &str = "quick-index.bin";
