//! `kmd config` — manage configuration

use color_eyre::Result;

pub enum Action {
    Get(String),
    Set(String, String),
    Edit,
    Path,
}

pub fn run(action: Option<Action>) -> Result<()> {
    let mut config = super::load_config()?;

    match action {
        Some(Action::Get(key)) => match config.get_value(&key) {
            Some(value) => println!("{}", value),
            None => {
                return Err(color_eyre::eyre::eyre!("Unknown config key: {}", key));
            }
        },
        Some(Action::Set(key, value)) => {
            config
                .set_value(&key, &value)
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
            config
                .save()
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
            println!("Set {} = {}", key, value);
        }
        Some(Action::Edit) => {
            let config_path = config
                .config_path
                .as_ref()
                .ok_or_else(|| color_eyre::eyre::eyre!("No config path"))?;

            // Create default config if it doesn't exist
            if !config_path.exists() {
                config
                    .save()
                    .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
                println!("Created default config at: {}", config_path.display());
            }

            let editor = config
                .general
                .editor
                .clone()
                .or_else(|| std::env::var("EDITOR").ok())
                .or_else(|| std::env::var("VISUAL").ok())
                .unwrap_or_else(|| {
                    if cfg!(target_os = "windows") {
                        "notepad".to_string()
                    } else {
                        "vi".to_string()
                    }
                });

            println!("Opening {} with {}...", config_path.display(), editor);
            super::editor_command(&editor).arg(config_path).status()?;
        }
        Some(Action::Path) | None => {
            let config_dir = kmd_core::Config::default_config_dir();
            let config_path = config_dir.join(kmd_core::CONFIG_FILENAME);
            println!("{}", config_path.display());
        }
    }

    Ok(())
}
