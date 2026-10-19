//! `:scope` — 검색 범위 관리 화면과 추가·제거 후 즉시 인덱싱 (docs/18)
//!
//! 판단(포함 여부·레거시 전환·저장)은 `kmd_core::scope`가 하고, 여기서는 화면 갱신과
//! "추가하면 바로 검색된다"를 맡는다: 저장 → 데몬에 리빌드 요청(없으면 직접 빌드)
//! → 새 캐시로 엔진 교체 → 상태줄에 결과.

use std::path::PathBuf;

use super::*;

/// 리빌드 IPC 대기 상한 — 데몬은 파일명 인덱스만 만들고 응답한다(본문 인덱스는
/// 뒤에서). 실측 전체 리빌드 1초 안쪽이지만 큰 폴더를 넣으면 길어질 수 있다.
const REINDEX_TIMEOUT: Duration = Duration::from_secs(120);

/// 엔진 교체 뒤 상태줄에 남길 결과
#[derive(Debug, Clone)]
pub(super) enum ScopeReport {
    Added(PathBuf),
    /// 제거한 폴더와 코어의 안내 문구(상위 폴더가 계속 덮는 경우 포함)
    Removed(PathBuf, String),
}

impl App {
    /// `:scope [경로]` — 범위 목록 또는 경로 자동완성.
    pub(super) fn handle_scope_query(&mut self, query: &str) {
        let rest = kmd_core::scope::strip_command(query);
        let engine = &self.engine;
        let items = kmd_core::scope::results(
            &self.runtime_config.launcher,
            rest,
            &|p| engine.count_within(p),
            self.use_emoji,
        );
        self.apply_contains_items(items);
    }

    /// 폴더를 범위에 추가하고 바로 인덱싱한다 (`:scope` 추가 행, 경로 입력 행).
    pub(super) fn scope_add(&mut self, path: PathBuf) -> Task<Message> {
        match kmd_core::scope::add(&mut self.runtime_config, &path) {
            Ok((kmd_core::scope::AddOutcome::AlreadyCovered(_), msg)) => {
                self.status_message = Some(msg);
                Task::none()
            }
            Ok((_, _)) => self.start_scope_reindex(ScopeReport::Added(path)),
            Err(e) => {
                self.status_message = Some(e);
                Task::none()
            }
        }
    }

    /// 선택한 범위 폴더를 뺀다 (Ctrl+Del). `:scope` 목록 행이 아니면 None.
    pub(super) fn scope_remove_selected(&mut self) -> Option<Task<Message>> {
        let path = self
            .results
            .get(self.selected)?
            .item
            .keywords
            .strip_prefix(kmd_core::scope::SCOPE_FOLDER_MARKER)
            .map(PathBuf::from)?;
        Some(
            match kmd_core::scope::remove(&mut self.runtime_config, &path) {
                Ok(msg) => {
                    self.scope_undo = Some(path.clone());
                    self.start_scope_reindex(ScopeReport::Removed(path, msg))
                }
                Err(e) => {
                    self.status_message = Some(e);
                    Task::none()
                }
            },
        )
    }

    /// 방금 뺀 폴더를 되돌린다 (Ctrl+Z, `:scope` 화면에서만).
    pub(super) fn scope_undo(&mut self) -> Option<Task<Message>> {
        if kmd_core::query_prefix::prefix_of(self.query.trim())
            != kmd_core::query_prefix::QueryPrefix::Scope
        {
            return None;
        }
        let path = self.scope_undo.take()?;
        Some(self.scope_add(path))
    }

