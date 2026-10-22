//! 플레이 화면 — 소환술 라운드(S12)·보스전 게이지(S13)·결과(S7)·일시정지(S18)

use std::time::Instant;

use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::*;
use crate::tui::dojo::action::Action;
use crate::tui::dojo::app::{row_mark, App, Outcome};
use crate::tui::dojo::summon::{Round, VISIBLE};
use kmd_core::ItemKind;

fn kind_tag(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::App => "App",
        ItemKind::Directory => "Dir",
        ItemKind::WebSearch => "Web",
        _ => "File",
    }
}

fn gauge(pct: i32, w: usize, color: Color) -> Vec<Span<'static>> {
    let pct = pct.clamp(0, 100) as usize;
    let filled = w * pct / 100;
    vec![
        span("█".repeat(filled), fg(color)),
        span("░".repeat(w - filled), fg(DIM)),
        span(format!(" {pct:>3}%"), fg(SUB)),
    ]
}

pub fn round(f: &mut Frame, stage: Rect, app: &App, round: &Round) {
    let Some((_, belt, lesson)) = app.content.lesson(&round.lesson_id) else {
        return;
    };
    let place = if lesson.is_boss() {
        "관문"
    } else {
        "소환술"
    };
    let right = vec![
        span("■ ", fg(belt_color(&belt.id))),
        span(format!("{} · {}", belt.name, lesson.title), fg(SUB)),
    ];
    let body = chrome(
        f,
        stage,
        place,
        right,
        "Enter 소환   ↑↓ 고르기   Tab 힌트 (별 -1)   Esc 일시정지",
        app,
    );

    // 머리 — 의뢰 번호 · Par · 타수 (보스는 HP·기·시간)
    let n = round.requests.len();
    let mut head = vec![
        span(format!("  의뢰 {} / {n}", round.idx + 1), bold(TEXT)),
        span("        Par ", fg(SUB)),
        span(round.par().to_string(), bold(TEXT)),
        span("     타수 ", fg(SUB)),
        span(
            round.strokes.to_string(),
            bold(if round.strokes > round.par() {
                RED
            } else {
                YELLOW
            }),
        ),
    ];
    if let Some(left) = round.remaining_time(Instant::now()) {
        let s = left.as_secs();
        head.push(span(
            format!("     남은 시간 {}:{:02}", s / 60, s % 60),
            bold(if s < 20 { RED } else { TEXT }),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(head)), row(body, 0));

    let mut y = 1;
    if let Some(b) = &round.boss {
        let mut hp = vec![
            span(format!("  {}  ", lesson.title), bold(RED)),
            span("보스 ", fg(SUB)),
        ];
        hp.extend(gauge(b.hp, 20, RED));
        hp.push(span("   기 ", fg(SUB)));
        hp.extend(gauge(b.ki, 14, GREEN));
        f.render_widget(Paragraph::new(Line::from(hp)), row(body, 1));
        y = 2;
    }

    // 의뢰
    let req = round.request();
    f.render_widget(
        Paragraph::new(Line::from(span(format!("  \"{}\"", req.ask), fg(SAY)))).block(rounded(
            Line::from(span(format!(" 의뢰 {} / {n} ", round.idx + 1), bold(PEACH))),
        )),
        sub(body, 1, y, 78, 3),
    );

    // 검색창 + 결과
    let border = match round.flash {
        Some((true, _)) => GREEN,
        Some((false, _)) => RED,
        None => BORDER,
    };
    let mut results: Vec<Line> = Vec::with_capacity(VISIBLE);
    for (i, item) in round.results.iter().enumerate() {
        let text = format!(
            " {} {}  {}  {}",
            if i == round.selected { "▸" } else { " " },
            fit(&item.name, 30),
            fit(kind_tag(item.kind), 4),
            fit(&item.path, 36)
        );
        results.push(Line::from(span(
            text,
            if i == round.selected { sel() } else { fg(TEXT) },
        )));
    }
    if round.results.is_empty() {
        let msg = if round.query.is_empty() {
            "  몇 글자를 쳐라."
        } else {
            "  결과 없음 — 지우고 다시."
        };
        results.push(Line::from(span(msg, fg(DIM))));
    }
    let title = Line::from(vec![
        span(" ❯ ", bold(ACCENT)),
        span(format!("{} ", round.query), bold(TEXT)),
    ]);
    let search_area = sub(body, 1, y + 3, 78, VISIBLE as u16 + 2);
    f.render_widget(
        Paragraph::new(results).block(rounded(title).border_style(fg(border))),
        search_area,
    );
    // IME 조합 글자가 입력 위치에 보이도록 실제 커서를 검색창에 둔다 (docs/10 §5-1)
    let cursor_x = search_area.x + 4 + width(&round.query) as u16;
    f.set_cursor_position((cursor_x.min(search_area.right() - 2), search_area.y));

    // 아래 — 힌트·접두 비급·배너·사부
    let mut y2 = y + 3 + VISIBLE as u16 + 2;
    if round.hint_shown {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                span("  힌트 · 사부의 수  ", fg(SUB)),
                span(round.sage().display(), bold(GOLD)),
            ])),
            row(body, y2),
        );
        y2 += 1;
    }
    f.render_widget(
        Paragraph::new(Line::from(vec![
            span("  접두 비급  ", fg(SUB)),
            span(".pdf", bold(ACT)),
            span(" 확장자   ", fg(SUB)),
            span("@gh", bold(ACT)),
            span(" 웹   ", fg(SUB)),
            span("!", bold(ACT)),
            span(" 셸", fg(SUB)),
        ])),
        row(body, y2),
    );
    if round.hangul_warn {
        f.render_widget(
            Paragraph::new(Line::from(span(
                format!(
                    "  ! 한글이 들어왔다 — 영문으로: {}",
                    app.bindings.label(Action::ToggleHangul)
                ),
                bold(YELLOW),
            ))),
            row(body, y2 + 1),
        );
    }
    f.render_widget(
        Paragraph::new(sensei_line(&app.sensei_line)),
        row(body, body.height - 1),
    );
}

