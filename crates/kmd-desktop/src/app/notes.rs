//! `:m` 메모 — 판단은 `kmd_core::notes`, 여기는 화면·단축키 (docs/23)

use super::*;

use kmd_core::notes::{self, EditStart};

/// 한글 입력 중에는 Ctrl+E 가 `ㄷ` 으로 온다 — 메모는 한글로 쓰다 누르는 일이 많다
fn is_letter(c: &str, ascii: &str, jamo: &str) -> bool {
    c.eq_ignore_ascii_case(ascii) || c == jamo
}

impl App {
    /// `kmd-desktop --memo` — `:m ` 입력으로 시작한다
    pub(super) fn start_memo(&mut self) -> Task<Message> {
        self.keep_input_source = true;
        self.query = notes::MEMO_SEED.to_string();
        let task = self.perform_search();
        let cursor = iced::widget::operation::move_cursor_to_end::<Message>(self.input_id.clone());
        Task::batch([task, cursor])
    }

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

    /// 메모 단축키. 메모 명령이 아니면 None — 다른 단축키로 넘어간다
    pub(super) fn handle_notes_key(
        &mut self,
        key: &keyboard::Key,
        modifiers: &keyboard::Modifiers,
    ) -> Option<Task<Message>> {
        use keyboard::key::Named;
        #[cfg(target_os = "macos")]
        let ctrl_like = modifiers.control() || modifiers.logo();
        #[cfg(not(target_os = "macos"))]
        let ctrl_like = modifiers.control();
        if !ctrl_like {
            return None;
        }
        let query = self.query.trim();
        if crate::query_prefix::prefix_of(query) == crate::query_prefix::Prefix::Clipboard {
            return match key {
                keyboard::Key::Character(c) if is_letter(c, "s", "ㄴ") => self.clip_to_note(),
                _ => None,
            };
        }
        if !notes::is_notes_query(query) {
            return None;
        }
        match key {
            keyboard::Key::Named(Named::Delete) => self.note_delete_selected(),
            keyboard::Key::Character(c) if is_letter(c, "e", "ㄷ") => self.note_edit_selected(),
            // macOS는 ⌘D — `:scope` 와 같은 이유 (⌘⌫ 는 입력창이 함께 받는다)
            keyboard::Key::Character(c) if is_letter(c, "d", "ㅇ") => self.note_delete_selected(),
            keyboard::Key::Character(c) if is_letter(c, "p", "ㅔ") => self.note_focus_selected(),
            keyboard::Key::Character(c) if is_letter(c, "z", "ㅋ") => {
                let result = self.notes.undo_last()?;
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
        let item = self.results.get(self.selected)?.item.clone();
        let result = self.notes.delete_item(&item)?;
        let task = self.perform_search();
        self.status_message = Some(result.unwrap_or_else(|e| e));
        Some(task)
    }

    /// 고른 할 일로 뽀모도로 시작
    fn note_focus_selected(&mut self) -> Option<Task<Message>> {
        let item = self.results.get(self.selected)?.item.clone();
        match self.notes.start_focus_item(&item)? {
            Ok(_) => Some(exit_launcher()),
            Err(e) => {
                self.status_message = Some(e);
                Some(Task::none())
            }
        }
    }

    /// `;` 에서 고른 클립보드 항목을 메모로 — 전체 글은 데몬에 있다 (N1.2)
    fn clip_to_note(&mut self) -> Option<Task<Message>> {
        let marker = self
            .results
            .get(self.selected)?
            .item
            .keywords
            .strip_prefix("kmd:clip:")?;
        let id = marker.split(':').next()?.parse::<u64>().ok()?;
        Some(Task::future(async move {
            let request = kmd_core::ipc::Request::ClipText { id };
            let result = tokio::task::spawn_blocking(move || {
                match kmd_core::ipc::send_request_result(&request) {
                    Ok(kmd_core::ipc::Response::ClipText { text }) => Ok(text),
                    Ok(kmd_core::ipc::Response::Error { message }) => Err(message),
                    Ok(other) => Err(format!("예기치 않은 데몬 응답: {other:?}")),
                    Err(e) => Err(format!("데몬에 연결할 수 없습니다 ({e})")),
                }
            })
            .await
            .unwrap_or_else(|e| Err(format!("클립보드 항목 읽기 실패: {e}")));
            Message::ClipTextForNote(result)
        }))
    }

    pub(super) fn handle_clip_text_for_note(&mut self, result: Result<String, String>) {
        self.status_message = Some(
            result
                .and_then(|text| self.notes.save_text(&text))
                .unwrap_or_else(|e| e),
        );
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
