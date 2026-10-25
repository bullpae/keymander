//! TUI interactive launcher frontend
//!
//! Activated when `kmd` is run with no subcommand.

pub mod app;
pub mod dojo;
pub mod event;
pub mod settings;
pub mod theme;
pub mod ui;

use color_eyre::Result;
use kmd_core::single_instance::Guard;

/// Run the TUI launcher.
///
/// `show_on_ready` — if true, show the console window after the first TUI
/// frame renders (for hotkey launch where the window starts hidden).
pub fn run(instance_guard: Option<Guard>, show_on_ready: bool) -> Result<()> {
    app::run_app(instance_guard, show_on_ready)
}