pub fn pause(f: &mut Frame, stage: Rect, sel: usize) {
    let area = Rect {
        x: stage.x + 21,
        y: stage.y + 6,
        width: 38,
        height: 11,
    };
    f.render_widget(Clear, area);
    let item = |i: usize, label: &str, key: &str| {
        let text = format!(
            "  {} {}{}",
            if i == sel { "▶" } else { " " },
            fit(label, 18),
            fit(key, 10)
        );
        Line::from(span(text, if i == sel { sel_style() } else { fg(TEXT) }))
    };
    let lines = vec![
        Line::from(""),
        Line::from(span("숨 고르기", bold(TEXT))).alignment(Alignment::Center),
        Line::from(""),
        item(0, "계속", "Enter"),
        item(1, "처음부터", "R"),
        item(2, "그만두기", "Esc"),
        Line::from(""),
        Line::from(span("  기록되지 않는다 · 별 변화 없음", fg(DIM))),
    ];
    f.render_widget(
        Paragraph::new(lines).block(rounded(Line::from(span(" 일시정지 ", bold(ACCENT))))),
        area,
    );
}

fn sel_style() -> Style {
    sel()
}

pub fn result(f: &mut Frame, stage: Rect, app: &App, out: &Outcome) {
    let Some((_, belt, _)) = app.content.lesson(&out.lesson_id) else {
        return;
    };
    let right = vec![
        span("■ ", fg(belt_color(&belt.id))),
        span(format!("{} · {}", belt.name, out.title), fg(SUB)),
    ];
    let body = chrome(f, stage, "결과", right, "Enter 지도로   R 다시", app);

    let (headline, color) = match (out.cleared, out.is_boss) {
        (true, true) => ("격  파", GOLD),
        (true, false) => ("완  주", GREEN),
        (false, _) => ("패  배", RED),
    };
    let mut top = vec![span(headline, bold(color)), span("    ", Style::new())];
    top.extend(stars(out.stars));
    top.push(span(format!("    점수 {}", out.score), bold(TEXT)));
    if out.new_best {
        top.push(span("   ◆ 최고 기록", bold(GOLD)));
    }
    f.render_widget(
        Paragraph::new(Line::from(top)).alignment(Alignment::Center),
        row(body, 1),
    );

    // 스코어카드
    let mut lines = vec![Line::from(span(
        format!(
            " {} {} {} {} {}",
            fit("의뢰", 32),
            fit("내 수", 6),
            fit("Par", 5),
            fit("사부의 수", 18),
            "결과"
        ),
        fg(SUB),
    ))];
    for r in &out.rows {
        let mine = r.strokes.map_or("—".to_string(), |s| s.to_string());
        let mut spans = vec![
            span(format!(" {} ", fit(&r.ask, 32)), fg(TEXT)),
            span(format!("{} ", fit(&mine, 6)), bold(TEXT)),
            span(format!("{} ", fit(&r.par.to_string(), 5)), fg(SUB)),
            span(format!("{} ", fit(&r.sage, 18)), fg(GOLD)),
        ];
        match row_mark(r) {
            Some(m) => {
                let c = match m.symbol() {
                    "◎" => GOLD,
                    "◉" => GREEN,
                    "─" => SUB,
                    "▲" => YELLOW,
                    _ => RED,
                };
                spans.push(span(format!("{} {}", m.symbol(), m.name()), fg(c)));
                if r.strokes.is_some_and(|s| s <= r.sage_strokes) {
                    spans.push(span(" ◆", bold(GOLD)));
                }
            }
            None => spans.push(span("—", fg(DIM))),
        }
        lines.push(Line::from(spans));
    }
    let total: u32 = out.rows.iter().filter_map(|r| r.strokes).sum();
    let par: u32 = out
        .rows
        .iter()
        .filter(|r| r.strokes.is_some())
        .map(|r| r.par)
        .sum();
    lines.push(Line::from(""));
    lines.push(Line::from(span(
        format!(
            " 합계 {total}타 / Par {par}   실수 {}   힌트 {}   {}초",
            out.mistakes,
            out.hints,
            out.duration.as_secs()
        ),
        fg(SUB),
    )));
    if let Some(ki) = out.ki {
        lines.push(Line::from(span(format!(" 남은 기 {ki}%"), fg(SUB))));
    }
    let h = lines.len() as u16 + 2;
    f.render_widget(
        Paragraph::new(lines).block(panel(Line::from(span(" 스코어카드 ", bold(ACCENT))))),
        sub(body, 1, 3, 78, h),
    );

    let mut y = 3 + h + 1;
    if let Some(name) = &out.promoted {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                span("━━━━━  ", fg(DIM)),
                span("승  급", bold(GOLD)),
                span(format!("  ━━━━━   {name} 수련이 열렸다"), fg(TEXT)),
            ]))
            .alignment(Alignment::Center),
            row(body, y),
        );
        y += 1;
    }
    if let Some(id) = &out.new_scroll {
        if let Some(s) = app.content.scroll(id) {
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    span("  새 비급  ", bold(GOLD)),
                    span(format!("卷{}  ", s.id), fg(GOLD)),
                    span(s.title.clone(), bold(TEXT)),
                    span("  — 문 › 비급 도감에서 펼쳐 보아라", fg(DIM)),
                ])),
                row(body, y),
            );
        }
    }
    f.render_widget(
        Paragraph::new(sensei_line(&app.sensei_line)),
        row(body, body.height - 1),
    );
}
