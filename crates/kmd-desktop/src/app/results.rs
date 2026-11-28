//! 결과 목록 커밋 — 서명 비교로 불필요한 갱신을 거르고, 문자열/결과 변환 헬퍼

use super::*;

/// 자모로 끝난 질의를 초성 질의로 보기까지 기다리는 시간 — 한글 조합은 키 간격이 이보다 짧다
const IME_SETTLE_MS: u64 = 300;

impl App {
    pub(super) fn apply_contains_items<I>(&mut self, items: I)
    where
        I: IntoIterator<Item = IndexItem>,
    {
        self.apply_contains_results(items_to_results(items));
    }

    pub(super) fn apply_contains_results(&mut self, results: Vec<kmd_core::SearchResult>) {
        self.commit_results(results, kmd_core::SearchMode::Contains, true);
    }

    pub(super) fn clear_results_state(&mut self, mode: kmd_core::SearchMode) {
        self.results.clear();
        self.last_results_signature = 0;
        self.search_mode = mode;
        self.selected = 0;
    }

    pub(super) fn should_limit_reorder_for_ime(&self) -> bool {
        self.query
            .chars()
            .rev()
            .find(|c| !c.is_whitespace())
            .is_some_and(kmd_core::hangul::is_jamo)
            && self.ime_settled_query.as_deref() != Some(self.query.as_str())
    }

    /// 자모 꼬리 질의가 잠시 그대로면 검색한다 — 조합 중이었다면 그 사이 글자가 바뀐다
    pub(super) fn schedule_ime_settle(query: String) -> Task<Message> {
        Task::future(async move {
            tokio::time::sleep(std::time::Duration::from_millis(IME_SETTLE_MS)).await;
            Message::ImeSettled(query)
        })
    }

    pub(super) fn handle_ime_settled(&mut self, query: String) -> Task<Message> {
        if query != self.query {
            return Task::none();
        }
        self.ime_settled_query = Some(query);
        self.perform_search()
    }

    fn kind_code(kind: ItemKind) -> u8 {
        match kind {
            ItemKind::App => 1,
            ItemKind::Directory => 2,
            ItemKind::File => 3,
            ItemKind::Executable => 4,
            ItemKind::SystemCommand => 5,
            ItemKind::WebSearch => 6,
            ItemKind::Calculator => 7,
            ItemKind::Emoji => 8,
            ItemKind::Shell => 9,
        }
    }

    fn results_signature(results: &[kmd_core::SearchResult]) -> u64 {
        let mut hasher = DefaultHasher::new();
        results.len().hash(&mut hasher);
        for r in results.iter().take(64) {
            Self::kind_code(r.item.kind).hash(&mut hasher);
            r.item.name.hash(&mut hasher);
            r.item.path.hash(&mut hasher);
        }
        hasher.finish()
    }

    pub(super) fn commit_results(
        &mut self,
        new_results: Vec<kmd_core::SearchResult>,
        mode: kmd_core::SearchMode,
        reset_selection: bool,
    ) {
        let signature = Self::results_signature(&new_results);
        self.search_mode = mode;

        if signature == self.last_results_signature {
            return;
        }

        // IME 조합 중에는 결과 재정렬을 잠시 제한해 조합 안정성을 우선한다.
        if self.should_limit_reorder_for_ime() && !self.results.is_empty() {
            return;
        }

        // 같은 질의의 재검색(엔진 교체·파일 색인 갱신)이면 고른 항목을 지킨다 — 예전엔 ↓ 로 고른 뒤
        // Enter 직전에 뒤에서 결과가 바뀌면 첫 항목이 실행됐다
        let keep = (self.committed_query == self.query)
            .then(|| self.results.get(self.selected))
            .flatten()
            .map(|r| (r.item.kind, r.item.path.clone(), r.item.name.clone()));
        self.committed_query.clone_from(&self.query);

        self.results = new_results;
        self.last_results_signature = signature;
        let kept = keep.and_then(|(kind, path, name)| {
            self.results
                .iter()
                .position(|r| r.item.kind == kind && r.item.path == path && r.item.name == name)
        });
        if let Some(i) = kept {
            self.selected = i;
        } else if reset_selection {
            self.selected = 0;
        } else if self.selected >= self.results.len() {
            self.selected = self.results.len().saturating_sub(1);
        }
    }
}

/// UTF-8 안전한 문자열 truncation (한글/CJK 안전)
pub(super) fn truncate_str(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{truncated}...")
    }
}

/// 상세 타이틀 영역 픽셀 폭과 폰트 크기로 안전한 최대 글자 수를 추정한다.
///
/// 한글/전각 문자를 고려해 보수적인 폭 계수(0.95em)를 사용한다.
pub(super) fn estimate_title_max_chars(width_px: f32, font_size: f32) -> usize {
    let em = (font_size * 0.95).max(1.0);
    ((width_px / em).floor() as usize).clamp(8, 64)
}

pub(super) fn items_to_results(
    items: impl IntoIterator<Item = kmd_core::IndexItem>,
) -> Vec<kmd_core::SearchResult> {
    items
        .into_iter()
        .map(|item| kmd_core::SearchResult {
            item,
            score: SCORE_PLUGIN,
        })
        .collect()
}
