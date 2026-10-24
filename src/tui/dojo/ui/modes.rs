//! 수련 화면 — 키 골프(S6·S7)·대련(S8)·미로·보이지 않는 마우스(S10)·보스 게이지(S13)

use std::time::{Duration, Instant};

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::*;
use crate::tui::dojo::action::Action;
use crate::tui::dojo::app::App;
use crate::tui::dojo::gauntlet::Gauntlet;
use crate::tui::dojo::golf::{self, GolfRound, Phase as GolfPhase};
use crate::tui::dojo::ime::{ImeRound, Phase as ImePhase};
use crate::tui::dojo::maze::{MazeRound, Phase as MazePhase, Tile};
use crate::tui::dojo::mouse::{
    Drill, LeakPhase, MouseRound, RatState, ARENA_H, ARENA_W, LIST_VIEW, RAT_W,
};
use crate::tui::dojo::play::BossState;
use crate::tui::dojo::quiz::{Phase as QuizPhase, QuizRound};
use crate::tui::dojo::score::Mark;
use crate::tui::dojo::spar::{Judge, Spar, Style as SparStyle, CARD_REF, LANES};

/// 라운드 공통 머리 — 띠 · 과 이름
fn round_chrome(f: &mut Frame, stage: Rect, app: &App, place: &str, id: &str, hint: &str) -> Rect {
    let right = match app.content.lesson(id) {
        Some((_, belt, lesson)) => vec![
            span("■ ", fg(belt_color(&belt.id))),
            span(format!("{} · {}", belt.name, lesson.title), fg(SUB)),
        ],
        None => Vec::new(),
    };
    chrome(f, stage, place, right, hint, app)
}

/// 화면 아래 한 줄 피드백 (1.6초)
fn feedback(f: &mut Frame, area: Rect, fb: &Option<(String, Instant)>, color: Color) {
    if let Some((msg, at)) = fb {
        if at.elapsed() < Duration::from_millis(1600) {
            f.render_widget(
                Paragraph::new(Line::from(span(format!("  {msg}"), bold(color)))),
                area,
            );
        }
    }
}

fn sensei_row(f: &mut Frame, body: Rect, app: &App) {
    f.render_widget(
        Paragraph::new(sensei_line(&app.sensei_line)),
        row(body, body.height - 1),
    );
}

/// 보스 게이지 한 줄 — HP · 기
fn boss_gauges(name: &str, b: &BossState, now: Instant) -> Line<'static> {
    let hurt = b.recently_hurt(now);
    let mut v = vec![
        span(
            format!("  ⚔ {name}  "),
            if hurt {
                Style::new().fg(MANTLE).bg(RED).add_modifier(Modifier::BOLD)
            } else {
                bold(RED)
            },
        ),
        span("보스 ", fg(SUB)),
    ];
    v.extend(gauge(b.hp, 16, RED));
    v.push(span("   기 ", fg(SUB)));
    v.extend(gauge(b.ki, 12, if b.ki < 30 { YELLOW } else { GREEN }));
    Line::from(v)
}

/// 보스 그림 — 맞은 직후엔 붉게, 내가 맞은 직후엔 웃는다
fn boss_art(id: &str, b: &BossState, now: Instant) -> Vec<Line<'static>> {
    let hurt = b.recently_hurt(now);
    let gloat = b.recently_struck(now);
    let art: Vec<String> = if id.starts_with("purple") {
        let eyes = if hurt {
            "x  x"
        } else if gloat {
            "^  ^"
        } else {
            "◉  ◉"
        };
        vec![
            "   (\\__/)".into(),
            format!("  ( {eyes} )"),
            "  ( >ww< )~".into(),
            "  /|    |\\".into(),
            "   ⌒    ⌒".into(),
        ]
    } else {
        let eyes = if hurt {
            "✕    ✕"
        } else if gloat {
            "◠    ◠"
        } else {
            "◣    ◢"
        };
        let mouth = if gloat { "▽▽" } else { "▼▼" };
        vec![
            "   /\\    /\\".into(),
            "  /  \\__/  \\".into(),
            format!(" │  {eyes}  │"),
            format!(" │    {mouth}    │"),
            "  \\ ← → ↑ ↓/".into(),
        ]
    };
    let color = if hurt { YELLOW } else { RED };
    art.into_iter()
        .map(|l| Line::from(span(l, bold(color))))
        .collect()
}

/// 큰 글자 한가운데 — 카운트다운 등
fn big_center(f: &mut Frame, area: Rect, text: &str, color: Color) {
    let y = area.y + area.height / 2;
    f.render_widget(
        Paragraph::new(Line::from(span(text.to_string(), bold(color))))
            .alignment(Alignment::Center),
        Rect {
            y,
            height: 1,
            ..area
        },
    );
}

// ── 키 골프 (S6 · S7) ─────────────────────────────────────────────────────────

