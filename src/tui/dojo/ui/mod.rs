//! dojo 화면 — 공통 프레임·팔레트·위젯 (docs/10 §5, 와이어프레임 S1~S18)
//!
//! 모든 화면은 80×24 무대 위에 그린다. 터미널이 더 크면 무대를 가운데에 둔다 —
//! 넓어져도 시선 이동이 늘지 않게.

mod modes;
mod play;
mod screens;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::app::{App, Screen, SHAKE};
use super::play::Play;

pub const STAGE_W: u16 = 80;
pub const STAGE_H: u16 = 24;

// ── 팔레트 — 색의 의미는 고정 (docs/10 §5-1) ─────────────────────────────────
pub const ACCENT: Color = Color::Rgb(86, 210, 255);
pub const GREEN: Color = Color::Rgb(80, 250, 123);
pub const YELLOW: Color = Color::Rgb(255, 230, 120);
pub const PEACH: Color = Color::Rgb(255, 165, 96);
pub const RED: Color = Color::Rgb(255, 110, 140);
pub const GOLD: Color = Color::Rgb(245, 197, 66);
pub const TEXT: Color = Color::Rgb(220, 228, 255);
pub const SUB: Color = Color::Rgb(154, 163, 199);
pub const DIM: Color = Color::Rgb(90, 95, 128);
pub const BORDER: Color = Color::Rgb(74, 77, 104);
pub const SEL_BG: Color = Color::Rgb(68, 72, 98);
pub const MANTLE: Color = Color::Rgb(20, 20, 32);
pub const SAY: Color = Color::Rgb(232, 217, 168);
/// 쥐는 손
pub const HOLD: Color = Color::Rgb(122, 162, 255);
/// 움직이는 손
pub const ACT: Color = PEACH;

pub fn belt_color(id: &str) -> Color {
    match id {
        "white" => Color::Rgb(242, 242, 242),
        "yellow" => Color::Rgb(255, 216, 74),
        "orange" => Color::Rgb(255, 154, 60),
        "green" => Color::Rgb(76, 217, 100),
        "blue" => Color::Rgb(74, 163, 255),
        "purple" => Color::Rgb(183, 125, 255),
        "brown" => Color::Rgb(176, 122, 79),
        "black" => Color::Rgb(150, 150, 160),
        "dan" => GOLD,
        _ => Color::Rgb(138, 138, 138),
    }
}

pub fn fg(c: Color) -> Style {
    Style::new().fg(c)
}

pub fn bold(c: Color) -> Style {
    Style::new().fg(c).add_modifier(Modifier::BOLD)
}

pub fn sel() -> Style {
    Style::new()
        .bg(SEL_BG)
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}

pub fn span<'a>(text: impl Into<std::borrow::Cow<'a, str>>, style: Style) -> Span<'a> {
    Span::styled(text, style)
}

/// 표시폭 `width` 에 맞춰 자르거나 공백으로 채운다 (한글 2칸)
pub fn fit(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    let mut out = String::new();
    let mut w = UnicodeWidthStr::width(text);
    if w <= width {
        out.push_str(text);
    } else {
        // 넘치면 마지막 한 칸을 말줄임에 내준다
        w = 0;
        if width > 0 {
            for ch in text.chars() {
                let cw = ch.width().unwrap_or(0);
                if w + cw > width - 1 {
                    break;
                }
                out.push(ch);
                w += cw;
            }
            out.push('…');
            w += 1;
        }
    }
    out.push_str(&" ".repeat(width.saturating_sub(w)));
    out
}

pub fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// 테두리 상자
pub fn panel<'a>(title: Line<'a>) -> Block<'a> {
    Block::bordered().border_style(fg(BORDER)).title(title)
}

pub fn rounded<'a>(title: Line<'a>) -> Block<'a> {
    panel(title).border_type(BorderType::Rounded)
}

/// 사부 한 줄
pub fn sensei_line(text: &str) -> Line<'static> {
    Line::from(vec![
        span(" ", Style::new()),
        span(
            "사부",
            Style::new()
                .bg(GOLD)
                .fg(MANTLE)
                .add_modifier(Modifier::BOLD),
        ),
        span("  ", Style::new()),
        span(
            format!("\"{text}\""),
            Style::new().fg(SAY).add_modifier(Modifier::ITALIC),
        ),
    ])
}

