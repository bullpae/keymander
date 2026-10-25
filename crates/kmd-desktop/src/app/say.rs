//! `:say` 입버릇 — 런처 한 줄 카드와 타이핑 연습 (docs/21)
//!
//! 해석·채점·저장은 `kmd_core::say` 가 하고, 여기서는 화면에 붙이기만 한다.

use super::*;

use kmd_core::say::{self, command};

/// 런처 한 줄 카드 — 프로세스(= 런처 창 하나)마다 한 번 읽는다
#[derive(Debug, Default)]
pub(super) struct SayCardSlot {
    /// 읽었는지 — 빈 입력으로 돌아올 때마다 파일을 다시 읽지 않는다
    loaded: bool,
    card: Option<say::Card>,
    /// Tab 으로 영어를 열었는지
    revealed: bool,
}

/// 입버릇 파일 위치 — 테스트는 사용자 데이터 대신 임시 폴더를 쓴다 (`config_save_path` 와 같은 이유)
pub(super) fn default_say_store() -> say::SayStore {
    #[cfg(test)]
    {
        say::SayStore::new(
            std::env::temp_dir().join(format!("kmd_desktop_test_say_{}", std::process::id())),
        )
    }
    #[cfg(not(test))]
    {
        say::SayStore::default_location()
    }
}

impl App {
    fn say_ctx(&self) -> command::Ctx<'_> {
        command::Ctx {
            cfg: &self.runtime_config.say,
            now: say::now_ms(),
            revealed: self.say_revealed.as_deref(),
            use_emoji: self.use_emoji,
        }
    }

    pub(super) fn handle_say_query(&mut self, query: &str) {
        let items = command::items(query, &self.say_store, self.say_ctx());
        self.apply_contains_items(items);
    }

    /// `:say` Enter 확정 — 현재 질의를 다시 해석해 적용·저장한다.
    pub(super) fn commit_say_command(&mut self, target: Option<String>) -> Task<Message> {
        let trimmed = self.query.trim();
        let normalized = kmd_core::query_prefix::normalize_slash_command(trimmed);
        let query = normalized.as_deref().unwrap_or(trimmed).to_string();
        let applied =
            match command::commit(&query, target.as_deref(), &self.say_store, self.say_ctx()) {
                Ok(a) => a,
                Err(e) => {
                    self.status_message = Some(e);
                    return Task::none();
                }
            };
        if let Some(enabled) = applied.set_enabled {
            // 저장에 실패했는데 켜졌다고 말하면 안 된다
            if let Some(err) = save_config(move |c| c.say.enabled = enabled) {
                self.status_message = Some(err);
                return Task::none();
            }
            self.runtime_config.say.enabled = enabled;
        }
        if let Some(text) = &applied.copy {
            if let Err(e) = kmd_core::clipboard::set_text(text) {
                self.status_message = Some(format!("클립보드 복사 실패: {e}"));
                return Task::none();
            }
        }
        if let Some(id) = applied.reveal {
            self.say_revealed = Some(id);
        }
        if let Some(next) = applied.next_query {
            self.query = next;
            self.selected = 0;
        }
        // 정답 열기처럼 입력이 그대로여도 행을 다시 그려야 한다
        let task = self.perform_search();
        if !applied.message.is_empty() {
            self.status_message = Some(applied.message);
        }
        task
    }

    /// 빈 입력 화면의 카드 — 처음 한 번만 파일을 읽고, 노출 기록은 뒤에서 쓴다.
    /// 최근 실행 항목이 없으면 카드가 보일 자리(상태 줄)가 없으므로 항목으로 넣는다.
    pub(super) fn say_empty_state(&mut self, recent: &mut Vec<IndexItem>) {
        let cfg = &self.runtime_config.say;
        if !cfg.enabled || !cfg.card {
            return;
        }
        if !self.say_card.loaded {
            self.say_card.loaded = true;
            let store = self.say_store.clone();
            let now = say::now_ms();
            self.say_card.card = say::load_card(&store, now);
            if self.say_card.card.is_some() {
                // 첫 화면을 그리는 길에서 디스크 쓰기를 뺀다 — 잃어도 노출 수 하나다
                std::thread::spawn(move || {
                    if let Err(e) = say::record_card_seen(&store, now) {
                        tracing::warn!("입버릇 노출 기록 실패: {e}");
                    }
                });
            }
        }
        if recent.is_empty() {
            if let Some(card) = &self.say_card.card {
                recent.push(IndexItem {
                    name: card.line(self.say_card.revealed, self.use_emoji),
                    path: "Enter: :say 연습".to_string(),
                    kind: ItemKind::SystemCommand,
                    source: Source::Plugin,
                    icon: String::new(),
                    keywords: format!("{}:say ", command::SAY_SEED_MARKER),
                    icon_path: None,
                });
            }
        }
    }

    /// 상태 줄에 보일 카드 — (왼쪽 문구, 오른쪽 안내). 빈 입력이고 알림이 없을 때만.
    pub(super) fn say_status_line(&self) -> Option<(String, &'static str)> {
        if !self.query.trim().is_empty() || self.status_message.is_some() {
            return None;
        }
        let card = self.say_card.card.as_ref()?;
        // 최근 항목이 없어 카드가 항목으로 들어갔으면 상태 줄에 또 보이지 않는다
        let seed = format!("{}:say ", command::SAY_SEED_MARKER);
        if self.results.iter().any(|r| r.item.keywords == seed) {
            return None;
        }
        let hint = if card.recall && !self.say_card.revealed {
            "Tab: 영어 보기"
        } else {
            ":say 연습"
        };
        Some((card.line(self.say_card.revealed, self.use_emoji), hint))
    }

    /// 빈 입력에서 Tab — 떠올리기 단계 카드의 영어를 연다
    pub(super) fn handle_say_card_key(&mut self, key: &keyboard::Key) -> Option<Task<Message>> {
        let is_tab = matches!(key, keyboard::Key::Named(keyboard::key::Named::Tab));
        let hidden = self
            .say_card
            .card
            .as_ref()
            .is_some_and(|c| c.recall && !self.say_card.revealed);
        if !is_tab || !hidden || !self.query.trim().is_empty() {
            return None;
        }
        self.say_card.revealed = true;
        // 최근 항목이 없어 카드가 항목으로 들어간 경우 다시 그린다
        Some(self.perform_search())
    }
}
