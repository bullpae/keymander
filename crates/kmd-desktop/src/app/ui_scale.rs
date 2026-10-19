//! UI 치수 스케일 — font_size 하나로 모든 치수와 창 높이를 계산

use super::*;

/// font_size 기반 비례 계산으로 모든 UI 치수를 결정.
/// `font_size` 하나를 바꾸면 pill, row, 상태바, 아이콘, 서브 폰트가 모두 연동된다.
#[derive(Debug, Clone, Copy)]
pub struct UiScale {
    pub font: f32,
    pub visible_rows: usize,
    pub pill_height: f32,
    /// 검색바(pill) 높이 (레이아웃 문서화 겸 테스트 검증용)
    #[allow(dead_code)]
    pub search_bar_height: f32,
    pub row_height: f32,
    pub status_bar_height: f32,
    pub full_window_height: f32,
    /// 결과가 없을 때의 창 높이 — 검색바 pill + 카드 패딩만.
    /// 투명 합성이 안 되는 환경(가상 GPU, 소프트웨어 렌더러)에서 빈 영역이
    /// 검게 칠해지는 문제를 피하기 위해 결과 유무에 따라 창 자체를 리사이즈한다.
    pub collapsed_window_height: f32,
    /// "No results found" 힌트 영역의 고정 높이 (창 높이 계산과 일치해야 함).
    pub hint_area_height: f32,
    pub result_icon: f32,
    pub title_font: f32,
    pub subtitle_font: f32,
    pub badge_font: f32,
    pub status_font: f32,
    pub hint_font: f32,
    pub detail_name_font: f32,
    pub detail_big_icon: f32,
    pub path_label_font: f32,
    pub path_text_font: f32,
    pub action_icon_font: f32,
    pub action_label_font: f32,
    pub action_shortcut_font: f32,
    pub no_results_font: f32,
}

impl UiScale {
    pub fn new(raw_font: f32, raw_rows: usize) -> Self {
        let f = raw_font.clamp(12.0, 32.0);
        let visible_rows = raw_rows.clamp(4, 20);
        let pill_height = (f * 2.9).round(); // 16→46
        let search_bar_height = pill_height;
        let row_height = (f * 3.0).round(); // 16→48
        let status_bar_height = (f * 1.75).round(); // 16→28
                                                    // 카드 외곽 container 의 padding(CARD_PAD) 상하 (view.rs 의 card 구조와 일치).
                                                    // 테두리(teal 2px, peach 1px)는 iced 에서 bounds 안쪽에 그려지므로 높이에 더하지 않는다.
        let full_window_height = search_bar_height
            + CARD_PAD * 2.0
            + 1.0
            + (visible_rows as f32 * row_height)
            + 1.0
            + status_bar_height;
        let collapsed_window_height = search_bar_height + CARD_PAD * 2.0;
        let no_results_font = (f * 0.8125).round(); // 16→13
        let hint_area_height = (no_results_font * 1.4).ceil() + 16.0;
        Self {
            font: f,
            visible_rows,
            pill_height,
            search_bar_height,
            row_height,
            status_bar_height,
            full_window_height,
            collapsed_window_height,
            hint_area_height,
            result_icon: (f * 2.0).round(),            // 16→32
            title_font: (f * 1.0).round(),             // 16→16
            subtitle_font: (f * 0.8125).round(),       // 16→13
            badge_font: (f * 0.625).round(),           // 16→10
            status_font: (f * 0.6875).round(),         // 16→11
            hint_font: (f * 0.6875).round(),           // 16→11
            detail_name_font: (f * 0.9375).round(),    // 16→15
            detail_big_icon: (f * 3.25).round(),       // 16→52
            path_label_font: (f * 0.625).round(),      // 16→10
            path_text_font: (f * 0.6875).round(),      // 16→11
            action_icon_font: (f * 0.8125).round(),    // 16→13
            action_label_font: (f * 0.75).round(),     // 16→12
            action_shortcut_font: (f * 0.625).round(), // 16→10
            no_results_font,
        }
    }
}

/// 주어진 font_size, visible_rows로 전체 창 높이를 계산 (main.rs 에서 사용).
pub fn full_window_height(font_size: f32, visible_rows: usize) -> f32 {
    UiScale::new(font_size, visible_rows).full_window_height
}

/// 결과가 없을 때(검색바만)의 창 높이 (main.rs 에서 사용).
pub fn collapsed_window_height(font_size: f32, visible_rows: usize) -> f32 {
    UiScale::new(font_size, visible_rows).collapsed_window_height
}

/// 창을 처음 만들 때의 높이 (main.rs 에서 사용).
///
/// 높이를 고정하는 플랫폼(macOS)에서는 처음부터 full 로 띄운다 — 부팅 직후
/// collapsed 로 띄웠다가 늘리면 그 한 번의 리사이즈에서 왜곡이 그대로 보인다.
pub fn initial_window_height(font_size: f32, visible_rows: usize) -> f32 {
    let u = UiScale::new(font_size, visible_rows);
    if FIXED_WINDOW_HEIGHT || crate::window_transparent() {
        u.full_window_height
    } else {
        u.collapsed_window_height
    }
}