    /// `:scope` 전용 키 — Ctrl+Del 제거, Ctrl+Z 되돌리기, Tab 하위 폴더.
    /// 해당 없으면 None (일반 키 처리로 넘긴다).
    pub(super) fn handle_scope_key(
        &mut self,
        key: &keyboard::Key,
        modifiers: &keyboard::Modifiers,
    ) -> Option<Task<Message>> {
        use keyboard::key::Named;
        #[cfg(target_os = "macos")]
        let ctrl_like = modifiers.control() || modifiers.logo();
        #[cfg(not(target_os = "macos"))]
        let ctrl_like = modifiers.control();
        match key {
            keyboard::Key::Named(Named::Delete) if ctrl_like => self.scope_remove_selected(),
            // macOS는 ⌘D — ⌘⌫/⌫ 조합은 입력창이 함께 받아 `:scope` 글자를 지워 버린다
            // (구독은 위젯이 소비한 키도 받는다). ⌘D는 입력창에 바인딩이 없다.
            keyboard::Key::Character(c)
                if cfg!(target_os = "macos") && ctrl_like && c.eq_ignore_ascii_case("d") =>
            {
                self.scope_remove_selected()
            }
            keyboard::Key::Character(c) if ctrl_like && c.eq_ignore_ascii_case("z") => {
                self.scope_undo()
            }
            keyboard::Key::Named(Named::Tab) if !modifiers.shift() => self.scope_tab_complete(),
            _ => None,
        }
    }

    /// Tab — 선택한 하위 폴더/추가 후보로 입력을 채운다 (셸 자동완성처럼).
    pub(super) fn scope_tab_complete(&mut self) -> Option<Task<Message>> {
        let kw = &self.results.get(self.selected)?.item.keywords;
        let path = kw
            .strip_prefix(kmd_core::scope::SCOPE_BROWSE_MARKER)
            .or_else(|| kw.strip_prefix(kmd_core::scope::SCOPE_ADD_MARKER))?;
        self.query = kmd_core::scope::browse_seed(path);
        self.selected = 0;
        Some(self.perform_search())
    }

    /// 저장된 범위로 인덱스를 다시 만들고 엔진을 교체한다.
    pub(super) fn start_scope_reindex(&mut self, report: ScopeReport) -> Task<Message> {
        let label = match &report {
            ScopeReport::Added(p) | ScopeReport::Removed(p, _) => p.display().to_string(),
        };
        self.scope_report = Some(report);
        // 목록을 바로 다시 그린다 — 추가/제거가 즉시 보이고, 개수는 교체 후 채워진다
        if kmd_core::query_prefix::prefix_of(self.query.trim())
            == kmd_core::query_prefix::QueryPrefix::Scope
        {
            self.query = ":scope".to_string();
        }
        let task = self.perform_search();
        self.status_message = Some(format!("{label} 인덱싱 중…"));
        Task::batch([
            task,
            Task::future(async move {
                let res = tokio::task::spawn_blocking(reindex_blocking)
                    .await
                    .unwrap_or_else(|e| Err(format!("인덱싱 작업 실패: {e}")));
                Message::ScopeReindexed(res)
            }),
        ])
    }

    pub(super) fn handle_scope_reindexed(&mut self, res: Result<String, String>) -> Task<Message> {
        match res {
            Ok(msg) => {
                tracing::info!("scope reindex: {msg}");
                self.spawn_full_engine_load_task()
            }
            Err(e) => {
                self.scope_report = None;
                self.status_message = Some(format!("인덱싱 실패: {e}"));
                Task::none()
            }
        }
    }

    /// 엔진 교체가 끝난 뒤 남길 결과 문구 (handle_engine_ready가 부른다).
    pub(super) fn take_scope_report_message(&mut self) -> Option<String> {
        Some(match self.scope_report.take()? {
            ScopeReport::Added(p) => {
                kmd_core::scope::added_message(&p, self.engine.count_within(&p))
            }
            ScopeReport::Removed(_, msg) => {
                format!("{msg} · {} 되돌리기", kmd_core::scope::UNDO_KEY_LABEL)
            }
        })
    }
}

fn reindex_blocking() -> Result<String, String> {
    kmd_core::scope::rebuild_index_now(REINDEX_TIMEOUT).map(|via_daemon| {
        if via_daemon {
            "데몬 리빌드"
        } else {
            "직접 빌드"
        }
        .to_string()
    })
}