pub fn golf(f: &mut Frame, stage: Rect, app: &App, r: &GolfRound) {
    let now = Instant::now();
    let hint = match r.phase {
        GolfPhase::Play => "Tab 힌트 (별 -1)   R 다시 치기   Esc 일시정지",
        GolfPhase::HoleOut { .. } => "Enter 다음 홀   Space 리플레이   Esc 일시정지",
    };
    let body = round_chrome(f, stage, app, "키 골프", &r.lesson_id, hint);
    let hole = r.hole();
    let sage_shown = matches!(r.phase, GolfPhase::HoleOut { .. }) || r.hint_shown;
    let mut head = vec![
        span(
            format!("  ■ {} · {}번 홀 ", r.course_name, r.idx + 1),
            bold(TEXT),
        ),
        span(fit(&hole.title, 16), fg(SUB)),
        span("  Par ", fg(SUB)),
        span(r.par().to_string(), bold(TEXT)),
        span("   타수 ", fg(SUB)),
        span(
            r.strokes.to_string(),
            bold(if r.strokes > r.par() { RED } else { YELLOW }),
        ),
        span("   사부의 수 ", fg(SUB)),
        span(
            if sage_shown {
                r.sage().strokes.to_string()
            } else {
                "?".into()
            },
            bold(GOLD),
        ),
    ];
    if r.results.is_empty() && r.idx == 0 {
        head.push(span(format!("   {}홀", r.holes.len()), fg(DIM)));
    }
    f.render_widget(Paragraph::new(Line::from(head)), row(body, 0));
    let mut y = 1;
    if let Some(b) = &r.boss {
        let name = app
            .content
            .lesson(&r.lesson_id)
            .map(|(_, _, l)| l.title.clone())
            .unwrap_or_default();
        f.render_widget(Paragraph::new(boss_gauges(&name, b, now)), row(body, 1));
        y = 2;
    }

    // 버퍼 — 리플레이 중이면 사부의 수를 따라 움직이는 유령 커서
    let replay = r.replay_buffer(now);
    let (buf, ghost) = match &replay {
        Some((b, _)) => (b, true),
        None => (&r.buf, false),
    };
    let top = r.view_top(buf);
    let selection = buf.selection();
    let target = hole.target;
    let inside =
        |range: Option<(usize, usize)>, at: usize| range.is_some_and(|(a, b)| a <= at && at < b);
    // 선형 위치 — 줄바꿈도 한 칸 (Buffer 의 선택·과녁 좌표계)
    let mut lin: usize = buf.lines[..top].iter().map(|l| l.len() + 1).sum();
    let mut lines = Vec::new();
    for (i, cells) in buf.lines.iter().enumerate().skip(top).take(golf::VIEW) {
        let mut v = vec![span(format!(" {:>2}  ", i + 1), fg(DIM))];
        let n = cells.len();
        let line_start = lin;
        lin += n + 1;
        for c in 0..=n {
            let at = line_start + c;
            let selected = inside(selection, at);
            let aimed = inside(target, at);
            let ch = cells
                .get(c)
                .map_or(" ".to_string(), |cell| cell.ch.to_string());
            let junk = cells.get(c).is_some_and(|cell| cell.junk);
            let is_cursor = buf.row == i && buf.col == c;
            let is_flag = hole.flag == Some((i, c));
            // 줄 끝의 줄바꿈이 선택·과녁에 들면 한 칸으로 보인다
            let shows_newline = c == n && (selected || aimed);
            let style = if is_cursor {
                Style::new()
                    .bg(if ghost { GREEN } else { ACCENT })
                    .fg(MANTLE)
                    .add_modifier(Modifier::BOLD)
            } else if is_flag {
                Style::new()
                    .bg(GOLD)
                    .fg(MANTLE)
                    .add_modifier(Modifier::BOLD)
            } else if selected && aimed {
                Style::new().bg(GREEN).fg(MANTLE)
            } else if selected {
                let s = Style::new().bg(HOLD).fg(MANTLE);
                if junk {
                    s.add_modifier(Modifier::CROSSED_OUT | Modifier::BOLD)
                } else {
                    s
                }
            } else if junk {
                Style::new()
                    .fg(RED)
                    .add_modifier(Modifier::CROSSED_OUT | Modifier::BOLD)
            } else if aimed {
                Style::new()
                    .fg(GOLD)
                    .add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
            } else {
                fg(TEXT)
            };
            if c == n && !is_cursor && !is_flag && !shows_newline {
                break;
            }
            v.push(span(ch, style));
        }
        if hole.flag.is_some_and(|(fr, _)| fr == i) {
            v.push(span("   ⚑", bold(GOLD)));
        }
        lines.push(Line::from(v));
    }
    let border = match r.flash {
        Some((true, _)) => GREEN,
        Some((false, _)) => RED,
        None => BORDER,
    };
    let title = if ghost {
        span(" 사부의 수 리플레이 ", bold(GREEN))
    } else {
        span(" 수련장.txt ", bold(ACCENT))
    };
    let box_h = golf::VIEW as u16 + 2;
    f.render_widget(
        Paragraph::new(lines).block(panel(Line::from(title)).border_style(fg(border))),
        sub(body, 1, y, 78, box_h),
    );
    let y = y + box_h;

    if let GolfPhase::HoleOut { .. } = r.phase {
        hole_out(f, sub(body, 1, y, 78, body.height - y - 1), app, r);
        return;
    }

    // 목표
    let goal = match (hole.flag, hole.target) {
        (_, Some((a, b))) => {
            let text: String = r
                .hole()
                .start
                .lines
                .iter()
                .map(|l| l.iter().map(|c| c.ch).collect::<String>())
                .collect::<Vec<_>>()
                .join("⏎")
                .chars()
                .skip(a)
                .take(b - a)
                .collect();
            let text = if width(&text) > 30 {
                fit(&text, 30)
            } else {
                text
            };
            format!("금빛 밑줄 「{text}」 을 정확히 감싸라")
        }
        (Some((fr, fc)), None) => {
            let ch = r.buf.lines[fr]
                .get(fc)
                .map_or("줄 끝".to_string(), |c| format!("'{}'", c.ch));
            format!("⚑ {}행 {} 로", fr + 1, ch)
        }
        (None, None) => format!("빨간 군더더기 {}글자를 지워라", r.buf.junk_left()),
    };
    let used: Vec<&str> = r
        .used
        .iter()
        .rev()
        .take(24)
        .rev()
        .map(|a| a.glyph())
        .collect();
    let left = r.par() as i64 - r.strokes as i64;
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![span("  목표  ", fg(SUB)), span(goal, bold(GOLD))]),
            Line::from(vec![
                span("  쓴 수 ", fg(SUB)),
                span(
                    if used.is_empty() {
                        "—".into()
                    } else {
                        used.join(" ")
                    },
                    fg(TEXT),
                ),
                span(
                    format!("   남은 Par {left}"),
                    fg(if left < 0 { RED } else { SUB }),
                ),
            ]),
        ]),
        sub(body, 0, y, 80, 2),
    );
    let mut y = y + 2;
    if r.hint_shown {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                span("  힌트 · 사부의 수  ", fg(SUB)),
                span(r.sage().display(), bold(GOLD)),
            ])),
            row(body, y),
        );
        y += 1;
    }

    // 허용된 수 — 사용자 배치의 키로
    let mut moves: Vec<Span> = vec![span("  허용 ", fg(SUB))];
    let mut selecting = false;
    for e in &r.allowed[r.idx] {
        // 선택 수는 줄이 넘치니 한 번만 — "Shift 를 더하면 선택"
        if e.action.unshifted().is_some() {
            selecting = true;
            continue;
        }
        let key = app.bindings.key(e.action);
        moves.push(span(key, bold(ACT)));
        moves.push(span(format!("{} ", e.action.glyph()), fg(SUB)));
        if e.seq.len() > 1 {
            moves.push(span("(2타) ", fg(DIM)));
        }
    }
    if selecting {
        moves.push(span("+Shift", bold(HOLD)));
        moves.push(span(" = 선택", fg(SUB)));
    }
    f.render_widget(Paragraph::new(Line::from(moves)), row(body, y));
    y += 1;
    if !r.unobservable.is_empty() {
        let names: Vec<&str> = r.unobservable.iter().map(|a| a.label()).collect();
        f.render_widget(
            Paragraph::new(Line::from(span(
                format!(
                    "  이 터미널에서 관측 불가: {} — Par 에서 뺐다",
                    names.join(", ")
                ),
                fg(DIM),
            ))),
            row(body, y),
        );
    }
    feedback(f, row(body, body.height - 2), &r.feedback, YELLOW);
    sensei_row(f, body, app);
}

