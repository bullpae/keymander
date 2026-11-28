pub mod config;
pub mod daemon;
pub mod dojo;
pub mod emoji;
pub mod golden;
pub mod grep;
pub mod history;
pub mod index;
pub mod keymap;
pub mod launch;
pub mod memo;
pub mod plugin;
pub mod portable;
pub mod prompt;
pub mod say;
pub mod scope;
pub mod search;
pub mod upgrade;
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
    Ok(kmd_core::Database::open_default(kmd_core::DbRole::Shared)?)
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
    kmd_core::SearchEngine::with_items(&config.launcher.kind_weights, index.items)
}

/// `$EDITOR` 값으로 편집기 명령을 만든다. `code --wait` 처럼 인자가 붙은 값이 흔하다 — 통째로
/// 실행 파일 이름으로 쓰면 찾지 못한다. 값 전체가 있는 파일이면(공백 든 Windows 경로) 그대로 쓴다.
pub fn editor_command(editor: &str) -> std::process::Command {
    let editor = editor.trim();
    if std::path::Path::new(editor).is_file() {
        return std::process::Command::new(editor);
    }
    let mut parts = editor.split_whitespace();
    let mut cmd = std::process::Command::new(parts.next().unwrap_or(editor));
    cmd.args(parts);
    cmd
}

#[cfg(test)]
mod editor_tests {
    #[test]
    fn 인자가_붙은_편집기를_나눈다() {
        let cmd = super::editor_command("code --wait");
        assert_eq!(cmd.get_program(), "code");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args, ["--wait"]);
        assert_eq!(super::editor_command("vi").get_args().count(), 0);
    }
}
