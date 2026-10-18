//! 결과 목록 커밋 — 서명 비교로 불필요한 갱신을 거르고, 문자열/결과 변환 헬퍼

use super::*;

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

        self.results = new_results;
        self.last_results_signature = signature;
        if reset_selection {
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

/// IME 조합 중 생성되는 trailing 자모를 완화해 즉시 검색 반응성을 높인다.
///
/// 예: `하ㄴ` -> `하`, `한ㄱ` -> `한`
pub(super) fn relaxed_hangul_query(query: &str) -> Option<String> {
    if query.is_empty() {
        return None;
    }

    let mut chars: Vec<char> = query.chars().collect();
    let original_len = chars.len();
    while let Some(last) = chars.last() {
        if kmd_core::hangul::is_jamo(*last) {
            chars.pop();
        } else {
            break;
        }
    }

    if chars.is_empty() || chars.len() == original_len {
        return None;
    }

    Some(chars.into_iter().collect())
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
