//! Plugin discovery and loading

use std::path::{Path, PathBuf};

use super::PluginManifest;

/// Discover plugins in the given directory
pub fn discover_plugins(plugin_dir: &Path) -> Vec<(PathBuf, PluginManifest)> {
    let mut plugins = Vec::new();

    let Ok(entries) = std::fs::read_dir(plugin_dir) else {
        return plugins;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join("manifest.toml");
        if !manifest_path.exists() {
            continue;
        }

        match load_manifest(&manifest_path) {
            Ok(manifest) => {
                tracing::info!("Discovered plugin: {} v{}", manifest.name, manifest.version);
                plugins.push((path, manifest));
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to load plugin manifest at {:?}: {}",
                    manifest_path,
                    e
                );
            }
        }
    }

    plugins
}

/// Load a plugin manifest from a TOML file
fn load_manifest(path: &Path) -> Result<PluginManifest, String> {
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    toml::from_str(&content).map_err(|e| e.to_string())
}

/// Get the default plugin directory — 포터블 모드면 `kmd-data/plugins`
/// (예전엔 OS 데이터 폴더를 직접 조합해 포터블 설치에서도 시스템 경로를 봤다).
pub fn default_plugin_dir() -> PathBuf {
    crate::Config::default_data_dir().join("plugins")
}
