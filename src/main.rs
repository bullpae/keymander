//! keymander (kmd) — CLI-first cross-platform keyboard launcher
//!
//! Usage:
//!   kmd              → Launch TUI interactive mode
//!   kmd search <q>   → Search from CLI
//!   kmd launch <item>→ Launch a program/file
//!   kmd index        → Manage search index
//!   kmd config       → Manage configuration
//!   kmd history      → View/clear launch history
//!   kmd emoji <q>    → Search and copy Unicode emoji
//!   kmd portable     → Manage portable mode

mod cmd;
mod tui;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "kmd",
    version,
    about = "Keyboard-first cross-platform launcher",
    long_about = "키보드 하나로 모든 것을 지휘한다 — CLI-first cross-platform launcher"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Search for applications, files, and commands
    Search {
        /// Search query
        query: String,
        /// Maximum number of results
        #[arg(short, long, default_value = "20")]
        limit: usize,
        /// Output as JSON
        #[arg(short, long)]
        json: bool,
    },
    /// Launch a program, file, or URL
    Launch {
        /// Item to launch (name or path)
        target: String,
    },
    /// Manage the search index
    Index {
        /// Force rebuild the entire index
        #[arg(long)]
        rebuild: bool,
        /// Show index statistics
        #[arg(long)]
        stats: bool,
        /// Suggest active folders outside the current search scope
        #[arg(long)]
        suggest: bool,
    },
    /// Search inside document contents (FTS5 full-text index)
    Grep {
        /// Search query (2+ chars). Operators: "exact phrase", -exclude, ext:md, path:docs
        query: Vec<String>,
        /// Re-sync the content index before searching
        #[arg(long)]
        sync: bool,
        /// Maximum results to show
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    /// Manage configuration
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    /// View and manage launch history
    History {
        #[command(subcommand)]
        action: Option<HistoryAction>,
    },
    /// Manage plugins
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
    /// Launch desktop GUI overlay (Spotlight-style)
    Desktop,
    /// Global hotkey daemon (start/stop/status)
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Manage portable mode (store all data next to exe)
    Portable {
        #[command(subcommand)]
        action: Option<PortableAction>,
    },
    /// Search and copy Unicode emoji
    Emoji {
        /// Search query (e.g. "fire", "heart", "smile")
        query: Option<String>,
        /// Only list results, don't copy to clipboard
        #[arg(short, long)]
        list: bool,
        /// Output results as JSON
        #[arg(short, long)]
        json: bool,
        /// Copy the Nth result to clipboard (default: 1)
        #[arg(short, long)]
        copy: Option<usize>,
    },
    /// Manage prompt templates for @ll multi-LLM
    Prompt {
        #[command(subcommand)]
        action: Option<PromptAction>,
    },
    /// Show version information
    Version,
    /// Manage keymap backend (Kanata prototype)
    Keymap {
        #[command(subcommand)]
        action: KeymapAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Get a config value
    Get { key: String },
    /// Set a config value
    Set { key: String, value: String },
    /// Open config file in editor
    Edit,
    /// Show config file path
    Path,
}

#[derive(Subcommand)]
enum HistoryAction {
    /// List recent launches
    List {
        #[arg(short, long, default_value = "20")]
        limit: usize,
        /// Output as JSON
        #[arg(short, long)]
        json: bool,
    },
    /// Clear all history
    Clear,
}

#[derive(Subcommand)]
enum PluginAction {
    /// List installed plugins
    List,
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Start the daemon
    Start,
    /// Stop the daemon
    Stop,
    /// Restart the daemon (stop, then start)
    Restart,
    /// Show daemon status
    Status,
    /// [macOS] Key-injection self-test through the live event tap (docs/14 Tier 2)
    E2e,
    /// Register daemon to start at login
    Install,
    /// Unregister daemon from starting at login
    Uninstall,
    /// [실험] 지정한 지연 후 전경 앱에 붙여넣기(Cmd+V/Ctrl+V)를 주입한다.
    /// 클립보드 확장(P3) 검증용 — 지연 동안 붙여넣을 앱으로 전환해 테스트한다.
    #[command(hide = true)]
    PasteTest {
        /// 주입 전 대기(ms). 이 시간 안에 대상 앱으로 포커스를 옮긴다.
        #[arg(long, default_value_t = 3000)]
        delay_ms: u64,
    },
    /// [실험] 클립보드 히스토리 n번째 항목을 지연 후 전경 앱에 붙여넣는다.
    /// 클립보드 P1(docs/12 흐름 A) 검증용.
    #[command(hide = true)]
    ClipTest {
        /// 붙여넣을 슬롯 (1=최신). 기본 1.
        #[arg(long, default_value_t = 1)]
        slot: usize,
        /// 주입 전 대기(ms).
        #[arg(long, default_value_t = 3000)]
        delay_ms: u64,
        /// 런처 열기 전 앱으로 포커스를 되돌린 뒤 붙여넣기 (흐름 B).
        #[arg(long)]
        to_previous: bool,
    },
    /// [실험] 현재 전경 앱을 흐름 B 붙여넣기 대상으로 기억 (검증용).
    #[command(hide = true)]
    ClipCapture,
}

#[derive(Subcommand)]
enum PortableAction {
    /// Enable portable mode (create kmd-data/ next to exe, migrate data)
    Enable,
    /// Disable portable mode (migrate data to system paths, remove kmd-data/)
    Disable,
}

#[derive(Subcommand)]
enum PromptAction {
    /// List saved prompt templates
    List,
    /// Add a new prompt template
    Add {
        /// Template name (alphanumeric, hyphens, underscores)
        name: String,
        /// Template body ({query} placeholder supported)
        body: String,
    },
    /// Remove a prompt template
    Remove {
        /// Template name to remove
        name: String,
    },
}

#[derive(Subcommand)]
enum KeymapAction {
    /// Start keymap backend process
    Start,
    /// Stop keymap backend process
    Stop,
    /// Show keymap process status
    Status,
    /// List keymap profiles
    List,
    /// Set active profile
    Use { profile: String },
    /// Create profile from built-in preset and set active (default: vim-nav)
    Init { profile: Option<String> },
    /// Show available built-in presets
    ListPresets,
}

fn main() -> color_eyre::Result<()> {
    // ── 0. Capture the real terminal window (Windows only) ───────────────
    // On Windows 11 with Windows Terminal as default, GetConsoleWindow()
    // returns an internal pseudo-console handle (0×0 rect).  The actual
    // visible window belongs to Windows Terminal.  We capture it via
    // GetForegroundWindow() right at startup, before anything changes.
    #[cfg(windows)]
    win_console::capture_terminal_window();

    // ── 1. Hide console for toggle-off path (Windows only) ───────────────
    // If launched from a shortcut/hotkey, we own the console window.
    // Hide it immediately so the toggle-off path is invisible.
    // If launched from cmd.exe/powershell, we share their console — don't hide.
    #[cfg(windows)]
    let owns_console = win_console::is_sole_console_owner();
    #[cfg(windows)]
    if owns_console {
        win_console::hide();
    }

    // ── 2. Single-instance check ─────────────────────────────────────────
    let data_dir = kmd_core::Config::default_data_dir();
    let instance_guard = match kmd_core::single_instance::acquire_or_toggle(&data_dir) {
        kmd_core::single_instance::InstanceAction::Acquired(guard) => Some(guard),
        kmd_core::single_instance::InstanceAction::SignalledExisting => {
            // Existing instance was told to quit — exit with hidden console.
            return Ok(());
        }
    };

    // ── 3. Set up UTF-8 / VT processing ─────────────────────────────────
    // Window centering is handled by Windows Terminal's `centerOnLaunch`
    // setting.  We just need to show the window when the TUI is ready.
    #[cfg(windows)]
    win_console::setup();

    // ── 3b. Clean up stale diagnostic log from previous versions ────────
    if let Ok(tmp) = std::env::var("TEMP").or_else(|_| std::env::var("TMP")) {
        let log_path = std::path::Path::new(&tmp).join("kmd_center.log");
        let _ = std::fs::remove_file(log_path);
    }

    // ── 4. Normal startup ────────────────────────────────────────────────
    color_eyre::install()?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    // For non-TUI commands launched from a hotkey/shortcut, we need the
    // console visible for output.  TUI mode keeps it hidden until ready.
    #[cfg(windows)]
    if owns_console && cli.command.is_some() {
        win_console::show();
    }

    match cli.command {
        Some(Commands::Search { query, limit, json }) => {
            cmd::search::run(&query, limit, json)?;
        }
        Some(Commands::Launch { target }) => {
            cmd::launch::run(&target)?;
        }
        Some(Commands::Grep { query, sync, limit }) => {
            cmd::grep::run(&query.join(" "), limit, sync)?;
        }
        Some(Commands::Index {
            rebuild,
            stats,
            suggest,
        }) => {
            cmd::index::run(rebuild, stats, suggest)?;
        }
        Some(Commands::Config { action }) => {
            cmd::config::run(action.map(|a| match a {
                ConfigAction::Get { key } => cmd::config::Action::Get(key),
                ConfigAction::Set { key, value } => cmd::config::Action::Set(key, value),
                ConfigAction::Edit => cmd::config::Action::Edit,
                ConfigAction::Path => cmd::config::Action::Path,
            }))?;
        }
        Some(Commands::History { action }) => {
            cmd::history::run(action.map(|a| match a {
                HistoryAction::List { limit, json } => cmd::history::Action::List(limit, json),
                HistoryAction::Clear => cmd::history::Action::Clear,
            }))?;
        }
        Some(Commands::Plugin { action }) => {
            cmd::plugin::run(match action {
                PluginAction::List => cmd::plugin::Action::List,
            })?;
        }
        Some(Commands::Desktop) => {
            cmd::daemon::launch_desktop()?;
        }
        Some(Commands::Daemon { action }) => {
            cmd::daemon::run(match action {
                DaemonAction::Start => cmd::daemon::Action::Start,
                DaemonAction::Stop => cmd::daemon::Action::Stop,
                DaemonAction::Restart => cmd::daemon::Action::Restart,
                DaemonAction::E2e => cmd::daemon::Action::E2e,
                DaemonAction::Status => cmd::daemon::Action::Status,
                DaemonAction::Install => cmd::daemon::Action::Install,
                DaemonAction::Uninstall => cmd::daemon::Action::Uninstall,
                DaemonAction::PasteTest { delay_ms } => cmd::daemon::Action::PasteTest { delay_ms },
                DaemonAction::ClipTest {
                    slot,
                    delay_ms,
                    to_previous,
                } => cmd::daemon::Action::ClipTest {
                    slot,
                    delay_ms,
                    to_previous,
                },
                DaemonAction::ClipCapture => cmd::daemon::Action::ClipCapture,
            })?;
        }
        Some(Commands::Portable { action }) => {
            cmd::portable::run(action.map(|a| match a {
                PortableAction::Enable => cmd::portable::Action::Enable,
                PortableAction::Disable => cmd::portable::Action::Disable,
            }))?;
        }
        Some(Commands::Emoji {
            query,
            list,
            json,
            copy,
        }) => {
            cmd::emoji::run(query.as_deref().unwrap_or(""), list, json, copy)?;
        }
        Some(Commands::Prompt { action }) => {
            cmd::prompt::run(action.map(|a| match a {
                PromptAction::List => cmd::prompt::Action::List,
                PromptAction::Add { name, body } => cmd::prompt::Action::Add { name, body },
                PromptAction::Remove { name } => cmd::prompt::Action::Remove { name },
            }))?;
        }
        Some(Commands::Version) => {
            cmd::version::run();
        }
        Some(Commands::Keymap { action }) => {
            cmd::keymap::run(match action {
                KeymapAction::Start => cmd::keymap::Action::Start,
                KeymapAction::Stop => cmd::keymap::Action::Stop,
                KeymapAction::Status => cmd::keymap::Action::Status,
                KeymapAction::List => cmd::keymap::Action::List,
                KeymapAction::Use { profile } => cmd::keymap::Action::Use { profile },
                KeymapAction::Init { profile } => cmd::keymap::Action::Init { profile },
                KeymapAction::ListPresets => cmd::keymap::Action::ListPresets,
            })?;
        }
        // No subcommand → launch TUI (pass instance guard so event loop can check it)
        None => {
            #[cfg(windows)]
            {
                tui::run(instance_guard, owns_console)?;
            }
            #[cfg(not(windows))]
            {
                tui::run(instance_guard, false)?;
            }
        }
    }

    Ok(())
}

// ── Windows console management ───────────────────────────────────────────────
//
// On Windows 11, the default terminal is Windows Terminal (wt.exe).
// `GetConsoleWindow()` returns an internal pseudo-console handle (rect 0×0),
// not the actual visible window.  We capture the real HWND via
// `GetForegroundWindow()` at process start.
//
// Window **centering** is delegated to Windows Terminal's `centerOnLaunch`
// setting (set in WT's settings.json).  Trying to `MoveWindow` from our
// process is unreliable because WT creates and positions its window before
// our main() runs — any repositioning from our side is a visible race.
//
// The captured HWND is used only for hide/show:
// - **Hotkey launch**: hide immediately, do single-instance check, show
//   after TUI renders the first frame.  Toggle-off path: the second
//   instance hides + signals quit + exits — no visible flash.
// - **cmd.exe / PowerShell**: shared terminal, no hide/show needed.

#[cfg(windows)]
mod win_console {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicIsize, Ordering};

    const SW_HIDE: i32 = 0;
    const SW_SHOW: i32 = 5;
    const STD_OUTPUT_HANDLE: u32 = (-11i32) as u32;
    const CP_UTF8: u32 = 65001;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

    // ── Terminal window handle ───────────────────────────────────────────
    //
    // On Windows 11, the default terminal is Windows Terminal (wt.exe).
    // `GetConsoleWindow()` returns an internal pseudo-console handle
    // (rect 0×0), not the actual visible window.
    //
    // We capture the real window via `GetForegroundWindow()` at process
    // start, when the newly-created terminal window is the foreground.
    //
    // Window **centering** is handled by Windows Terminal's own
    // `centerOnLaunch` setting — trying to `MoveWindow` from our process
    // is a race against WT's own window management.
    //
    // The captured HWND is used only for hide/show (toggle-off path).

    static TERMINAL_HWND: AtomicIsize = AtomicIsize::new(0);

    /// Capture the actual terminal window handle.
    /// Must be called as early as possible in main().
    pub fn capture_terminal_window() {
        unsafe {
            let fg = GetForegroundWindow();
            let hwnd = if !fg.is_null() {
                fg
            } else {
                GetConsoleWindow()
            };
            if !hwnd.is_null() {
                TERMINAL_HWND.store(hwnd as isize, Ordering::SeqCst);
            }
        }
    }

    /// Get the stored terminal window handle.
    fn terminal_hwnd() -> *mut c_void {
        let v = TERMINAL_HWND.load(Ordering::SeqCst);
        if v != 0 {
            v as *mut c_void
        } else {
            unsafe { GetConsoleWindow() }
        }
    }

    /// Check if we are the only process on this console.
    pub fn is_sole_console_owner() -> bool {
        let mut pids = [0u32; 64];
        let count = unsafe { GetConsoleProcessList(pids.as_mut_ptr(), 64) };
        count <= 1
    }

    /// Hide the terminal window immediately.
    pub fn hide() {
        let hwnd = terminal_hwnd();
        if !hwnd.is_null() {
            unsafe {
                ShowWindow(hwnd, SW_HIDE);
            }
        }
    }

    /// Show the terminal window.
    pub fn show() {
        let hwnd = terminal_hwnd();
        if !hwnd.is_null() {
            unsafe {
                ShowWindow(hwnd, SW_SHOW);
            }
        }
    }

    /// Set UTF-8 code page and enable VT processing for ANSI sequences.
    pub fn setup() {
        unsafe {
            SetConsoleOutputCP(CP_UTF8);
            SetConsoleCP(CP_UTF8);

            let handle = GetStdHandle(STD_OUTPUT_HANDLE);
            if !handle.is_null() && handle as isize != -1 {
                let mut mode: u32 = 0;
                if GetConsoleMode(handle, &mut mode) != 0 {
                    SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
                }
            }
        }
    }

    /// Windows Terminal / VS Code 등 이모지를 지원하는 모던 터미널인지 감지.
    /// conhost(레거시 콘솔)에서는 이모지 렌더링이 불가능하므로 ASCII 폴백 필요.
    pub fn is_modern_terminal() -> bool {
        // Windows Terminal은 WT_SESSION 환경변수를 설정
        if std::env::var("WT_SESSION").is_ok() {
            return true;
        }
        // VS Code, JetBrains 등 내장 터미널
        if std::env::var("TERM_PROGRAM").is_ok() {
            return true;
        }
        false
    }

    // ── FFI declarations ─────────────────────────────────────────────────
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetConsoleWindow() -> *mut c_void;
        fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
        fn GetStdHandle(std_handle: u32) -> *mut c_void;
        fn GetConsoleMode(handle: *mut c_void, mode: *mut u32) -> i32;
        fn SetConsoleMode(handle: *mut c_void, mode: u32) -> i32;
        fn SetConsoleOutputCP(code_page: u32) -> i32;
        fn SetConsoleCP(code_page: u32) -> i32;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn ShowWindow(hwnd: *mut c_void, cmd_show: i32) -> i32;
        fn GetForegroundWindow() -> *mut c_void;
    }
}
