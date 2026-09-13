//! UI 렌더링 — view, 결과 목록, 상세 패널, 치트시트

use super::*;
use super::{context_actions_for, estimate_title_max_chars, truncate_str};

impl App {
    /// 아이템 아이콘 — 브랜드(컬러 PNG 또는 모노 글리프) → 앱 PNG →
    /// 시스템 SVG(테마 틴트 + 컨테이너) → 이모지 텍스트 폴백.
    /// `emoji_font`는 텍스트 폴백일 때의 글자 크기.
    ///
    /// `general.brand_icons = "mono"`면 브랜드도 Simple Icons 단색 글리프를
    /// WebSearch 색(teal)으로 틴트해 시스템 아이콘과 같은 톤으로 그린다.
    fn item_icon_element<'a>(
        &'a self,
        item: &'a kmd_core::IndexItem,
        size: f32,
        emoji_font: f32,
    ) -> Element<'a, Message> {
        let mono = self
            .runtime_config
            .general
            .brand_icons
            .eq_ignore_ascii_case("mono");

        if mono {
            if let Some(handle) =
                crate::brand_icons::mono_brand_icon_for_item(item.kind, &item.keywords, &item.path)
                    .or_else(|| crate::brand_icons::mono_brand_icon_for_settings(&item.keywords))
            {
                return self.tinted_svg_icon(handle, self.theme.teal, size, 0.56);
            }
        } else if let Some(handle) =
            crate::brand_icons::brand_icon_for_item(item.kind, &item.keywords, &item.path)
                .or_else(|| crate::brand_icons::brand_icon_for_settings(&item.keywords))
        {
            return image(handle)
                .content_fit(iced::ContentFit::Fill)
                .width(size)
                .height(size)
                .into();
        }

        if let Some(handle) =
            crate::app_icons::cached_icon_for_item(item.kind, &item.path, item.icon_path.as_deref())
        {
            return image(handle)
                .content_fit(iced::ContentFit::Fill)
                .width(size)
                .height(size)
                .into();
        }

        if let Some((handle, role)) = crate::system_icons::system_icon_for(&item.icon, item.kind) {
            // 모노 모드에서 글리프 없는 웹 서비스(grok/daum/papago 등)도
            // 브랜드와 같은 teal로 맞춰 목록 톤을 유지한다.
            let color = if mono && item.kind == kmd_core::ItemKind::WebSearch {
                self.theme.teal
            } else {
                role.color(&self.theme)
            };
            return self.tinted_svg_icon(handle, color, size, 0.62);
        }

        container(text(&item.icon).size(emoji_font))
            .center_x(size)
            .center_y(size)
            .into()
    }

    /// 틴트 SVG + 12% 알파 라운드 컨테이너 렌더링. `scale`은 컨테이너 대비
    /// 글리프 비율 (스트로크형 0.62, 면형 브랜드 글리프는 0.56이 옵티컬 밸런스).
    fn tinted_svg_icon(
        &self,
        handle: iced::widget::svg::Handle,
        color: Color,
        size: f32,
        scale: f32,
    ) -> Element<'_, Message> {
        let box_bg = Color { a: 0.12, ..color };
        let glyph = iced::widget::svg(handle)
            .width(size * scale)
            .height(size * scale)
            .style(move |_: &_, _| iced::widget::svg::Style { color: Some(color) });
        container(glyph)
            .center_x(size)
            .center_y(size)
            .style(move |_: &_| container::Style {
                background: Some(Background::Color(box_bg)),
                border: Border {
                    radius: (size * 0.25).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    fn view_brand_mark(&self) -> Element<'_, Message> {
        let t = &self.theme;
        let u = self.ui;
        let copper = t.peach;

        // copper 색으로 아주 연한 원형 뱃지 배경
        let badge_bg = Color { a: 0.12, ..copper };
        let badge_border = Color { a: 0.35, ..copper };

        let badge_size = u.pill_height - 16.0;
        let icon = container(
            text("\u{00BB}") // »
                .size((u.font * 1.15).round())
                .color(copper),
        )
        .center_x(badge_size) // center_x(len)은 너비를 len으로 설정하며 가운데 정렬
        .center_y(badge_size)
        .style(move |_: &_| container::Style {
            background: Some(Background::Color(badge_bg)),
            border: Border {
                radius: (badge_size / 2.0).into(),
                width: 1.0,
                color: badge_border,
            },
            ..Default::default()
        });

        mouse_area(icon)
            .on_press(Message::BrandClicked)
            .on_right_press(Message::BrandRightClicked)
            .interaction(iced::mouse::Interaction::Pointer)
            .into()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let t = &self.theme;
        let has_results = !self.results.is_empty();
        let text_color = t.text;
        let overlay_color = t.overlay;
        let accent_color = t.accent;
        // Windows: 불투명 창(투명 합성 불가 환경에서 검게 렌더링) — 카드도 완전
        // 불투명으로 칠해 창 배경과 이음새 없이 이어지게 한다. (main.rs 참고)
        let bg = if cfg!(target_os = "windows") && !crate::window_transparent() {
            Color {
                a: 1.0,
                ..t.background
            }
        } else {
            t.background_with_opacity()
        };

        let divider_c = t.border;

        // ── 검색바 콘텐츠 ──
        let u = self.ui;
        let brand = self.view_brand_mark();

        let input = text_input("Search anything...", &self.query)
            .id(self.input_id.clone())
            .on_input(Message::QueryChanged)
            .width(Fill)
            .size(u.font)
            .padding(Padding::from([4, 8]))
            .style(move |_theme, status| {
                let is_focused = matches!(status, text_input::Status::Focused { .. });
                let ph_color = if is_focused {
                    Color {
                        a: overlay_color.a * 0.5,
                        ..overlay_color
                    }
                } else {
                    overlay_color
                };
                text_input::Style {
                    background: Background::Color(Color::TRANSPARENT),
                    border: Border::default(),
                    icon: overlay_color,
                    placeholder: ph_color,
                    value: text_color,
                    selection: Color {
                        a: 0.3,
                        ..accent_color
                    },
                }
            });

        let search_row = row![brand, input]
            .spacing(8)
            .align_y(iced::Alignment::Center)
            .padding(Padding::from([0, 12]));

        let search_bar = container(search_row).width(Fill).center_y(u.pill_height);

        // ── 카드 본체 (항상 동일한 트리 구조 — 포커스/IME 안정성 보장) ──
        let sep_h = if has_results { 1.0 } else { 0.0 };
        let h_sep = container(text(""))
            .width(Fill)
            .height(sep_h)
            .style(move |_: &_| container::Style {
                background: Some(Background::Color(divider_c)),
                ..Default::default()
            });

        let h_sep2 = container(text(""))
            .width(Fill)
            .height(sep_h)
            .style(move |_: &_| container::Style {
                background: Some(Background::Color(divider_c)),
                ..Default::default()
            });

        let vert_divider = container(text(""))
            .width(if has_results { 1 } else { 0 })
            .height(Fill)
            .style(move |_: &_| container::Style {
                background: Some(Background::Color(divider_c)),
                ..Default::default()
            });

        let is_keys_mode =
            crate::query_prefix::prefix_of(self.query.trim()) == crate::query_prefix::Prefix::Keys;

        let results_body: Element<'_, Message> = if is_keys_mode && has_results {
            container(scrollable(self.view_cheatsheet()).width(Fill).height(Fill))
                .width(Fill)
                .height(Length::Fill)
                .into()
        } else {
            let left_col = Column::new()
                .push(self.view_results_list())
                .push(h_sep2)
                .push(self.view_status_bar());

            row![
                container(left_col).width(Length::FillPortion(2)),
                vert_divider,
                container(self.view_detail_panel())
                    .width(Length::FillPortion(1))
                    .clip(true),
            ]
            .width(Fill)
            .height(if has_results {
                Length::Fill
            } else {
                Length::Fixed(0.0)
            })
            .into()
        };

        let card_col = Column::new()
            .push(search_bar)
            .push(h_sep)
            .push(results_body);

        // ── 카드 스타일 (pill_height 기반 일관된 라운드) ──
        let border_color = Color {
            a: if has_results { 0.28 } else { 0.52 },
            ..accent_color
        };
        let inner_line = Color { a: 0.16, ..t.peach };
        // Windows: 불투명 창에서는 바깥 링·간격(CARD_PAD)이 창 배경색 띠로
        // 노출되고 DPI 소수 배율에서 우측이 서브픽셀로 잘려 상하좌우가
        // 비대칭으로 보인다 — 링을 없애고(teal 테두리가 창 가장자리에 밀착)
        // 라운드는 DWM 네이티브 코너 클립(약 8px)과 일치시킨다.
        // macOS 투명 pill은 이중 테두리 디자인 유지.
        // pill 라운드(~23px)를 그리면 DWM 이 모서리를 잘라내 형태가 깨진다.
        // 투명 창이면 모서리 바깥을 우리가 투명으로 남길 수 있으므로 플랫폼과
        // 무관하게 같은 라운드를 그린다(= UI 통일). 불투명 창(Windows 폴백)에서는
        // 모서리 바깥이 창 배경색으로 채워지므로 DWM 코너 클립(8px)에 맞춘다.
        let (radius, ring_width) = if crate::window_transparent() {
            (u.pill_height / 2.0, 1.0)
        } else {
            (8.0, 0.0)
        };

        let card_surface = container(card_col)
            .width(Fill)
            .style(move |_: &_| container::Style {
                background: Some(Background::Color(bg)),
                border: Border {
                    radius: radius.into(),
                    width: 2.0,
                    color: border_color,
                },
                ..Default::default()
            });

        let card = container(card_surface)
            .width(Fill)
            .padding(CARD_PAD)
            .style(move |_: &_| container::Style {
                border: Border {
                    radius: (radius + CARD_PAD).into(),
                    width: ring_width,
                    color: inner_line,
                },
                ..Default::default()
            });

        // ── 안정적 트리: 항상 동일한 3개 자식 (card_stack, hint, bg_dismiss) ──
        // 카드 빈 영역 어디서든 창 드래그 — text_input·결과 행 등 자식이
        // capture 한 클릭은 이 mouse_area 까지 내려오지 않는다.
        let draggable_card = mouse_area(card).on_press(Message::StartWindowDrag);

        // 좌우 리사이즈 히트존은 stack 오버레이로 카드 위에 겹쳐 레이아웃 여백 0.
        // 히트존 5px = 바깥 링 1 + 간격 2 + teal 테두리 2 — 콘텐츠(좌우 12px 안쪽
        // 패딩)와 겹치지 않는다.
        let edge_w: f32 = 5.0;
        let left_edge = mouse_area(container(text("")).width(edge_w).height(Fill))
            .on_press(Message::StartWindowResize(window::Direction::West))
            .interaction(iced::mouse::Interaction::ResizingHorizontally);
        let right_edge = mouse_area(container(text("")).width(edge_w).height(Fill))
            .on_press(Message::StartWindowResize(window::Direction::East))
            .interaction(iced::mouse::Interaction::ResizingHorizontally);
        let resize_overlay = row![left_edge, Space::new().width(Fill), right_edge]
            .width(Fill)
            .height(Fill);

        let card_stack =
            stack![draggable_card, resize_overlay]
                .width(Fill)
                .height(if has_results {
                    Length::Fill
                } else {
                    Length::Shrink
                });

        // 고정 높이(u.hint_area_height) — 접힌 창 높이 계산과 일치해야 함 (app.rs)
        let hint_area: Element<'_, Message> = if !has_results && !self.query.trim().is_empty() {
            container(
                text("No results found")
                    .size(u.no_results_font)
                    .color(t.overlay)
                    .center(),
            )
            .width(Fill)
            .center_x(Fill)
            .center_y(u.hint_area_height)
            .into()
        } else {
            container(text("")).width(0).height(0).into()
        };

        let bg_dismiss = mouse_area(container(text("")).width(Fill).height(if has_results {
            Length::Fixed(0.0)
        } else {
            Length::Fill
        }))
        .on_press(Message::BackgroundClicked);

        let content = Column::new()
            .push(card_stack)
            .push(hint_area)
            .push(bg_dismiss);

        container(content).width(Fill).height(Fill).into()
    }

    pub(super) fn view_results_list(&self) -> Element<'_, Message> {
        let mut list = Column::new().spacing(0);
        for (i, result) in self.results.iter().enumerate() {
            list = list.push(self.view_result_row(i, result));
        }

        scrollable(container(list).width(Fill))
            .id(self.scrollable_id.clone())
            .height(Fill)
            .into()
    }

    pub(super) fn view_result_row<'a>(
        &'a self,
        index: usize,
        result: &'a kmd_core::SearchResult,
    ) -> Element<'a, Message> {
        let t = &self.theme;
        let is_selected = index == self.selected;
        let item = &result.item;

        let sel_color = if is_selected {
            t.accent
        } else {
            Color::TRANSPARENT
        };
        let u = self.ui;
        let left_bar = container(text(""))
            .width(4)
            .height(u.row_height - 8.0)
            .style(move |_: &_| container::Style {
                background: Some(Background::Color(sel_color)),
                border: Border {
                    radius: 2.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });

        let icon_size = u.result_icon;
        let icon_element = self.item_icon_element(item, icon_size, icon_size - 6.0);
        let title = text(&item.name)
            .size(u.title_font)
            .color(t.text)
            .wrapping(Wrapping::None);
        let subtitle = text(&item.path)
            .size(u.subtitle_font)
            .color(t.subtext)
            .wrapping(Wrapping::None);
        let info = iced::widget::column![title, subtitle]
            .spacing(2)
            .width(Fill)
            .clip(true);

        let kind_color = t.kind_color(item.kind);
        let kind_label = item.kind.to_string();
        let badge_bg = Color {
            a: 0.10,
            ..kind_color
        };
        let badge_border = Color {
            a: 0.20,
            ..kind_color
        };
        let badge = container(text(kind_label).size(u.badge_font).color(kind_color))
            .padding(Padding::from([2, 6]))
            .style(move |_: &_| container::Style {
                background: Some(Background::Color(badge_bg)),
                border: Border {
                    radius: 4.0.into(),
                    width: 1.0,
                    color: badge_border,
                },
                ..Default::default()
            });

        let bg = if is_selected {
            t.surface2
        } else {
            Color::TRANSPARENT
        };

        let row_content = row![left_bar, icon_element, info, badge]
            .spacing(10)
            .align_y(iced::Alignment::Center)
            .padding(Padding::from([4, 12]));

        mouse_area(
            container(row_content)
                .width(Fill)
                .height(u.row_height)
                .center_y(Fill)
                .style(move |_: &_| container::Style {
                    background: Some(Background::Color(bg)),
                    ..Default::default()
                }),
        )
        .on_press(Message::ResultClicked(index))
        .into()
    }

    /// :keys 치트시트 전체를 한 페이지 팝업 형태로 렌더링
    pub(super) fn view_cheatsheet(&self) -> Element<'_, Message> {
        let t = &self.theme;
        let u = self.ui;
        let font_sm = u.hint_font;
        let font_md = font_sm + 2.0;
        let font_lg = font_md + 2.0;
        let hint_c = t.overlay;
        let text_c = t.text;
        let accent = t.accent;
        let border_c = Color { a: 0.12, ..accent };
        let shortcut_w = Length::Fixed(u.pill_height * 3.5);

        // 섹션 인덱스 수집: 헤더(━━) 위치 목록
        let header_indices: Vec<usize> = self
            .results
            .iter()
            .enumerate()
            .filter(|(_, r)| r.item.name.starts_with("━━"))
            .map(|(i, _)| i)
            .collect();

        let section_count = header_indices.len();
        let mid = section_count.div_ceil(2);

        let mut left_col = Column::new().spacing(8).width(Fill);
        let mut right_col = Column::new().spacing(8).width(Fill);

        for (sec_idx, &hdr_pos) in header_indices.iter().enumerate() {
            let hdr = &self.results[hdr_pos].item;
            let title = hdr
                .name
                .trim_start_matches('━')
                .trim_end_matches('━')
                .trim();

            let mut col = Column::new().spacing(2);
            let title_row = row![
                text(&hdr.icon).size(font_md).color(accent),
                text(title)
                    .size(font_lg)
                    .color(accent)
                    .wrapping(Wrapping::None),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center);
            col = col.push(title_row);
            col = col.push(Space::new().height(4));

            // 다음 헤더까지의 엔트리
            let end = header_indices
                .get(sec_idx + 1)
                .copied()
                .unwrap_or(self.results.len());
            for entry_idx in (hdr_pos + 1)..end {
                let item = &self.results[entry_idx].item;
                let r = row![
                    container(
                        text(&item.name)
                            .size(font_md)
                            .color(text_c)
                            .wrapping(Wrapping::None)
                    )
                    .width(shortcut_w),
                    text(&item.path)
                        .size(font_sm)
                        .color(hint_c)
                        .wrapping(Wrapping::None),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center);
                col = col.push(r);
            }

            let card: Element<'_, Message> = container(col)
                .padding(Padding::from([10, 14]))
                .width(Fill)
                .style(move |_: &_| container::Style {
                    border: Border {
                        radius: 8.0.into(),
                        width: 1.0,
                        color: border_c,
                    },
                    ..Default::default()
                })
                .into();

            if sec_idx < mid {
                left_col = left_col.push(card);
            } else {
                right_col = right_col.push(card);
            }
        }

        let grid = row![left_col, right_col].spacing(10).width(Fill);

        container(grid)
            .padding(Padding::from([10, 14]))
            .width(Fill)
            .into()
    }

    pub(super) fn view_status_bar(&self) -> Element<'_, Message> {
        let t = &self.theme;
        let u = self.ui;
        // 알릴 일이 있으면 모드·개수 자리를 잠시 내준다 — 저장 실패 같은
        // 이벤트는 검색 결과가 아니므로 결과 목록에 끼워 넣지 않는다.
        let status_text = match &self.status_message {
            Some(msg) => msg.clone(),
            None => format!(
                "{}  \u{00B7}  {} results",
                self.search_mode.label(),
                self.results.len()
            ),
        };

        let left = text(status_text).size(u.status_font).color(t.overlay);
        let right = text("Esc to close").size(u.status_font).color(t.overlay);

        let bar = row![left, Space::new().width(Fill), right]
            .padding(Padding::from([4, 14]))
            .align_y(iced::Alignment::Center);

        container(bar)
            .width(Fill)
            .height(u.status_bar_height)
            .into()
    }

    pub(super) fn view_detail_panel(&self) -> Element<'_, Message> {
        let t = &self.theme;
        let u = self.ui;

        let Some(result) = self.results.get(self.selected) else {
            let hint_color = t.overlay;
            let shortcuts_col = iced::widget::column![
                text("Shortcuts").size(u.hint_font + 1.0).color(hint_color),
                container(text("")).height(8),
                text("Enter    Open").size(u.hint_font).color(hint_color),
                text("\u{2191}\u{2193}      Navigate")
                    .size(u.hint_font)
                    .color(hint_color),
                text("Esc       Close").size(u.hint_font).color(hint_color),
                text("Tab       Detail").size(u.hint_font).color(hint_color),
            ]
            .spacing(4);
            return container(
                container(shortcuts_col)
                    .width(Fill)
                    .padding(Padding::from([20, 16])),
            )
            .width(Fill)
            .height(Fill)
            .into();
        };
        let item = &result.item;

        // ── 아이콘 영역 (그라데이션 배경 위 큰 아이콘) ──
        let big_icon_size = u.detail_big_icon;
        let big_icon = self.item_icon_element(item, big_icon_size, big_icon_size - 10.0);

        let kind_color = t.kind_color(item.kind);
        let icon_glow = Color {
            a: 0.06,
            ..kind_color
        };
        let icon_area = container(
            container(big_icon)
                .width(Fill)
                .center_x(Fill)
                .padding(Padding::from([20, 0])),
        )
        .width(Fill)
        .style(move |_: &_| container::Style {
            background: Some(Background::Color(icon_glow)),
            ..Default::default()
        });

        // ── 이름 (Bold) ──
        // 상세 패널 폭 기준으로 타이틀 최대 폭/글자 수를 동적으로 계산한다.
        // 좌우 마진을 항상 확보해 패널 경계를 넘지 않도록 한다.
        let detail_panel_width = (self.window_width / 3.0).max(220.0);
        let title_side_margin = 24.0;
        let title_width = (detail_panel_width - (title_side_margin * 2.0)).max(120.0);
        let title_max_chars = estimate_title_max_chars(title_width, u.detail_name_font);
        let name_str = truncate_str(&item.name, title_max_chars);
        let name_label = container(
            text(name_str)
                .size(u.detail_name_font)
                .color(t.text)
                .width(Length::Fixed(title_width))
                .wrapping(Wrapping::WordOrGlyph),
        )
        .width(Fill)
        .clip(true)
        .center_x(Fill)
        .padding(Padding::from([8, 12]));

        // ── "Path" 라벨 + 경로 (왼쪽 정렬, 말줄임) ──
        let path_text = if item.path.chars().count() > 35 {
            let end: String = item
                .path
                .chars()
                .rev()
                .take(32)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("...{end}")
        } else {
            item.path.clone()
        };
        let path_section = container(
            iced::widget::column![
                text("Path").size(u.path_label_font).color(t.overlay),
                text(path_text)
                    .size(u.path_text_font)
                    .color(t.subtext)
                    .wrapping(Wrapping::None),
            ]
            .spacing(2),
        )
        .width(Fill)
        .padding(Padding::from([4, 16]));

        // ── 뱃지 행 ──
        let kind_label = item.kind.to_string();
        let badge_bg = Color {
            a: 0.12,
            ..kind_color
        };
        let badge_border_c = Color {
            a: 0.22,
            ..kind_color
        };
        let badge = container(text(kind_label).size(u.badge_font).color(kind_color))
            .padding(Padding::from([2, 8]))
            .style(move |_: &_| container::Style {
                background: Some(Background::Color(badge_bg)),
                border: Border {
                    radius: 10.0.into(),
                    width: 1.0,
                    color: badge_border_c,
                },
                ..Default::default()
            });
        let badge_row = container(badge).width(Fill).padding(Padding::from([2, 16]));

        // ── 구분선 (패딩 있는 정돈된 선) ──
        let sep_color = t.border;
        let divider = container(
            container(text(""))
                .width(Fill)
                .height(1)
                .style(move |_: &_| container::Style {
                    background: Some(Background::Color(sep_color)),
                    ..Default::default()
                }),
        )
        .padding(Padding::from([4, 14]));

        // ── 액션 목록 ──
        let actions = context_actions_for(item.kind);
        let mut action_list = Column::new().spacing(1);
        for (i, action) in actions.iter().enumerate() {
            let action_clone = *action;
            let label_text = action.label_for_kind(item.kind).to_string();
            let shortcut_text = action.shortcut().to_string();
            let is_primary = i == 0;

            let text_color = if is_primary { t.accent } else { t.text };
            let accent_c = t.accent;
            let surface_c = t.surface2;

            let icon_text = match action {
                ContextAction::Open => "\u{2197}",
                ContextAction::OpenAsAdmin => "\u{26A1}",
                ContextAction::OpenFolder => "\u{1F4C1}",
                ContextAction::CopyPath => "\u{1F4CB}",
                ContextAction::CopyName => "\u{270D}",
            };

            let icon_label = text(icon_text).size(u.action_icon_font);
            let label = text(label_text).size(u.action_label_font).color(text_color);
            let shortcut = text(shortcut_text)
                .size(u.action_shortcut_font)
                .color(t.overlay);

            let action_row = row![
                container(icon_label).width(22).center_x(22),
                label,
                Space::new().width(Fill),
                shortcut
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center)
            .padding(Padding::from([6, 14]));

            let action_btn =
                container(action_row)
                    .width(Fill)
                    .style(move |_: &_| container::Style {
                        background: if is_primary {
                            Some(Background::Color(Color {
                                a: 0.10,
                                ..accent_c
                            }))
                        } else {
                            Some(Background::Color(Color {
                                a: 0.0,
                                ..surface_c
                            }))
                        },
                        border: if is_primary {
                            Border {
                                radius: 6.0.into(),
                                width: 1.0,
                                color: Color {
                                    a: 0.18,
                                    ..accent_c
                                },
                            }
                        } else {
                            Border::default()
                        },
                        ..Default::default()
                    });

            let wrapped = if is_primary {
                container(
                    mouse_area(action_btn)
                        .on_press(Message::RunAction(action_clone))
                        .interaction(iced::mouse::Interaction::Pointer),
                )
                .padding(Padding::from([2, 10]))
            } else {
                container(
                    mouse_area(action_btn)
                        .on_press(Message::RunAction(action_clone))
                        .interaction(iced::mouse::Interaction::Pointer),
                )
                .padding(Padding::from([0, 10]))
            };

            action_list = action_list.push(wrapped);
        }

        let panel_content = iced::widget::column![
            icon_area,
            name_label,
            path_section,
            badge_row,
            divider,
            action_list,
        ]
        .spacing(0);

        container(panel_content).width(Fill).height(Fill).into()
    }
}