/// 홀 아웃 오버레이 — 내 수 대 사부의 수, 코스 스코어카드
fn hole_out(f: &mut Frame, area: Rect, app: &App, r: &GolfRound) {
    let Some(res) = r.results.last() else {
        return;
    };
    let mark = Mark::of(res.strokes, res.par);
    let color = mark_color(mark);
    let diff = res.strokes as i64 - res.par as i64;
    let mut title = vec![
        span(format!(" {}번 홀 ", r.idx + 1), bold(ACCENT)),
        span(
            format!(" {} {} ", mark.symbol(), big_word(mark)),
            bold(color),
        ),
        span(format!("{} {:+} ", mark.name(), diff), fg(color)),
    ];
    if res.strokes <= res.sage {
        title.push(span(" ◆ 사부와 같은 수 ", bold(GOLD)));
    }
    let mine: Vec<&str> = r
        .my_moves
        .last()
        .map(|m| m.iter().map(|a| a.glyph()).collect())
        .unwrap_or_default();
    let mut lines = vec![
        Line::from(vec![
            span("  내 수      ", fg(SUB)),
            span(fit(&mine.join(" "), 44), fg(TEXT)),
            span(format!("{:>3}타", res.strokes), bold(TEXT)),
        ]),
        Line::from(vec![
            span("  사부의 수  ", fg(SUB)),
            span(fit(&r.sage().display(), 44), fg(GOLD)),
            span(format!("{:>3}타", res.sage), bold(GOLD)),
        ]),
    ];
    // 코스 스코어카드
    let n = r.holes.len();
    let mut h = vec![span("  홀   ", fg(SUB))];
    let mut p = vec![span("  Par  ", fg(SUB))];
    let mut s = vec![span("  타수 ", fg(SUB))];
    for i in 0..n {
        h.push(span(format!("{:>3}", i + 1), fg(DIM)));
        p.push(span(format!("{:>3}", r.pars[i]), fg(SUB)));
        match r.results.get(i) {
            Some(x) => s.push(span(
                format!("{:>3}", x.strokes),
                bold(mark_color(Mark::of(x.strokes, x.par))),
            )),
            None => s.push(span("  ·", fg(DIM))),
        }
    }
    let total: u32 = r.results.iter().map(|x| x.strokes).sum();
    let par: u32 = r.pars.iter().take(r.results.len()).sum();
    h.push(span("    합", fg(DIM)));
    p.push(span(format!("{par:>6}"), fg(SUB)));
    s.push(span(format!("{total:>6}"), bold(TEXT)));
    lines.push(Line::from(h));
    lines.push(Line::from(p));
    lines.push(Line::from(s));
    lines.push(Line::from(span(
        "  Space 사부의 수 리플레이 · Enter 다음 홀",
        fg(DIM),
    )));
    let _ = app;
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(rounded(Line::from(title)).border_style(fg(color))),
        area,
    );
}

fn big_word(m: Mark) -> &'static str {
    match m {
        Mark::Eagle => "E A G L E",
        Mark::Birdie => "B I R D I E",
        Mark::Par => "P A R",
        Mark::Bogey => "B O G E Y",
        Mark::Double => "D O U B L E",
    }
}

pub fn mark_color(m: Mark) -> Color {
    match m {
        Mark::Eagle => GOLD,
        Mark::Birdie => GREEN,
        Mark::Par => SUB,
        Mark::Bogey => YELLOW,
        Mark::Double => RED,
    }
}

// ── 대련 (S8) ─────────────────────────────────────────────────────────────────

fn big_arrow(a: Action) -> [&'static str; 5] {
    match a {
        Action::WordLeft => [
            "  ▄█  ▄█ ",
            " ███ ███ ",
            "◀██ ◀██  ",
            " ███ ███ ",
            "  ▀█  ▀█ ",
        ],
        Action::WordRight => [
            " █▄  █▄  ",
            " ███ ███ ",
            "  ██▶ ██▶",
            " ███ ███ ",
            " █▀  █▀  ",
        ],
        Action::PageUp => [
            "  ▲▲▲▲▲  ",
            " ███████ ",
            "  █████  ",
            "   ███   ",
            "  PgUp   ",
        ],
        Action::PageDown => [
            "  PgDn   ",
            "   ███   ",
            "  █████  ",
            " ███████ ",
            "  ▼▼▼▼▼  ",
        ],
        Action::Backspace => [
            "   ▄████ ",
            " ▄█ ╲╱ █ ",
            "◀██ ╱╲ █ ",
            " ▀█▄▄▄▄█ ",
            "   ⌫     ",
        ],
        Action::Delete => [
            " ████▄   ",
            " █ ╲╱ █▄ ",
            " █ ╱╲ ██▶",
            " █▄▄▄▄█▀ ",
            "     ⌦   ",
        ],
        Action::Up => [
            "    ▲    ",
            "   ███   ",
            "  █████  ",
            "   ███   ",
            "   ███   ",
        ],
        Action::Down => [
            "   ███   ",
            "   ███   ",
            "  █████  ",
            "   ███   ",
            "    ▼    ",
        ],
        Action::Left => [
            "   ▄█    ",
            " ▄██████ ",
            "◀███████ ",
            " ▀██████ ",
            "   ▀█    ",
        ],
        _ => [
            "    █▄   ",
            " ██████▄ ",
            " ███████▶",
            " ██████▀ ",
            "    █▀   ",
        ],
    }
}

fn lane_color(a: Action) -> Color {
    match a {
        Action::Left => PEACH,
        Action::Down => ACCENT,
        Action::Up => GREEN,
        Action::WordLeft | Action::WordRight => YELLOW,
        Action::PageUp | Action::PageDown => Color::Rgb(183, 125, 255),
        Action::Backspace | Action::Delete => RED,
        _ => Color::Rgb(255, 121, 198),
    }
}

pub fn spar(f: &mut Frame, stage: Rect, app: &App, s: &Spar) {
    match s.style {
        SparStyle::Card => spar_card(f, stage, app, s),
        SparStyle::Rain => spar_rain(f, stage, app, s),
    }
}

fn combo_span(combo: u32) -> Span<'static> {
    let c = match combo {
        0..=4 => SUB,
        5..=9 => YELLOW,
        10..=19 => PEACH,
        _ => GOLD,
    };
    span(format!("콤보 ×{combo}"), bold(c))
}

fn spar_card(f: &mut Frame, stage: Rect, app: &App, s: &Spar) {
    let now = Instant::now();
    let body = round_chrome(
        f,
        stage,
        app,
        "대련",
        &s.lesson_id,
        "결과 키로 받아쳐라   Esc 일시정지",
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            span(
                format!(
                    "  ■ 대련 · 카드 {} / {}",
                    (s.idx + 1).min(s.prompts.len()),
                    s.prompts.len()
                ),
                bold(TEXT),
            ),
            span("            ", Style::new()),
            combo_span(s.combo),
        ])),
        row(body, 0),
    );
    let Some(want) = s.prompt() else {
        return;
    };
    let border = match s.flash {
        Some((true, _)) => GREEN,
        Some((false, _)) => RED,
        None => BORDER,
    };
    let art: Vec<Line> = big_arrow(want)
        .iter()
        .map(|l| {
            Line::from(span(l.to_string(), bold(lane_color(want)))).alignment(Alignment::Center)
        })
        .collect();
    f.render_widget(
        Paragraph::new(art).block(rounded(Line::from("")).border_style(fg(border))),
        sub(body, 29, 2, 22, 7),
    );
    let guide = if s.guided() {
        Line::from(vec![
            span(
                app.bindings.trigger("nav").unwrap_or("nav").to_string(),
                bold(HOLD),
            ),
            span("  +  ", fg(DIM)),
            span(app.bindings.key(want), bold(ACT)),
        ])
    } else {
        Line::from(span("· · ·  보지 말고 느껴라  · · ·", fg(DIM)))
    };
    f.render_widget(
        Paragraph::new(guide.alignment(Alignment::Center)),
        row(body, 10),
    );

    // 반응시간 막대 — 기준 1.5초
    let t = now.duration_since(s.prompt_at);
    let pct = (t.as_millis() * 100 / CARD_REF.as_millis()).min(100) as usize;
    let w = 30;
    let fill = w * pct / 100;
    let color = if pct < 50 {
        GREEN
    } else if pct < 85 {
        YELLOW
    } else {
        RED
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            span("█".repeat(fill), fg(color)),
            span("░".repeat(w - fill), fg(DIM)),
            span(format!("  {:.1}초", t.as_secs_f32()), fg(SUB)),
        ]))
        .alignment(Alignment::Center),
        row(body, 12),
    );

    let mut recent = vec![span("     최근  ", fg(SUB))];
    for r in &s.recent {
        if r.ok {
            recent.push(span(
                format!("✓ {:.2}  ", r.time.unwrap_or_default().as_secs_f32()),
                fg(GREEN),
            ));
        } else {
            recent.push(span(
                format!("✗ {} 를 침  ", r.got.map_or("?", |a| a.label())),
                fg(RED),
            ));
        }
    }
    f.render_widget(Paragraph::new(Line::from(recent)), row(body, 15));
    let avg = s
        .avg_time()
        .map_or("—".into(), |d| format!("{:.2}초", d.as_secs_f32()));
    f.render_widget(
        Paragraph::new(Line::from(span(
            format!(
                "     평균 {avg}   최고 콤보 ×{}   정확도 {:.0}%",
                s.max_combo,
                s.accuracy() * 100.0
            ),
            fg(SUB),
        ))),
        row(body, 16),
    );
    sensei_row(f, body, app);
    let _ = now;
}

