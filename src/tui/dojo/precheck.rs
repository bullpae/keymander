//! 입문 의식의 사전 점검 (docs/10 §6-3) — 키맵 백엔드, 배치 신뢰도, 터미널.

use super::binding::Trust;

/// 지금 키맵을 돌리는 것
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// 내장 데몬 (Windows / macOS)
    Daemon,
    /// kanata 프로세스
    Kanata,
    /// 아무것도 안 돈다 — 레이어 수련은 판정이 안 된다
    Off,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Backend::Daemon => "내장 데몬 실행 중",
            Backend::Kanata => "kanata 실행 중",
            Backend::Off => "꺼져 있음",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Report {
    pub backend: Backend,
    pub trust: Trust,
    pub profile: String,
    /// 터미널 식별자 (`TERM_PROGRAM` → Windows Terminal → `TERM`)
    pub terminal: Option<String>,
}

/// 점검을 한 번 돈다. 데몬 Ping 은 로컬 소켓 하나라 가볍다.
pub fn run(config: &kmd_core::Config) -> Report {
    let backend = if kmd_core::ipc::is_daemon_running() {
        Backend::Daemon
    } else if kmd_core::keymap::is_running() {
        Backend::Kanata
    } else {
        Backend::Off
    };
    let trust = match backend {
        Backend::Kanata if !kanata_profile_is_builtin(config) => Trust::Unknown,
        _ => Trust::Full,
    };
    Report {
        backend,
        trust,
        profile: config.launcher.keymap.active_profile.clone(),
        terminal: terminal_id(),
    }
}

/// kanata 가 내장 프리셋 원본 그대로를 쓰는가 — 직접 고친 `.kbd`면 배치를 해석할 수 없다
fn kanata_profile_is_builtin(config: &kmd_core::Config) -> bool {
    let name = &config.launcher.keymap.active_profile;
    let Some(preset) = kmd_core::keymap::preset_content(name) else {
        return false;
    };
    let mut path = kmd_core::keymap::active_profile_path(config);
    if !path.exists() {
        path = kmd_core::keymap::profile_dir(config).join(kmd_core::keymap::with_extension(name));
    }
    std::fs::read_to_string(path).is_ok_and(|text| text.trim() == preset.trim())
}

pub fn terminal_id() -> Option<String> {
    if let Ok(t) = std::env::var("TERM_PROGRAM") {
        return Some(t);
    }
    if std::env::var_os("WT_SESSION").is_some() {
        return Some("Windows Terminal".into());
    }
    std::env::var("TERM").ok()
}