/// 별 세 칸 — ★★☆
pub fn stars(n: u8) -> Vec<Span<'static>> {
    let n = n.min(3) as usize;
    vec![
        span("★".repeat(n), fg(GOLD)),
        span("☆".repeat(3 - n), fg(DIM)),
    ]
}

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    let Some(mut stage) = stage(area) else {
        screens::too_small(f, area);
        return;
    };
    // 실수 직후 무대가 한 칸 흔들린다 (docs/10 §4-2)
    if let Some(at) = app.shake {
        let e = at.elapsed();
        if e < SHAKE && area.width > STAGE_W && (e.as_millis() / 35).is_multiple_of(2) {
            stage.x += 1;
        }
    }
    match &app.screen {
        Screen::Precheck => screens::precheck(f, stage, app),
        Screen::Calib(run) => screens::calib(f, stage, app, run),
        Screen::Lobby => screens::lobby(f, stage, app),
        Screen::Map => screens::map(f, stage, app),
        Screen::Intro(id) => screens::intro(f, stage, app, id),
        Screen::Codex => screens::codex(f, stage, app),
        Screen::Records => screens::records(f, stage, app),
        Screen::Diagnose(d, scroll) => screens::diagnose(f, stage, app, d, *scroll),
        Screen::Card => screens::card(f, stage, app),
        Screen::Seals(v) => screens::seals(f, stage, app, v),
        Screen::Round(p) => {
            match p.inner() {
                Play::Summon(r) => play::round(f, stage, app, r),
                Play::Golf(r) => modes::golf(f, stage, app, r),
                Play::Spar(r) => modes::spar(f, stage, app, r),
                Play::Maze(r) => modes::maze(f, stage, app, r),
                Play::Mouse(r) => modes::mouse(f, stage, app, r),
                Play::Ime(r) => modes::ime(f, stage, app, r),
                Play::Quiz(r) => modes::quiz(f, stage, app, r),
                Play::Mixed(_) => {}
            }
            if let Play::Mixed(g) = p.as_ref() {
                modes::gauntlet(f, stage, app, g);
            }
            if let Some(sel) = app.pause {
                play::pause(f, stage, sel);
            }
        }
        Screen::Result(out) => play::result(f, stage, app, out),
    }
}

/// 80×24 무대 — 터미널이 작으면 `None`
fn stage(area: Rect) -> Option<Rect> {
    if area.width < STAGE_W || area.height < STAGE_H {
        return None;
    }
    Some(Rect {
        x: area.x + (area.width - STAGE_W) / 2,
        y: area.y + (area.height - STAGE_H) / 2,
        width: STAGE_W,
        height: STAGE_H,
    })
}

/// 공통 프레임 — 머리줄(브랜드·위치 / 오른쪽 상태) + 키 안내줄. 본문 영역(22행)을 돌려준다.
pub fn chrome(
    f: &mut Frame,
    stage: Rect,
    place: &str,
    right: Vec<Span<'static>>,
    hint: &str,
    app: &App,
) -> Rect {
    let [head, body, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(stage);
    let hd = Style::new().bg(MANTLE);
    let mut left = vec![
        span(" ", hd),
        span("道場", hd.fg(PEACH).add_modifier(Modifier::BOLD)),
        span(" ", hd),
        span("kmd dojo", hd.fg(TEXT).add_modifier(Modifier::BOLD)),
    ];
    if !place.is_empty() {
        left.push(span(format!("  {place}"), hd.fg(SUB)));
    }
    let lw: usize = left.iter().map(|s| width(&s.content)).sum();
    let rw: usize = right.iter().map(|s| width(&s.content)).sum();
    let gap = (STAGE_W as usize).saturating_sub(lw + rw + 1);
    left.push(span(" ".repeat(gap), hd));
    left.extend(
        right
            .into_iter()
            .map(|s| Span::styled(s.content, s.style.bg(MANTLE))),
    );
    left.push(span(" ", hd));
    f.render_widget(Paragraph::new(Line::from(left)).style(hd), head);

    // 알림이 있으면 키 안내 대신 보여 준다
    let foot_line = match &app.notice {
        Some((msg, _)) => Line::from(vec![span(" ", hd), span(msg.clone(), hd.fg(YELLOW))]),
        None => Line::from(vec![
            span(" ", hd),
            span(hint.to_string(), hd.fg(Color::Rgb(130, 140, 170))),
        ]),
    };
    f.render_widget(Paragraph::new(foot_line).style(hd), foot);
    body
}

/// 게이지 막대 — `█████░░░  70%`
pub fn gauge(pct: i32, w: usize, color: Color) -> Vec<Span<'static>> {
    let pct = pct.clamp(0, 100) as usize;
    let filled = w * pct / 100;
    vec![
        span("█".repeat(filled), fg(color)),
        span("░".repeat(w - filled), fg(DIM)),
        span(format!(" {pct:>3}%"), fg(SUB)),
    ]
}