fn spar_rain(f: &mut Frame, stage: Rect, app: &App, s: &Spar) {
    let now = Instant::now();
    let body = round_chrome(
        f,
        stage,
        app,
        "대련",
        &s.lesson_id,
        "판정선에서 받아쳐라   Esc 일시정지",
    );
    let judged = s.notes.iter().filter(|n| n.judged.is_some()).count();
    f.render_widget(
        Paragraph::new(Line::from(vec![
            span(
                format!("  ■ 화살비 · {judged} / {}", s.notes.len()),
                bold(TEXT),
            ),
            span("        ", Style::new()),
            combo_span(s.combo),
        ])),
        row(body, 0),
    );

    // 네 갈래 — 판정선은 아래에서 셋째 줄
    const LANE_W: u16 = 8;
    let lanes = sub(body, 2, 1, LANE_W * 4 + 2, 19);
    let line_y = lanes.height - 3;
    let border = match s.flash {
        Some((false, _)) => RED,
        _ => BORDER,
    };
    f.render_widget(panel(Line::from("")).border_style(fg(border)), lanes);
    let inner = Rect {
        x: lanes.x + 1,
        y: lanes.y + 1,
        width: lanes.width - 2,
        height: lanes.height - 2,
    };
    let travel = f32::from(line_y - 1);
    if let Some(clock) = s.clock(now) {
        for n in s.notes.iter().filter(|n| n.judged.is_none()) {
            let left = n.at.as_secs_f32() - clock.as_secs_f32();
            let frac = left / s.fall.as_secs_f32();
            if !(-0.15..=1.0).contains(&frac) {
                continue;
            }
            let y = (f32::from(line_y - 1) - frac * travel).round() as i32;
            if y < 0 || y >= i32::from(inner.height) {
                continue;
            }
            let lane = LANES.iter().position(|a| *a == n.lane).unwrap_or(0) as u16;
            f.render_widget(
                Paragraph::new(Line::from(span(
                    format!(" ◆ {} ◆", n.lane.label()),
                    bold(lane_color(n.lane)),
                ))),
                Rect {
                    x: inner.x + lane * LANE_W,
                    y: inner.y + y as u16,
                    width: LANE_W,
                    height: 1,
                },
            );
        }
    }
    // 판정선과 갈래 이름
    f.render_widget(
        Paragraph::new(Line::from(span(
            "━".repeat(inner.width as usize),
            bold(GOLD),
        ))),
        Rect {
            y: inner.y + line_y - 1,
            height: 1,
            ..inner
        },
    );
    let mut labels = Vec::new();
    for a in LANES {
        labels.push(span(
            format!(" {} {:<4}", a.label(), fit(&app.bindings.key(a), 4)),
            bold(lane_color(a)),
        ));
    }
    f.render_widget(
        Paragraph::new(Line::from(labels)),
        Rect {
            y: inner.y + line_y,
            height: 1,
            ..inner
        },
    );
    if let Some(n) = s.countdown_left(now) {
        big_center(f, inner, &format!("— {n} —"), GOLD);
    }

    // 오른쪽 — 판정·보스·전적
    let right = sub(body, 40, 1, 39, 19);
    let mut lines: Vec<Line> = Vec::new();
    if let Some(b) = &s.boss {
        lines.extend(boss_art(&s.lesson_id, b, now));
        lines.push(Line::from(""));
        let mut hp = vec![span(" 보스 ", fg(SUB))];
        hp.extend(gauge(b.hp, 18, RED));
        lines.push(Line::from(hp));
        let mut ki = vec![span(" 기   ", fg(SUB))];
        ki.extend(gauge(b.ki, 18, if b.ki < 30 { YELLOW } else { GREEN }));
        lines.push(Line::from(ki));
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from(span(" 화살이 판정선에 닿는 순간", fg(SUB))));
        lines.push(Line::from(span(" 그 방향을 받아쳐라.", fg(SUB))));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            span(" 極 ", bold(GOLD)),
            span("±90ms  ", fg(DIM)),
            span("妙 ", bold(GREEN)),
            span("±180ms  ", fg(DIM)),
            span("可 ", bold(ACCENT)),
            span("±300ms", fg(DIM)),
        ]));
    }
    lines.push(Line::from(""));
    match s.last_judge {
        Some((j, at)) if now.duration_since(at) < Duration::from_millis(600) => {
            let c = match j {
                Judge::Perfect => GOLD,
                Judge::Great => GREEN,
                Judge::Good => ACCENT,
                Judge::Miss => RED,
            };
            lines.push(Line::from(span(format!("   {}", j.label()), bold(c))));
        }
        _ => lines.push(Line::from("")),
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        span(format!(" 極 {}  ", s.judged(Judge::Perfect)), fg(GOLD)),
        span(format!("妙 {}  ", s.judged(Judge::Great)), fg(GREEN)),
        span(format!("可 {}  ", s.judged(Judge::Good)), fg(ACCENT)),
        span(format!("失 {}", s.judged(Judge::Miss)), fg(RED)),
    ]));
    lines.push(Line::from(span(
        format!(" 최고 콤보 ×{}", s.max_combo),
        fg(SUB),
    )));
    f.render_widget(Paragraph::new(lines), right);
    sensei_row(f, body, app);
}

// ── 미로 ──────────────────────────────────────────────────────────────────────

