//! UI layout and rendering
//!
//! Layout structure:
//!   ┌──────────────────────────────────────┐
//!   │  » key·mander v0.2.0 · 5997 items    │  header (1 row)
//!   ├──────────────────────────────────────┤
//!   │  » Command                           │  input (3 rows, rounded)
//!   │  > query_                             │
//!   │                                      │
//!   ├──────────────────────────────────────┤
//!   │  Results               ╭─ Preview ──╮│
//!   │  ▸ item 1              │ Name: ...  ││  content (flex)
//!   │    item 2              │ Type: ...  ││
//!   │    ...                 ╰────────────╯│
//!   ├──────────────────────────────────────┤
//!   │  fuzzy  42 results  ↑↓ Tab Enter Esc│  status (1 row)
//!   └──────────────────────────────────────┘

pub mod input;
pub mod list;
pub mod preview;

/// Minimum terminal width to show the preview panel
const MIN_PREVIEW_WIDTH: u16 = 80;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use super::app::AppState;
use super::theme::Theme;

/// Render the entire TUI
pub fn render(frame: &mut Frame, state: &AppState, theme: &Theme) {
    let area = frame.area();

    // Main layout: header + input + content + status
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Length(3), // input bar
            Constraint::Min(3),    // content (list + optional preview)
            Constraint::Length(1), // status bar
        ])
        .split(area);

    render_header(frame, chunks[0], state, theme);
    input::render(frame, chunks[1], state, theme);

    // Content: list only, or list + preview
    if state.show_preview && area.width > MIN_PREVIEW_WIDTH {
        let preview_pct = state.preview_width_percent.clamp(10, 80);
        let list_pct = 100 - preview_pct;
        let content_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(list_pct),
                Constraint::Percentage(preview_pct),
            ])
            .split(chunks[2]);

        list::render(frame, content_chunks[0], state, theme);
        preview::render(frame, content_chunks[1], state, theme);
    } else {
        list::render(frame, chunks[2], state, theme);
    }

    render_status(frame, chunks[3], state, theme);
}

// ── Header ───────────────────────────────────────────────────────────────────

fn render_header(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let version = env!("CARGO_PKG_VERSION");

    let mut spans = vec![Span::styled(
        " ",
        ratatui::style::Style::default().bg(theme.mantle),
    )];
    spans.extend(theme.brand_spans());
    spans.push(Span::styled(
        " ",
        ratatui::style::Style::default().bg(theme.mantle),
    ));
    spans.push(Span::styled(
        format!("v{}", version),
        theme.header_dim_style(),
    ));
    spans.push(Span::styled("  \u{00B7}  ", theme.header_dim_style())); // ·
    spans.push(Span::styled(
        state.total_items.to_string(),
        theme.header_accent_style(),
    ));
    spans.push(Span::styled(" items indexed", theme.header_dim_style()));
    let line = Line::from(spans);

    let header = ratatui::widgets::Paragraph::new(line)
        .style(ratatui::style::Style::default().bg(theme.mantle));
    frame.render_widget(header, area);
}

// ── Status Bar ───────────────────────────────────────────────────────────────

fn render_status(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    // Show temporary status message (e.g. "Copied to clipboard") if present
    if let Some(msg) = &state.status_message {
        let line = Line::from(vec![Span::styled(
            format!(" {} ", msg),
            theme.status_count_style(),
        )]);
        let status = ratatui::widgets::Paragraph::new(line)
            .style(ratatui::style::Style::default().bg(theme.mantle));
        frame.render_widget(status, area);
        return;
    }

    // 메모 행을 고르면 쓸 수 있는 키를 보여 준다 (docs/23)
    if let Some(hint) = state
        .results
        .get(state.selected_index)
        .and_then(|r| kmd_core::notes::key_hint(&r.item, state.query.trim()))
    {
        let line = Line::from(vec![Span::styled(
            format!(" {hint} "),
            theme.status_count_style(),
        )]);
        let status = ratatui::widgets::Paragraph::new(line)
            .style(ratatui::style::Style::default().bg(theme.mantle));
        frame.render_widget(status, area);
        return;
    }

    let mode_label = state.search_mode_label();
    let result_count = state.results.len();

    let is_drill = state.drill_path.is_some();

    let mut spans = vec![
        // Mode badge
        Span::styled(format!(" {} ", mode_label), theme.status_mode_style()),
        Span::styled(" ", theme.status_style()),
        // Result count
        Span::styled(result_count.to_string(), theme.status_count_style()),
        Span::styled(" results ", theme.status_style()),
        Span::styled("\u{2502} ", theme.status_style()), // │
    ];

    // Navigation hints (context-aware)
    if is_drill {
        spans.extend(vec![
            Span::styled("\u{2191}\u{2193}", theme.status_hint_key_style()), // ↑↓
            Span::styled(" navigate ", theme.status_hint_desc_style()),
            Span::styled("Tab", theme.status_hint_key_style()),
            Span::styled("/", theme.status_hint_desc_style()),
            Span::styled("\u{2192}", theme.status_hint_key_style()), // →
            Span::styled(" open ", theme.status_hint_desc_style()),
            Span::styled("\u{2190}", theme.status_hint_key_style()), // ←
            Span::styled("/", theme.status_hint_desc_style()),
            Span::styled("Esc", theme.status_hint_key_style()),
            Span::styled(" back ", theme.status_hint_desc_style()),
            Span::styled("Enter", theme.status_hint_key_style()),
            Span::styled(" launch", theme.status_hint_desc_style()),
        ]);
    } else {
        spans.extend(vec![
            Span::styled("\u{2191}\u{2193}", theme.status_hint_key_style()),
            Span::styled(" navigate ", theme.status_hint_desc_style()),
            Span::styled("Tab", theme.status_hint_key_style()),
            Span::styled(" open folder ", theme.status_hint_desc_style()),
            Span::styled("Enter", theme.status_hint_key_style()),
            Span::styled(" launch ", theme.status_hint_desc_style()),
            Span::styled("Esc", theme.status_hint_key_style()),
            Span::styled(" quit ", theme.status_hint_desc_style()),
        ]);
    }

    // Portable mode indicator
    if state.is_portable {
        spans.push(Span::styled(
            "\u{2502} P ", // │ P
            ratatui::style::Style::default()
                .fg(theme.peach)
                .bg(theme.mantle)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ));
    }

    // Language indicator on the right
    let lang = if state.hangul_mode && state.hangul_auto {
        "\u{D55C}\u{2022}" // 한• (auto)
    } else if state.hangul_mode {
        "\u{D55C}" // 한
    } else {
        "EN"
    };
    spans.push(Span::styled(
        format!("\u{2502} {} ", lang), // │
        if state.hangul_mode {
            theme.status_count_style()
        } else {
            theme.status_style()
        },
    ));

    let line = Line::from(spans);
    let status = ratatui::widgets::Paragraph::new(line)
        .style(ratatui::style::Style::default().bg(theme.mantle));
    frame.render_widget(status, area);
}
