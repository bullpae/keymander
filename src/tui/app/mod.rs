//! TUI application state and main event loop

use std::path::{Path, PathBuf};

use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste, KeyCode, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use kmd_core::action;
use kmd_core::hangul::{self, HangulComposer};
use kmd_core::index::{
    files::{dir_icon, icon_for_path},
    ItemKind, Source,
};
use kmd_core::plugin::{builtin_calc, builtin_shell, Extension};
use kmd_core::query_prefix::QueryPrefix;
use kmd_core::search::{SearchEngine, SearchMode, SearchResult};
use kmd_core::web;

use super::event::{AppEvent, EventHandler};
use super::settings::{self, SettingsAction, SettingsState};
use super::theme::Theme;
use super::ui;

mod drill;
mod execute;
mod input;
mod run;
mod scope;
mod search;
mod settings_bridge;
mod state;

use drill::{drill_back, drill_into_folder, list_directory_contents};
use execute::{execute_selected, poll_pending_shell};
use input::{flush_composer, handle_key, handle_paste};
pub use run::run_app;
use search::{handle_keymap_query, load_history_into_results, update_search};
use settings_bridge::{handle_settings_key_event, save_config_change};
pub use state::AppState;
use state::DrillState;

// ── Constants ────────────────────────────────────────────────────────────────

/// Max results returned by fuzzy search
const SEARCH_RESULT_LIMIT: usize = 50;

/// Max history entries shown when query is empty
const HISTORY_DISPLAY_LIMIT: usize = 20;

/// Score for web service list items (@ prefix browsing)
const SCORE_WEB_LIST: u32 = 0;

/// Score for a specific web search result
const SCORE_WEB_SEARCH: u32 = 100;

/// Score for explicit calculator results (:calc prefix)
const SCORE_CALC: u32 = 1000;

/// Score for inline calculator results (always on top)
const SCORE_CALC_INLINE: u32 = u32::MAX;

/// Score for directory listing items
const SCORE_DIR_LISTING: u32 = 0;

/// Multiplier for history frequency → score
const HISTORY_SCORE_MULTIPLIER: u32 = 100;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Wrap items into SearchResults with a uniform score
fn items_to_results(
    items: impl IntoIterator<Item = kmd_core::IndexItem>,
    score: u32,
) -> Vec<SearchResult> {
    items
        .into_iter()
        .map(|item| SearchResult { item, score })
        .collect()
}

#[cfg(test)]
mod tests;