pub fn maze(f: &mut Frame, stage: Rect, app: &App, r: &MazeRound) {
    let body = round_chrome(
        f,
        stage,
        app,
        "미로",
        &r.lesson_id,
        "결과 키로 걸어라   Tab 힌트 (별 -1)   Esc 일시정지",
    );
    let m = r.maze();
    let coins_total = m.coins.len();
    let mut head = vec![
        span(
            format!("  ■ 미로 {} / {} · {}", r.idx + 1, r.mazes.len(), m.title),
            bold(TEXT),
        ),
        span("    걸음 ", fg(SUB)),
        span(
            r.strokes.to_string(),
            bold(if r.strokes > r.par() { RED } else { YELLOW }),
        ),
        span("   Par ", fg(SUB)),
        span(r.par().to_string(), bold(TEXT)),
    ];
    if coins_total > 0 {
        head.push(span("   엽전 ", fg(SUB)));
        head.push(span(
            format!("◆ {} / {coins_total}", coins_total - r.coins_left.len()),
            bold(GOLD),
        ));
    }
    if m.fog {
        head.push(span("   어둠", fg(DIM)));
    }
    f.render_widget(Paragraph::new(Line::from(head)), row(body, 0));

    let hint_cells = if r.hint_shown {
        r.sage_cells()
    } else {
        Default::default()
    };
    let open = r.coins_left.is_empty();
    let blink = (r.started.elapsed().as_millis() / 400).is_multiple_of(2);
    let mut lines = Vec::new();
    for (y, rowv) in m.grid.iter().enumerate() {
        let mut v = Vec::new();
        for (x, t) in rowv.iter().enumerate() {
            let p = (x, y);
            if !r.visible(x, y) {
                v.push(span("░░", fg(Color::Rgb(40, 42, 60))));
                continue;
            }
            let cell = if p == r.pos {
                let c = match r.flash {
                    Some((false, _)) => RED,
                    Some((true, _)) => GREEN,
                    None => ACCENT,
                };
                span("◉ ", bold(c))
            } else if r.coins_left.contains(&p) {
                span("◆ ", bold(GOLD))
            } else {
                match t {
                    Tile::Wall => span("██", fg(BORDER)),
                    Tile::Exit if open => span("門", if blink { bold(GREEN) } else { bold(GOLD) }),
                    Tile::Exit => span("門", bold(RED)),
                    Tile::Floor if hint_cells.contains(&p) => span("∙ ", fg(GOLD)),
                    Tile::Floor if r.trail.contains(&p) => span("· ", fg(DIM)),
                    Tile::Floor => span("  ", Style::new()),
                }
            };
            v.push(cell);
        }
        lines.push(Line::from(v));
    }
    let w = (m.width() * 2 + 2) as u16;
    let h = m.grid.len() as u16 + 2;
    let x = (80u16.saturating_sub(w)) / 2;
    let border = match r.flash {
        Some((false, _)) => RED,
        _ => BORDER,
    };
    f.render_widget(
        Paragraph::new(lines).block(panel(Line::from("")).border_style(fg(border))),
        sub(body, x, 1, w, h),
    );
    if let MazePhase::Escaped(_) = r.phase {
        let res = r.results.last();
        let msg = match res {
            Some(h) if h.strokes <= h.sage => "탈  출  ◆ 사부와 같은 길",
            _ => "탈  출",
        };
        let area = sub(body, 20, 1 + h / 2 - 1, 40, 3);
        f.render_widget(Clear, area);
        f.render_widget(
            Paragraph::new(Line::from(span(msg, bold(GOLD))))
                .alignment(Alignment::Center)
                .block(rounded(Line::from("")).border_style(fg(GOLD))),
            area,
        );
    }
    let y = 1 + h;
    let mut keys = vec![span("  걸음  ", fg(SUB))];
    for a in [Action::Left, Action::Down, Action::Up, Action::Right] {
        keys.push(span(app.bindings.key(a), bold(ACT)));
        keys.push(span(format!("{}  ", a.glyph()), fg(SUB)));
    }
    keys.push(span(
        format!(
            "  ({} 을 쥐고)",
            app.bindings.trigger("nav").unwrap_or("nav")
        ),
        fg(DIM),
    ));
    if y < body.height - 2 {
        f.render_widget(Paragraph::new(Line::from(keys)), row(body, y));
    }
    feedback(f, row(body, body.height - 2), &r.feedback, YELLOW);
    sensei_row(f, body, app);
}

// ── 보이지 않는 마우스 (S10) ─────────────────────────────────────────────────

