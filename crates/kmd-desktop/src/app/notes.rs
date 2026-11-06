//! `:m` 메모 — 판단은 `kmd_core::notes`, 여기는 화면·단축키 (docs/23)

use super::*;

use kmd_core::notes::{self, EditStart};

/// 한글 입력 중에는 Ctrl+E 가 `ㄷ` 으로 온다 — 메모는 한글로 쓰다 누르는 일이 많다
fn is_letter(c: &str, ascii: &str, jamo: &str) -> bool {
    c.eq_ignore_ascii_case(ascii) || c == jamo
}

impl App {
    pub(super) fn handle_notes_query(&mut self, query: &str) {
        let items = self.notes.results(query, self.use_emoji);
        self.apply_contains_items(items);
    }

    pub(super) fn commit_note_command(&mut self, target: Option<String>) -> Task<Message> {
        let trimmed = self.query.trim();
        let normalized = kmd_core::query_prefix::normalize_slash_command(trimmed);
        let query = normalized.as_deref().unwrap_or(trimmed).to_string();
        let applied = match self.notes.commit(&query, target.as_deref()) {
            Ok(a) => a,
            Err(e) => {
                self.status_message = Some(e);
                return Task::none();
            }
        };
        if let Some(text) = &applied.copy {
            if let Err(e) = kmd_core::clipboard::set_text(text) {
                self.status_message = Some(format!("클립보드 복사 실패: {e}"));
                return Task::none();
            }
        }
        if applied.close {
            return exit_launcher();
        }
        if let Some(next) = applied.next_query {
            self.query = next;
            self.selected = 0;
        }
        let task = self.perform_search();
        self.status_message = Some(applied.message);
        task
    }

    /// 메모 단축키. `:m` 이 아니면 None — 다른 단축키로 넘어간다
    pub(super) fn handle_notes_key(
        &mut self,
        key: &keyboard::Key,
        modifiers: &keyboard::Modifiers,
    ) -> Option<Task<Message>> {
        use keyboard::key::Named;
        if crate::query_prefix::prefix_of(self.query.trim()) != crate::query_prefix::Prefix::Notes {
            return None;
        }
        #[cfg(target_os = "macos")]
        let ctrl_like = modifiers.control() || modifiers.logo();
        #[cfg(not(target_os = "macos"))]
        let ctrl_like = modifiers.control();
        if !ctrl_like {
            return None;
        }
        match key {
            keyboard::Key::Named(Named::Delete) => self.note_delete_selected(),
            keyboard::Key::Character(c) if is_letter(c, "e", "ㄷ") => self.note_edit_selected(),
            // macOS는 ⌘D — `:scope` 와 같은 이유 (⌘⌫ 는 입력창이 함께 받는다)
            keyboard::Key::Character(c) if is_letter(c, "d", "ㅇ") => self.note_delete_selected(),
            keyboard::Key::Character(c) if is_letter(c, "z", "ㅋ") => {
                let result = self.notes.undo_delete()?;
                let task = self.perform_search();
                self.status_message = Some(result.unwrap_or_else(|e| e));
                Some(task)
            }
            _ => None,
        }
    }

    fn selected_note_id(&self) -> Option<String> {
        let item = &self.results.get(self.selected)?.item;
        notes::selected_id(item).map(str::to_string)
    }

    fn note_delete_selected(&mut self) -> Option<Task<Message>> {
        let id = self.selected_note_id()?;
        let result = self.notes.delete(&id);
        let task = self.perform_search();
        self.status_message = Some(result.unwrap_or_else(|e| e));
        Some(task)
    }

    fn note_edit_selected(&mut self) -> Option<Task<Message>> {
        let id = self.selected_note_id()?;
        match self.notes.begin_edit(&id) {
            Ok(EditStart::Inline(query)) => {
                self.query = query;
                self.selected = 0;
                let task = self.perform_search();
                // 입력창 커서는 바뀐 값을 따라가지 않는다 — 고치던 글 끝으로 옮긴다
                let cursor =
                    iced::widget::operation::move_cursor_to_end::<Message>(self.input_id.clone());
                Some(Task::batch([task, cursor]))
            }
            Ok(EditStart::External(path)) => {
                match kmd_core::action::open_with_system(&path.to_string_lossy()) {
                    kmd_core::action::ActionResult::Error(e) => {
                        self.status_message = Some(format!("{} 열기 실패: {e}", path.display()));
                        Some(Task::none())
                    }
                    _ => Some(exit_launcher()),
                }
            }
            Err(e) => {
                self.status_message = Some(e);
                Some(Task::none())
            }
        }
    }
}
