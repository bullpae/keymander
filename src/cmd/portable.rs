//! `kmd portable` — manage portable mode

use color_eyre::Result;

/// Actions for the portable subcommand.
pub enum Action {
    /// Show current mode and paths
    Status,
    /// Enable portable mode (create kmd-data/, migrate data)
    Enable,
    /// Disable portable mode (migrate data to system paths, remove kmd-data/)
    Disable,
}

/// Files to migrate between system and portable modes.
const DATA_FILES: &[&str] = &[
    kmd_core::CONFIG_FILENAME,
    kmd_core::DB_FILENAME,
    kmd_core::INDEX_CACHE_FILENAME,
];

pub fn run(action: Option<Action>) -> Result<()> {
    let action = action.unwrap_or(Action::Status);

    match action {
        Action::Status => show_status(),
        Action::Enable => enable_portable(),
        Action::Disable => disable_portable(),
    }
}

fn show_status() -> Result<()> {
    let is_portable = kmd_core::portable::is_portable();
    let config_dir = kmd_core::Config::default_config_dir();
    let data_dir = kmd_core::Config::default_data_dir();

    if is_portable {
        println!("Mode:       Portable");
        println!("Data dir:   {}", data_dir.display());
    } else {
        println!("Mode:       System");
        println!("Config dir: {}", config_dir.display());
        println!("Data dir:   {}", data_dir.display());
    }

    if let Some(portable_dir) = kmd_core::portable::portable_data_dir() {
        println!(
            "Portable dir ({}): {}",
            if is_portable { "active" } else { "inactive" },
            portable_dir.display(),
        );
    }

    // List existing data files
    println!();
    println!("Data files:");
    for name in DATA_FILES {
        let path = data_dir.join(name);
        if path.exists() {
            let meta = std::fs::metadata(&path).ok();
            let size = meta.map(|m| format_size(m.len())).unwrap_or_default();
            println!("  [exists] {} ({})", name, size);
        } else {
            println!("  [none]   {}", name);
        }
    }

    Ok(())
}

fn enable_portable() -> Result<()> {
    if kmd_core::portable::is_portable() {
        println!("Already in portable mode.");
        show_status()?;
        return Ok(());
    }

    // Remember system paths BEFORE enabling (once kmd-data/ exists, paths change)
    let sys_config_dir = kmd_core::Config::default_config_dir();
    let sys_data_dir = kmd_core::Config::default_data_dir();

    // Create the portable directory
    let portable_dir = kmd_core::portable::enable()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to create portable directory: {}", e))?;

    println!("Created: {}", portable_dir.display());

    // Migrate existing data files
    let mut migrated = 0;
    for name in DATA_FILES {
        // config.toml comes from config_dir, others from data_dir
        let src = if *name == "config.toml" {
            sys_config_dir.join(name)
        } else {
            sys_data_dir.join(name)
        };

        if src.exists() {
            let dst = portable_dir.join(name);
            if !dst.exists() {
                std::fs::copy(&src, &dst)?;
                println!("  Copied: {} -> {}", src.display(), dst.display());
                migrated += 1;
            }
        }
    }

    if migrated == 0 {
        println!("  No existing data to migrate.");
    }

    println!();
    println!("Portable mode enabled.");
    println!("All data is now stored in: {}", portable_dir.display());
    println!("Original system files were preserved (not deleted).");

    Ok(())
}

fn disable_portable() -> Result<()> {
    if !kmd_core::portable::is_portable() {
        println!("Not in portable mode.");
        show_status()?;
        return Ok(());
    }

    let portable_dir = kmd_core::portable::portable_data_dir()
        .ok_or_else(|| color_eyre::eyre::eyre!("Cannot determine portable directory"))?;

    // Compute system paths (these are what they WOULD be without portable mode)
    let sys_config_dir = kmd_core::Config::system_config_dir();
    let sys_data_dir = kmd_core::Config::standard_data_dir();

    // Ensure system directories exist
    std::fs::create_dir_all(&sys_config_dir)?;
    std::fs::create_dir_all(&sys_data_dir)?;

    // Migrate data files to system paths
    let mut migrated = 0;
    for name in DATA_FILES {
        let src = portable_dir.join(name);
        if src.exists() {
            let dst = if *name == "config.toml" {
                sys_config_dir.join(name)
            } else {
                sys_data_dir.join(name)
            };
            std::fs::copy(&src, &dst)?;
            println!("  Copied: {} -> {}", src.display(), dst.display());
            migrated += 1;
        }
    }

    if migrated == 0 {
        println!("  No data to migrate.");
    }

    // Remove the portable directory
    kmd_core::portable::disable()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to remove portable directory: {}", e))?;

    println!();
    println!("System mode restored.");
    println!("Config: {}", sys_config_dir.display());
    println!("Data:   {}", sys_data_dir.display());

    Ok(())
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