fn draw_box(
    f: &mut Frame,
    arena: Rect,
    r: &crate::tui::dojo::mouse::Rect,
    label: &str,
    color: Color,
    thick: bool,
) {
    let area = Rect {
        x: arena.x + r.x,
        y: arena.y + r.y,
        width: r.w.min(arena.width.saturating_sub(r.x)),
        height: r.h.min(arena.height.saturating_sub(r.y)),
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    if r.h >= 3 {
        let block =
            rounded(Line::from("")).border_style(if thick { bold(color) } else { fg(color) });
        f.render_widget(
            Paragraph::new(Line::from(span(label.to_string(), bold(color))))
                .alignment(Alignment::Center)
                .block(block),
            area,
        );
    } else {
        f.render_widget(
            Paragraph::new(Line::from(span(
                fit(label, r.w as usize),
                Style::new()
                    .bg(color)
                    .fg(MANTLE)
                    .add_modifier(Modifier::BOLD),
            ))),
            area,
        );
    }
}

pub fn mouse(f: &mut Frame, stage: Rect, app: &App, r: &MouseRound) {
    let now = Instant::now();
    let place = match r.drill {
        Drill::Move => "포인터 이동",
        Drill::Aim => "조준",
        Drill::Wheel => "휠 수색",
        Drill::Drag => "드래그",
        Drill::Leak => "새는 글자 0",
        Drill::Rats => "보스전",
    };
    let body = round_chrome(
        f,
        stage,
        app,
        place,
        &r.lesson_id,
        "포인터는 과녁장 안에서만 판정   Esc 일시정지",
    );
    let (done, total) = r.progress();
    let secs = r.elapsed(now).as_secs();
    let mut head = vec![span(format!("  ■ {place}"), bold(TEXT))];
    match r.drill {
        Drill::Leak => {}
        Drill::Rats => head.push(span(format!("    잡은 쥐 {done} / {total}"), fg(SUB))),
        _ => {
            head.push(span(format!("    명중 {done} / {total}"), fg(SUB)));
            head.push(span(
                format!("    시간 {}:{:02}", secs / 60, secs % 60),
                bold(if r.elapsed(now) > r.par_time {
                    RED
                } else {
                    TEXT
                }),
            ));
            head.push(span(format!(" / 기준 {}초", r.par_time.as_secs()), fg(DIM)));
        }
    }
    f.render_widget(Paragraph::new(Line::from(head)), row(body, 0));
    if let Some(b) = &r.boss {
        let name = app
            .content
            .lesson(&r.lesson_id)
            .map(|(_, _, l)| l.title.clone())
            .unwrap_or_default();
        f.render_widget(Paragraph::new(boss_gauges(&name, b, now)), row(body, 1));
    }

    // 과녁장 — 안쪽 좌표 (0,0) 이 무대 (2,4)
    let outer = sub(body, 1, 2, ARENA_W + 2, ARENA_H + 2);
    let border = match r.flash {
        Some((true, _)) => GREEN,
        Some((false, _)) => RED,
        None => BORDER,
    };
    f.render_widget(
        panel(Line::from(span(" 과녁장 ", bold(ACCENT)))).border_style(fg(border)),
        outer,
    );
    let arena = Rect {
        x: outer.x + 1,
        y: outer.y + 1,
        width: ARENA_W,
        height: ARENA_H,
    };
    match r.drill {
        Drill::Move | Drill::Aim => {
            if let Some(t) = r.target() {
                let (label, color) = match t.button {
                    Some(crossterm::event::MouseButton::Right) => ("우", HOLD),
                    Some(_) => ("좌", ACT),
                    None => ("◎", GOLD),
                };
                let label = if t.rect.h >= 3 && t.button.is_some() {
                    format!("◎ {label}")
                } else {
                    label.to_string()
                };
                draw_box(f, arena, &t.rect, &label, color, true);
            }
            // 다음 과녁 미리 보기 (흐리게)
            if let Some(n) = r.targets.get(r.idx + 1) {
                draw_box(f, arena, &n.rect, "", Color::Rgb(60, 62, 84), false);
            }
        }
        Drill::Wheel => {
            let want = r.wanted.get(r.idx).copied();
            let name = want
                .and_then(|i| r.list.get(i))
                .cloned()
                .unwrap_or_default();
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    span(" 찾아라 ▶ ", fg(SUB)),
                    span(name, bold(GOLD)),
                ])),
                Rect { height: 1, ..arena },
            );
            for k in 0..LIST_VIEW {
                let i = r.offset + k;
                let Some(item) = r.list.get(i) else { break };
                let hot = Some(i) == want;
                let style = if hot {
                    Style::new()
                        .bg(GOLD)
                        .fg(MANTLE)
                        .add_modifier(Modifier::BOLD)
                } else if item.starts_with('卷') {
                    fg(TEXT)
                } else {
                    fg(DIM)
                };
                f.render_widget(
                    Paragraph::new(Line::from(span(
                        format!(" {:>3}  {}", i + 1, fit(item, 60)),
                        style,
                    ))),
                    Rect {
                        y: arena.y + 1 + k as u16,
                        height: 1,
                        width: arena.width - 2,
                        ..arena
                    },
                );
            }
            // 스크롤 막대
            let n = r.list.len();
            let pos = r.offset * (LIST_VIEW - 1) / n.saturating_sub(LIST_VIEW).max(1);
            for k in 0..LIST_VIEW {
                f.render_widget(
                    Paragraph::new(Line::from(span(
                        if k == pos { "┃" } else { "│" },
                        fg(if k == pos { ACCENT } else { DIM }),
                    ))),
                    Rect {
                        x: arena.x + arena.width - 1,
                        y: arena.y + 1 + k as u16,
                        width: 1,
                        height: 1,
                    },
                );
            }
            let dir = match want {
                Some(w) if w >= r.offset + LIST_VIEW => " ▼ 더 아래에 있다",
                Some(w) if w < r.offset => " ▲ 더 위에 있다",
                Some(_) => " ◆ 보인다 — 집어라",
                None => "",
            };
            f.render_widget(
                Paragraph::new(Line::from(span(dir, fg(SUB)))),
                Rect {
                    y: arena.y + arena.height - 1,
                    height: 1,
                    ..arena
                },
            );
        }
        Drill::Drag => {
            draw_box(f, arena, &r.folder, "▣ 비급 서고", GOLD, true);
            let color = if r.dragging.is_some() { ACCENT } else { TEXT };
            draw_box(f, arena, &r.icon, "≡ 일지", color, r.dragging.is_some());
        }
        Drill::Leak => {
            let trigger = app.bindings.trigger("mouse").unwrap_or("mouse 레이어 키");
            let (text, color) = match r.leak {
                LeakPhase::Ready => (
                    format!("Enter 로 시작 — 3초 안에 {trigger} 를 쥐어라"),
                    TEXT,
                ),
                LeakPhase::Count(_) => (
                    format!(
                        "{trigger} 를 쥐어라 …  {}",
                        r.countdown_left(now).unwrap_or(0)
                    ),
                    GOLD,
                ),
                LeakPhase::Active(_) => (
                    format!(
                        "마구 쳐라!  남은 {:.1}초",
                        r.leak_left(now).unwrap_or_default().as_secs_f32()
                    ),
                    PEACH,
                ),
                LeakPhase::Done => ("끝".into(), SUB),
            };
            big_center(f, Rect { height: 5, ..arena }, &text, color);
            let leak_color = if r.leaks == 0 { GREEN } else { RED };
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(vec![
                        span("새어 나온 글자  ", fg(SUB)),
                        span(r.leaks.to_string(), bold(leak_color)),
                    ]),
                    Line::from(vec![
                        span("마우스 움직임  ", fg(SUB)),
                        span("▮".repeat((r.activity as usize).min(40)), fg(ACCENT)),
                        span(format!(" {}", r.activity), fg(SUB)),
                    ]),
                ])
                .alignment(Alignment::Center),
                Rect {
                    y: arena.y + 7,
                    height: 2,
                    ..arena
                },
            );
        }
        Drill::Rats => {
            if let Some(clock) = r.clock(now) {
                for rat in &r.rats {
                    if clock < rat.born {
                        continue;
                    }
                    match rat.state {
                        RatState::Running => {
                            let x = rat.x(clock).round() as i32;
                            for (k, ch) in rat.glyph().chars().enumerate() {
                                let cx = x + k as i32;
                                if (0..i32::from(ARENA_W)).contains(&cx) {
                                    f.render_widget(
                                        Paragraph::new(Line::from(span(
                                            ch.to_string(),
                                            bold(TEXT),
                                        ))),
                                        Rect {
                                            x: arena.x + cx as u16,
                                            y: arena.y + rat.y,
                                            width: 1,
                                            height: 1,
                                        },
                                    );
                                }
                            }
                        }
                        RatState::Caught(at)
                            if now.duration_since(at) < Duration::from_millis(500) => {}
                        _ => {}
                    }
                }
                let _ = RAT_W;
            }
            if let Some(n) = r.countdown_left(now) {
                big_center(f, arena, &format!("— {n} — 쥐가 온다"), GOLD);
            }
            if let Some(b) = &r.boss {
                if b.recently_struck(now) {
                    f.render_widget(
                        Paragraph::new(Line::from(span(
                            "찍찍! 결국 너는 나를 잡게 될 것이다!",
                            bold(RED),
                        )))
                        .alignment(Alignment::Right),
                        Rect { height: 1, ..arena },
                    );
                }
            }
        }
    }
    // 맞힌 자리 — 터지는 별
    if let Some(((x, y), at)) = r.burst {
        let e = now.duration_since(at).as_millis();
        let glyph = if e < 130 {
            "✶"
        } else if e < 260 {
            "✷"
        } else {
            "·"
        };
        if x >= 0 && y >= 0 && x < i32::from(ARENA_W) && y < i32::from(ARENA_H) {
            f.render_widget(
                Paragraph::new(Line::from(span(glyph, bold(GOLD)))),
                Rect {
                    x: arena.x + x as u16,
                    y: arena.y + y as u16,
                    width: 1,
                    height: 1,
                },
            );
        }
    }
    // 포인터 위치 표시
    if r.drill != Drill::Leak {
        if let Some((x, y)) = r.pointer {
            f.render_widget(
                Paragraph::new(Line::from(span("✛", bold(ACCENT)))),
                Rect {
                    x: arena.x + x as u16,
                    y: arena.y + y as u16,
                    width: 1,
                    height: 1,
                },
            );
        }
    }

    // 아래 — 배치대로 그린 마우스 레이어 안내
    let b = &app.bindings;
    let k = |a: Action| b.key(a);
    let guide = Line::from(vec![
        span("  ", Style::new()),
        span(
            b.trigger("mouse").unwrap_or("mouse").to_string(),
            bold(HOLD),
        ),
        span(" 쥐고  ", fg(SUB)),
        span(
            format!(
                "{}↑ {}← {}↓ {}→",
                k(Action::PointerUp),
                k(Action::PointerLeft),
                k(Action::PointerDown),
                k(Action::PointerRight)
            ),
            bold(ACT),
        ),
        span("  ", Style::new()),
        span(k(Action::ClickLeft), bold(ACT)),
        span(" 좌클릭  ", fg(SUB)),
        span(k(Action::ClickRight), bold(ACT)),
        span(" 우클릭  ", fg(SUB)),
        span(
            format!("{}/{}", k(Action::WheelUp), k(Action::WheelDown)),
            bold(ACT),
        ),
        span(" 휠  ", fg(SUB)),
        span(k(Action::Precision), bold(ACT)),
        span(" 정밀", fg(SUB)),
    ]);
    f.render_widget(Paragraph::new(guide), row(body, 17));
    let tip = if r.drill == Drill::Move
        && !r.moved_seen
        && now.duration_since(r.started) > Duration::from_secs(4)
    {
        Some("이 터미널은 포인터 움직임을 보고하지 않는 듯하다 — 과녁을 클릭해도 된다".to_string())
    } else if r.drill == Drill::Leak && r.leak == LeakPhase::Ready {
        Some("마우스 레이어는 매핑 안 된 키를 막는다 — 그래서 마구 쳐도 안전하다".to_string())
    } else {
        None
    };
    if let Some(t) = tip {
        f.render_widget(
            Paragraph::new(Line::from(span(format!("  {t}"), fg(DIM)))),
            row(body, 18),
        );
    }
    feedback(f, row(body, 19), &r.feedback, YELLOW);
    sensei_row(f, body, app);
}

