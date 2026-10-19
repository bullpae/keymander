//! TUI 상태 — AppState 와 드릴다운 복귀 상태

use super::*;

// ── Application State ────────────────────────────────────────────────────────

/// Application state
pub struct AppState {
    /// Current search query (committed text only)
    pub query: String,
    /// Search results
    pub results: Vec<SearchResult>,
    /// Currently selected index
    pub selected_index: usize,
    /// Total indexed items
    pub total_items: usize,
    /// Show preview panel
    pub show_preview: bool,
    /// Preview panel width percentage (from config)
    pub preview_width_percent: u16,
    /// Current search mode
    pub(super) search_mode: SearchMode,
    /// Whether to quit
    pub(super) should_quit: bool,
    /// Whether to quit after launch
    pub(super) quit_on_launch: bool,
    /// Korean (Hangul) input mode
    pub hangul_mode: bool,
    /// Whether hangul mode was auto-activated (e.g. by :emoji prefix)
    pub hangul_auto: bool,
    /// Currently composing character (during Korean input)
    pub composing: Option<char>,
    /// Hangul composition engine
    pub(super) composer: HangulComposer,
    /// Folder drill-down stack
    pub(super) drill_stack: Vec<DrillState>,
    /// Current drill-down directory path (None = normal search mode)
    pub drill_path: Option<PathBuf>,
    /// Temporary status message (e.g. "Copied to clipboard")
    pub status_message: Option<String>,
    /// Settings modal (None = hidden, Some = active)
    pub settings: Option<SettingsState>,
    /// Portable mode indicator
    pub is_portable: bool,
    /// Use emoji icons (mirrors config.general.emoji_icons)
    pub use_emoji: bool,
    /// LLM 오토파일럿(데몬 키 주입 자동 제출) opt-in 여부.
    pub llm_autopilot: bool,
    /// 로드된 설정 — **키 입력마다 디스크를 다시 읽지 않기 위한 캐시**.
    /// 예전에는 `:keys`/`:keymap`/`:prompt`/`?` 핸들러가 매 키 입력마다
    /// `load_config()`로 config.toml을 읽었다 (데스크톱은 캐시를 쓴다).
    /// 설정을 바꾸는 경로는 이 값을 함께 갱신해 캐시가 어긋나지 않게 한다.
    pub config: kmd_core::Config,
    /// frecency 부스트 맵 — **키 입력마다 DB를 조회하지 않기 위한 캐시**.
    /// 예전에는 검색마다 `history::boost_results`가 `query_history(500)` +
    /// 맵 구축을 돌았다(데스크톱은 같은 자리에서 미리 로드한 맵을 쓴다).
    /// 시작 시 1회 로드하고, 실행을 기록한 직후에만 다시 읽는다.
    pub(super) frecency: kmd_core::history::FrecencyMap,
    /// 실행 중인 quick action의 결과 채널 (REF-05).
    ///
    /// quick action은 최대 10초(+읽기 유예 2초) 걸릴 수 있다. 예전에는 이벤트
    /// 루프 안에서 동기로 기다려 그동안 **키 입력도 화면 갱신도 멈췄다**.
    /// 이제 워커 스레드에서 돌리고, Tick마다 `poll_pending_shell`이 확인한다
    /// (데스크톱은 같은 작업을 워커 + `handle_shell_done`으로 처리한다).
    pub(super) pending_shell: Option<std::sync::mpsc::Receiver<kmd_core::plugin::ExtensionAction>>,
    /// Cached effective query (query + composing char), updated on every input change
    pub(super) cached_effective_query: String,
    /// Whether the UI needs to be redrawn
    pub dirty: bool,
}

/// Saved state for returning from a folder drill-down
pub(super) struct DrillState {
    pub(super) query: String,
    pub(super) results: Vec<SearchResult>,
    pub(super) selected_index: usize,
    pub(super) search_mode: SearchMode,
    pub(super) parent_drill_path: Option<PathBuf>,
}

impl AppState {
    /// `config`에서 파생되는 필드들을 다시 채운다.
    ///
    /// 이 필드들은 렌더·핸들러 경로에서 자주 읽혀 캐시로 두고 있다. 정본은 항상
    /// `self.config`이고, 설정이 바뀌는 경로(설정 모달 저장 등)는 config를 고친 뒤
    /// 이 함수 하나만 부르면 된다 — 예전에는 저장 시 12줄을 손으로 재대입해서
    /// 필드가 늘 때마다 누락되기 쉬웠다. `emoji_icons`는 인덱스 재빌드와 함께
    /// 다뤄야 해서 호출자가 따로 처리한다.
    pub(super) fn sync_config_mirrors(&mut self) {
        let general = &self.config.general;
        let launcher = &self.config.launcher;
        self.show_preview = general.show_preview;
        self.preview_width_percent = general.preview_width_percent;
        self.quit_on_launch = launcher.quit_on_launch;
        self.llm_autopilot = launcher.llm_autopilot;
    }

    /// 저장된 설정으로 캐시를 교체하고 파생 필드를 다시 채운다.
    ///
    /// 미리보기 표시는 Ctrl+P로 바꾸는 **런타임 상태**이기도 하다 — 예전엔 아무 설정
    /// 저장(`:prompt add` 등)에도 config 값으로 덮어써 사용자가 켠 미리보기가 꺼졌다.
    /// 설정의 show_preview 값 자체가 바뀐 경우에만 반영한다.
    pub(super) fn replace_config(&mut self, saved: kmd_core::Config) {
        let preview_changed = saved.general.show_preview != self.config.general.show_preview;
        let runtime_preview = self.show_preview;
        self.config = saved;
        self.sync_config_mirrors();
        if !preview_changed {
            self.show_preview = runtime_preview;
        }
    }

    pub fn search_mode_label(&self) -> &str {
        self.search_mode.label()
    }

    /// Get the effective query for display and search (query + composing char).
    /// Returns a borrowed `&str` from the internal cache — no allocation.
    pub fn effective_query(&self) -> &str {
        &self.cached_effective_query
    }

    /// Rebuild the cached effective query from `query` + optional `composing` char.
    /// Must be called whenever `query` or `composing` changes.
    pub(super) fn refresh_effective_query(&mut self) {
        self.cached_effective_query.clear();
        self.cached_effective_query.push_str(&self.query);
        if let Some(c) = self.composing {
            self.cached_effective_query.push(c);
        }
    }

    /// Mark the UI as needing a redraw.
    pub(super) fn mark_dirty(&mut self) {
        self.dirty = true;
    }
}