/// 한 줄 사각형
pub fn row(area: Rect, y: u16) -> Rect {
    Rect {
        x: area.x,
        y: area.y + y,
        width: area.width,
        height: 1,
    }
}

/// 영역 안의 부분 사각형 (x, y, w, h) — 범위를 넘지 않게 자른다
pub fn sub(area: Rect, x: u16, y: u16, w: u16, h: u16) -> Rect {
    let x = x.min(area.width);
    let y = y.min(area.height);
    Rect {
        x: area.x + x,
        y: area.y + y,
        width: w.min(area.width - x),
        height: h.min(area.height - y),
    }
}

// ── 두 손 키보드 (S4) ─────────────────────────────────────────────────────────

const KEY_ROWS: [(&str, &str); 3] = [
    ("", "Q W E R T Y U I O P [ ]"),
    ("Caps", "A S D F G H J K L ; '"),
    ("Shft", "Z X C V B N M , . /"),
];

/// 키맵 키 이름을 키보드 칸 이름으로 — `CapsLock`→`Caps`, `LShift`→`Shft`
fn cell_name(key: &str) -> String {
    match key.to_ascii_lowercase().as_str() {
        "capslock" | "caps" => "Caps".into(),
        "lshift" | "shift" => "Shft".into(),
        "lalt" | "alt" => "LAlt".into(),
        "ralt" => "RAlt".into(),
        "space" | "spc" => "Space".into(),
        _ => key.to_ascii_uppercase(),
    }
}

/// 쥐는 손(파랑)·움직이는 손(주황)을 칠한 키보드
pub fn keyboard(hold: &[String], act: &[String]) -> Vec<Line<'static>> {
    let hold: Vec<String> = hold.iter().map(|k| cell_name(k)).collect();
    let act: Vec<String> = act.iter().map(|k| cell_name(k)).collect();
    let paint = |name: &str, label: String| -> Span<'static> {
        if hold.iter().any(|h| h == name) {
            span(format!("[{label}]"), bold(HOLD))
        } else if act.iter().any(|a| a == name) {
            span(format!("[{label}]"), bold(ACT))
        } else {
            span(format!(" {label} "), fg(DIM))
        }
    };
    let mut out = Vec::new();
    for (i, (lead, keys)) in KEY_ROWS.iter().enumerate() {
        let mut line = vec![span(" ".repeat(2 + i * 2), Style::new())];
        if lead.is_empty() {
            line.push(span("       ", Style::new()));
        } else {
            line.push(paint(lead, lead.to_string()));
            line.push(span(" ", Style::new()));
        }
        for k in keys.split(' ') {
            line.push(paint(k, k.to_string()));
            line.push(span(" ", Style::new()));
        }
        out.push(Line::from(line));
    }
    out.push(Line::from(vec![
        span("           ", Style::new()),
        paint("LAlt", "LAlt".into()),
        span("  ", Style::new()),
        paint("Space", "      Space      ".into()),
        span("  ", Style::new()),
        paint("RAlt", "RAlt".into()),
    ]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit은_한글_폭을_세고_넘치면_말줄임한다() {
        assert_eq!(width(&fit("한글", 6)), 6);
        assert_eq!(fit("abcdef", 4), "abc…");
        assert_eq!(width(&fit("가나다라", 5)), 5);
    }

    #[test]
    fn 키보드는_쥐는_손과_움직이는_손을_다르게_칠한다() {
        let lines = keyboard(&["CapsLock".into()], &["H".into(), "J".into()]);
        let caps = lines[1]
            .spans
            .iter()
            .find(|s| s.content == "[Caps]")
            .unwrap();
        let h = lines[1].spans.iter().find(|s| s.content == "[H]").unwrap();
        assert_eq!(caps.style.fg, Some(HOLD));
        assert_eq!(h.style.fg, Some(ACT));
    }
}