// ── S11 한영 리듬 ─────────────────────────────────────────────────────────────

pub fn ime(f: &mut Frame, stage: Rect, app: &App, r: &ImeRound) {
    use unicode_width::UnicodeWidthChar;
    let now = Instant::now();
    let hint = match r.phase {
        ImePhase::Click => "mouse 레이어를 쥐고 과녁을 클릭   Esc 일시정지",
        _ => "따라 쳐라 · 틀리면 지우고   Esc 일시정지",
    };
    let body = round_chrome(f, stage, app, "한영 리듬", &r.lesson_id, hint);
    let secs = r.elapsed(now).as_secs();
    let mut head = vec![
        span("  ■ 한영 리듬", bold(TEXT)),
        span(format!("    {} / {} 줄", r.idx + 1, r.lines.len()), fg(SUB)),
        span(
            format!("    시간 {}:{:02}", secs / 60, secs % 60),
            bold(if r.par_time.is_some_and(|p| r.elapsed(now) > p) {
                RED
            } else {
                TEXT
            }),
        ),
    ];
    if let Some(p) = r.par_time {
        head.push(span(format!(" / 기준 {}초", p.as_secs()), fg(DIM)));
    }
    head.push(span("    전환 놓침 ", fg(SUB)));
    head.push(span(
        r.mode_misses.to_string(),
        bold(if r.mode_misses > 0 { RED } else { GREEN }),
    ));
    if r.combo >= 5 {
        head.push(span(format!("    ×{}", r.combo), bold(GOLD)));
    }
    f.render_widget(Paragraph::new(Line::from(head)), row(body, 0));
    if let Some(b) = &r.boss {
        let name = app
            .content
            .lesson(&r.lesson_id)
            .map(|(_, _, l)| l.title.clone())
            .unwrap_or_default();
        f.render_widget(Paragraph::new(boss_gauges(&name, b, now)), row(body, 1));
    }

    // 과녁장과 같은 자리 — 줄 사이 과녁의 마우스 좌표가 mouse 수련과 같다
    let outer = sub(body, 1, 2, ARENA_W + 2, ARENA_H + 2);
    let border = match r.flash {
        Some((true, _)) => GREEN,
        Some((false, _)) => RED,
        None => BORDER,
    };
    f.render_widget(
        panel(Line::from(span(" 두루마리 ", bold(ACCENT)))).border_style(fg(border)),
        outer,
    );
    let arena = Rect {
        x: outer.x + 1,
        y: outer.y + 1,
        width: ARENA_W,
        height: ARENA_H,
    };
    let left = 4u16;
    let line_at = |y: u16| Rect {
        x: arena.x + left,
        y: arena.y + y,
        width: arena.width - left,
        height: 1,
    };

    // 지금 상태 안내 — 다음 글자의 언어
    let state = match r.want_hangul() {
        _ if r.expected().is_none() && r.phase == ImePhase::Type => {
            span(" ⌫ 지우고 다시 ", Style::new().bg(RED).fg(MANTLE))
        }
        Some(true) => span(" 다음 · 한글 ", Style::new().bg(HOLD).fg(MANTLE)),
        Some(false) => span(" 다음 · 영문 ", Style::new().bg(ACT).fg(MANTLE)),
        None => span("", fg(DIM)),
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            state,
            span("   ", fg(DIM)),
            span("^", bold(GOLD)),
            span(
                format!(" = {} 톡", app.bindings.key(Action::ToggleHangul)),
                fg(SUB),
            ),
        ])),
        line_at(0),
    );

    // 목표 문장 + 그 아래 톡 표시(^). 표시는 ASCII 라 글자 폭이 흔들리지 않는다.
    let switches = r.switch_points(r.idx);
    let mut marks: Vec<Span> = Vec::new();
    let mut target: Vec<Span> = Vec::new();
    let done = r.typed.iter().take_while(|t| t.slip.is_none()).count();
    for (i, c) in r.line().chars().enumerate() {
        let w = c.width().unwrap_or(1);
        let mark = if switches.contains(&i) { "^" } else { " " };
        marks.push(span(
            format!("{mark}{}", " ".repeat(w.saturating_sub(1))),
            bold(if i < done { DIM } else { GOLD }),
        ));
        let style = if i < done {
            fg(GREEN)
        } else if i == done && r.phase == ImePhase::Type {
            Style::new()
                .fg(MANTLE)
                .bg(if super::super::hangul::is_hangul(c) {
                    HOLD
                } else {
                    ACT
                })
                .add_modifier(Modifier::BOLD)
        } else {
            bold(TEXT)
        };
        target.push(span(c.to_string(), style));
    }
    f.render_widget(Paragraph::new(Line::from(target)), line_at(2));
    f.render_widget(Paragraph::new(Line::from(marks)), line_at(3));

    // 도착한 글자 — 틀린 글자는 빨갛게. 터미널 커서를 끝에 두면 IME 조합 중인
    // 글자가 그 자리에 그려진다.
    let mut typed: Vec<Span> = vec![span("› ", fg(DIM))];
    let mut cursor_x = 2u16;
    for t in &r.typed {
        let style = match t.slip {
            None => fg(TEXT),
            Some(_) => Style::new().fg(MANTLE).bg(RED),
        };
        typed.push(span(t.ch.to_string(), style));
        cursor_x += t.ch.width().unwrap_or(1) as u16;
    }
    let typed_row = line_at(5);
    f.render_widget(
        Paragraph::new(Line::from(typed)),
        Rect {
            x: typed_row.x - 2,
            width: typed_row.width + 2,
            ..typed_row
        },
    );
    if r.phase == ImePhase::Type && app.pause.is_none() {
        f.set_cursor_position((typed_row.x - 2 + cursor_x, typed_row.y));
    }

    // 다음 줄 미리 보기
    if let Some(next) = r.lines.get(r.idx + 1) {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                span("다음  ", fg(DIM)),
                span(next.clone(), fg(Color::Rgb(70, 74, 100))),
            ])),
            line_at(7),
        );
    }

    match r.phase {
        ImePhase::Click => {
            if let Some(t) = &r.target {
                draw_box(f, arena, t, "쥠 ◎", GOLD, true);
            }
        }
        ImePhase::LineDone(_) => {
            big_center(
                f,
                Rect {
                    y: arena.y + 8,
                    height: 3,
                    ..arena
                },
                "— 한 줄 —",
                GREEN,
            );
        }
        ImePhase::Type => {}
    }

    feedback(f, row(body, body.height - 2), &r.feedback, YELLOW);
    sensei_row(f, body, app);
}

// ── S16 비급 시험 ─────────────────────────────────────────────────────────────

pub fn quiz(f: &mut Frame, stage: Rect, app: &App, q: &QuizRound) {
    let question = q.question();
    let scroll = app.content.scroll(&question.scroll);
    let mastery = app.scrolls.get(&question.scroll).map_or(0, |s| s.mastery);
    let right = vec![span(
        format!("문항 {} / {}", q.idx + 1, q.questions.len()),
        fg(SUB),
    )];
    let hint = match q.phase {
        QuizPhase::Ask => "↑↓ 고르기   1~4 바로 답   Enter 제출   Esc 일시정지",
        QuizPhase::Reveal(_) => "Enter 다음   Esc 일시정지",
    };
    let body = chrome(f, stage, "비급 시험", right, hint, app);

    let mut lines = vec![
        Line::from(""),
        Line::from(span(format!("  {}", question.q), bold(TEXT))),
        Line::from(""),
    ];
    for (i, c) in question.choices.iter().enumerate() {
        let (mark, style) = match q.phase {
            QuizPhase::Ask if i == q.sel => ("▶", sel()),
            QuizPhase::Ask => (" ", fg(TEXT)),
            QuizPhase::Reveal(_) if i == question.answer => ("✓", bold(GREEN)),
            QuizPhase::Reveal(p) if p == i => ("✗", bold(RED)),
            QuizPhase::Reveal(_) => (" ", fg(DIM)),
        };
        lines.push(Line::from(vec![
            span(format!("  {mark} {}   ", i + 1), style),
            span(fit(c, 66), style),
        ]));
    }
    let title = Line::from(vec![
        span(format!(" 卷{} 비급 시험  ", question.scroll), bold(GOLD)),
        span("숙련 ", fg(SUB)),
    ])
    .spans
    .into_iter()
    .chain(stars(mastery))
    .chain([span(" ", Style::new())])
    .collect::<Vec<_>>();
    let h = lines.len() as u16 + 2;
    f.render_widget(
        Paragraph::new(lines).block(rounded(Line::from(title))),
        sub(body, 1, 1, 78, h),
    );

    // 답을 냈으면 — 틀렸을 때는 그 권을 바로 펼친다
    if let QuizPhase::Reveal(p) = q.phase {
        let y = 1 + h + 1;
        let ok = p == question.answer;
        let mut text = vec![Line::from(span(
            if ok {
                "  ◆ 납득했다. 숙련 별이 하나 오른다."
            } else {
                "  ✗ 아직이다. 그 권을 다시 펼쳐 보아라."
            },
            if ok { bold(GREEN) } else { bold(RED) },
        ))];
        if let (false, Some(s)) = (ok, scroll) {
            text.push(Line::from(""));
            text.push(Line::from(span(
                format!("  {}", app.bindings.render(s.body.trim())),
                fg(TEXT),
            )));
        }
        f.render_widget(
            Paragraph::new(text)
                .wrap(ratatui::widgets::Wrap { trim: false })
                .block(panel(Line::from(span(
                    scroll.map_or(String::new(), |s| format!(" 卷{} {} ", s.id, s.title)),
                    fg(GOLD),
                )))),
            sub(body, 1, y, 78, body.height.saturating_sub(y + 1)),
        );
    }
    sensei_row(f, body, app);
}

// ── 이어 붙이기 머리 — 단계·보스 게이지 ──────────────────────────────────────

const STAGE_NUM: [&str; 9] = ["一", "二", "三", "四", "五", "六", "七", "八", "九"];

pub fn gauntlet(f: &mut Frame, stage: Rect, app: &App, g: &Gauntlet) {
    let now = Instant::now();
    let hd = Style::new().bg(MANTLE);
    let name = app
        .content
        .lesson(&g.lesson_id)
        .map_or(String::new(), |(_, _, l)| l.title.clone());
    let mut spans = vec![
        span(" 道場 ", hd.fg(PEACH).add_modifier(Modifier::BOLD)),
        span(
            if width(&name) > 20 {
                fit(&name, 20)
            } else {
                name.clone()
            },
            hd.fg(TEXT).add_modifier(Modifier::BOLD),
        ),
        span(" ", hd),
    ];
    for i in 0..g.stages.len() {
        let (mark, c) = if i < g.done.len() {
            ("◆", GOLD)
        } else if i == g.idx {
            ("◈", ACCENT)
        } else {
            ("◇", DIM)
        };
        spans.push(span(mark, hd.fg(c)));
    }
    spans.push(span(format!(" {} ", fit(&g.stage().title, 18)), hd.fg(SUB)));
    if let Some(b) = &g.boss {
        let bar = |pct: i32, c: Color| {
            let n = (pct.clamp(0, 100) as usize * 8).div_ceil(100);
            vec![
                span("█".repeat(n), hd.fg(c)),
                span("░".repeat(8 - n), hd.fg(DIM)),
            ]
        };
        spans.push(span(" 마", hd.fg(RED)));
        spans.extend(bar(b.hp, RED));
        spans.push(span(" 기", hd.fg(GREEN)));
        spans.extend(bar(b.ki, GREEN));
    }
    if let Some(left) = g.remaining_time(now) {
        let s = left.as_secs();
        spans.push(span(
            format!(" {}:{:02}", s / 60, s % 60),
            hd.fg(if s < 30 { RED } else { TEXT })
                .add_modifier(Modifier::BOLD),
        ));
    }
    let head = row(stage, 0);
    f.render_widget(Clear, head);
    f.render_widget(Paragraph::new(Line::from(spans)).style(hd), head);

    // 단계가 바뀐 직후 — 표지
    if g.banner(now) {
        let n = STAGE_NUM.get(g.idx).copied().unwrap_or("");
        let text = Line::from(vec![
            span(format!("第{n}關  "), bold(GOLD)),
            span(g.stage().title.clone(), bold(TEXT)),
        ]);
        let w = (width(&format!("第{n}關  {}", g.stage().title)) as u16 + 6).min(78);
        let area = sub(stage, (80 - w) / 2, 9, w, 3);
        f.render_widget(Clear, area);
        f.render_widget(
            Paragraph::new(text)
                .alignment(Alignment::Center)
                .block(rounded(Line::from("")).border_style(fg(GOLD))),
            area,
        );
    }
}
