//! 메뉴 계열 화면 — 입구·지도·입문·도감·기록·점검

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use super::*;
use crate::tui::dojo::action::Action;
use crate::tui::dojo::app::{os_name, App, SealView, MENU};
use crate::tui::dojo::binding::Trust;
use crate::tui::dojo::calib::{CalibRun, Step};
use crate::tui::dojo::card::{self, Tone};
use crate::tui::dojo::content::{Belt, Lesson};
use crate::tui::dojo::diagnose::{Diagnosis, Level};
use crate::tui::dojo::precheck::Backend;
use crate::tui::dojo::progress::Access;

pub fn too_small(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(span("道場", bold(PEACH))),
        Line::from(""),
        Line::from(span(
            format!(
                "창을 키워라 — 지금 {}×{}, 최소 80×24",
                area.width, area.height
            ),
            fg(TEXT),
        )),
        Line::from(span("Esc 하산", fg(DIM))),
    ];
    let y = area.height.saturating_sub(4) / 2;
    f.render_widget(
        Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
        Rect {
            y: area.y + y,
            height: area.height.saturating_sub(y),
            ..area
        },
    );
}

fn stars_total(app: &App) -> Vec<Span<'static>> {
    vec![span(
        format!(
            "★ {} / {}",
            app.standing.stars,
            app.content.lesson_count() * 3
        ),
        fg(GOLD),
    )]
}

/// 지금 수련 중인 띠 (전부 마쳤으면 단)
fn current_belt(app: &App) -> (String, Color) {
    match app.content.curriculum.belt.get(app.standing.current) {
        Some(b) => (b.name.clone(), belt_color(&b.id)),
        None => ("삼국 통일".into(), belt_color("dan")),
    }
}

// ── S2 입문 의식 (M1: 사전 점검) ──────────────────────────────────────────────

pub fn precheck(f: &mut Frame, stage: Rect, app: &App) {
    let r = &app.report;
    let term = r.terminal.clone().unwrap_or_else(|| "알 수 없음".into());
    let body = chrome(
        f,
        stage,
        "입문 의식",
        vec![span(term.clone(), fg(SUB))],
        "Enter 도장으로   C 키 감응 시험   D 배치 진단   Ctrl+C 하산",
        app,
    );

    let ok = |good: bool| {
        if good {
            span(" ✓ ", bold(GREEN))
        } else {
            span(" ! ", bold(YELLOW))
        }
    };
    let backend_line = match r.backend {
        Backend::Off => {
            "꺼져 있음 — 레이어 수련은 판정이 안 된다 (kmd daemon start / kmd keymap start)"
                .to_string()
        }
        b => format!("{} · 프로필 {}", b.label(), r.profile),
    };
    let trust_line = match r.trust {
        Trust::Full => "해석됨 — 안내와 시연을 네 배치로 그린다".to_string(),
        Trust::Unknown => "직접 고친 kanata 배치 — 키 대신 수 이름으로 안내한다".to_string(),
    };
    let b = &app.bindings;
    let nav = b.trigger("nav").unwrap_or("없음").to_string();
    let mouse = b.trigger("mouse").unwrap_or("없음").to_string();
    let lines = vec![
        Line::from(vec![
            ok(r.backend != Backend::Off),
            span("키맵       ", fg(SUB)),
            span(backend_line, fg(TEXT)),
        ]),
        Line::from(vec![
            ok(r.trust == Trust::Full),
            span("배치       ", fg(SUB)),
            span(trust_line, fg(TEXT)),
        ]),
        Line::from(vec![
            ok(true),
            span("레이어     ", fg(SUB)),
            span("nav ", fg(SUB)),
            span(nav, bold(HOLD)),
            span(" 홀드 · mouse ", fg(SUB)),
            span(mouse, bold(HOLD)),
            span(" 홀드 · 한/영 ", fg(SUB)),
            span(b.label(Action::ToggleHangul), bold(ACT)),
        ]),
        Line::from(vec![
            ok(true),
            span("터미널     ", fg(SUB)),
            span(term, fg(TEXT)),
        ]),
    ];
    let boxed = sub(body, 1, 1, 78, lines.len() as u16 + 2);
    f.render_widget(
        Paragraph::new(lines).block(panel(Line::from(span(" 사전 점검 ", bold(ACCENT))))),
        boxed,
    );

    let status = if app.calib.done {
        Line::from(vec![
            span("   ✓ ", bold(GREEN)),
            span("이 터미널은 시험을 마쳤다", fg(TEXT)),
        ])
    } else {
        Line::from(vec![
            span("   ! ", bold(YELLOW)),
            span("아직 시험 전 — 기본 별칭표로 판정한다. ", fg(TEXT)),
            span("C", bold(ACT)),
            span(" 로 시작 (1분)", fg(SUB)),
        ])
    };
    let missing: Vec<&str> = app.calib.unobservable.iter().map(|a| a.label()).collect();
    let calib = vec![
        Line::from(span(
            "   같은 수도 터미널마다 다른 키로 도착한다 (macOS 단어← → Terminal.app 은 ESC b).",
            fg(SUB),
        )),
        status,
        Line::from(span(
            if missing.is_empty() {
                "   관측 불가 수: 없음".to_string()
            } else {
                format!(
                    "   관측 불가 수: {} — 그 수가 주제인 과는 미보유로, Par 는 그 수 없이 계산한다",
                    missing.join(", ")
                )
            },
            fg(if missing.is_empty() { SUB } else { YELLOW }),
        )),
    ];
    f.render_widget(
        Paragraph::new(calib).block(panel(Line::from(span(" 키 감응 보정 ", bold(ACCENT))))),
        sub(body, 1, 8, 78, 5),
    );
    f.render_widget(
        Paragraph::new(sensei_line(
            "같은 수라도 터미널마다 다른 소리로 도착한다. 듣는 법부터 익혀라.",
        )),
        row(body, 16),
    );
}

// ── S2 키 감응 시험 ───────────────────────────────────────────────────────────

pub fn calib(f: &mut Frame, stage: Rect, app: &App, run: &CalibRun) {
    let term = app
        .report
        .terminal
        .clone()
        .unwrap_or_else(|| "알 수 없음".into());
    let done = run.current().is_none();
    let hint = if done {
        "Enter 점검으로"
    } else {
        "요청한 수를 네 배치대로   Tab 도착하지 않음   Esc 그만"
    };
    let body = chrome(
        f,
        stage,
        "입문 의식 · 키 감응 시험",
        vec![span(term.clone(), fg(SUB))],
        hint,
        app,
    );
    let now = std::time::Instant::now();
    let mut lines = Vec::new();
    for (i, a) in run.steps.iter().enumerate() {
        let chord = app.bindings.label(*a);
        let (mark, status, style) = match run.results.get(i) {
            Some(Step::Observed(id)) => (span(" ✓ ", bold(GREEN)), format!("도착 {id}"), fg(SUB)),
            Some(Step::Missing) => (
                span(" ✗ ", bold(YELLOW)),
                "관측 불가".to_string(),
                fg(YELLOW),
            ),
            None if i == run.results.len() => (
                span(" ▶ ", bold(ACCENT)),
                "기다리는 중".to_string(),
                bold(TEXT),
            ),
            None => (span("   ", Style::new()), String::new(), fg(DIM)),
        };
        lines.push(Line::from(vec![
            mark,
            span(fit(a.label(), 16), style),
            span(fit(&chord, 18), bold(ACT)),
            span(status, fg(DIM)),
        ]));
    }
    let h = lines.len() as u16 + 2;
    f.render_widget(
        Paragraph::new(lines).block(panel(Line::from(span(
            format!(" {term} 에 도착하는 수 "),
            bold(ACCENT),
        )))),
        sub(body, 1, 0, 50, h),
    );
    let mut side = Vec::new();
    match run.current() {
        Some(a) => {
            side.push(Line::from(span(" 지금 누를 수", fg(SUB))));
            side.push(Line::from(""));
            side.push(Line::from(span(format!(" {}", a.label()), bold(GOLD))));
            side.push(Line::from(span(
                format!(" {}", app.bindings.label(a)),
                bold(ACT),
            )));
            side.push(Line::from(""));
            if run.is_double(a) {
                side.push(Line::from(span(" 두 번 톡톡 — 첫 탭은", fg(SUB))));
                side.push(Line::from(span(" 단어 이동으로 먼저 온다", fg(SUB))));
            }
            if let Some((other, at)) = run.wrong {
                if now.duration_since(at) < std::time::Duration::from_millis(1500) {
                    side.push(Line::from(span(
                        format!(" 그건 {} 다 — 다시", other.label()),
                        bold(RED),
                    )));
                }
            }
            let left = run.remaining(now).as_secs_f32();
            let w = 20usize;
            let fill = ((left / 8.0) * w as f32) as usize;
            side.push(Line::from(""));
            side.push(Line::from(vec![
                span(" ", Style::new()),
                span("▮".repeat(fill.min(w)), fg(ACCENT)),
                span("▯".repeat(w - fill.min(w)), fg(DIM)),
            ]));
            side.push(Line::from(span(
                format!(" {left:.0}초 뒤 관측 불가"),
                fg(DIM),
            )));
        }
        None => {
            let ok = run
                .results
                .iter()
                .filter(|r| matches!(r, Step::Observed(_)))
                .count();
            side.push(Line::from(span(" 감응 시험 끝", bold(GOLD))));
            side.push(Line::from(""));
            side.push(Line::from(span(
                format!(" {} 수 중 {ok} 수 도착", run.steps.len()),
                fg(TEXT),
            )));
            let miss: Vec<&str> = app.calib.unobservable.iter().map(|a| a.label()).collect();
            if !miss.is_empty() {
                side.push(Line::from(""));
                side.push(Line::from(span(" 관측 불가", bold(YELLOW))));
                for m in miss {
                    side.push(Line::from(span(format!(" · {m}"), fg(YELLOW))));
                }
                side.push(Line::from(""));
                side.push(Line::from(span(" 그 수 없이 Par 를 다시", fg(SUB))));
                side.push(Line::from(span(" 계산한다. 잠기지 않는다.", fg(SUB))));
            }
        }
    }
    f.render_widget(
        Paragraph::new(side).block(rounded(Line::from(span(" 감응 ", bold(GOLD))))),
        sub(body, 53, 0, 26, 16),
    );
    let _ = done;
    f.render_widget(
        Paragraph::new(sensei_line(
            "같은 수라도 터미널마다 다른 소리로 도착한다. 듣는 법부터 익혀라.",
        )),
        row(body, body.height - 1),
    );
}

// ── S1 도장 입구 ──────────────────────────────────────────────────────────────

pub fn lobby(f: &mut Frame, stage: Rect, app: &App) {
    let (belt, belt_c) = current_belt(app);
    let right = vec![
        span("■ ", fg(belt_c)),
        span(format!("{belt} 수련 중 · "), fg(SUB)),
    ]
    .into_iter()
    .chain(stars_total(app))
    .collect();
    let body = chrome(
        f,
        stage,
        "",
        right,
        "↑↓ 이동   Enter 들어가기   Esc 하산",
        app,
    );

    let center = ratatui::layout::Alignment::Center;
    let banner = vec![
        Line::from(span("━".repeat(44), fg(DIM))),
        Line::from(vec![
            span("道  場", bold(PEACH)),
            span("      k m d   d o j o", bold(TEXT)),
        ]),
        Line::from(span("손이 떠나면 진다 — 홈로우 위의 무공", fg(SUB))),
        Line::from(span("━".repeat(44), fg(DIM))),
    ];
    f.render_widget(
        Paragraph::new(banner).alignment(center),
        sub(body, 0, 1, 80, 4),
    );

    let mut menu = vec![Line::from("")];
    for (i, (item, name, note)) in MENU.iter().enumerate() {
        let streak;
        let note = if *item == crate::tui::dojo::app::MenuItem::Daily {
            streak = if app.streak.today_done {
                format!("✓ 연속 {}일", app.streak.days)
            } else {
                format!("● {note}")
            };
            streak.as_str()
        } else {
            note
        };
        let text = format!(
            " {} {}{}",
            if i == app.lobby_sel { "▶" } else { " " },
            fit(name, 14),
            fit(note, 12)
        );
        menu.push(Line::from(span(
            text,
            if i == app.lobby_sel { sel() } else { fg(TEXT) },
        )));
    }
    f.render_widget(
        Paragraph::new(menu).block(rounded(Line::from(span(" 문(門) ", bold(ACCENT))))),
        sub(body, 2, 6, 34, 9),
    );

    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "수련생".into());
    let st = app.standing.belts.get(app.standing.current);
    let (done, req) = st.map_or((0, 0), |s| (s.done, s.required));
    let bar_w = 14usize;
    let filled = if req == 0 { 0 } else { bar_w * done / req };
    let remain = app.standing.remaining().unwrap_or(0);
    let seal = |name: &str, id: &str| {
        if app.seals.iter().any(|s| s.os == id) {
            span(format!("◆ {name}  "), bold(GOLD))
        } else {
            span(format!("◇ {name}  "), fg(DIM))
        }
    };
    let b = &app.bindings;
    let prof = vec![
        Line::from(""),
        Line::from(vec![span(" 수련생  ", fg(SUB)), span(user, bold(TEXT))]),
        Line::from(vec![
            span(" 띠      ", fg(SUB)),
            span(format!("■■■ {belt}"), bold(belt_c)),
        ]),
        Line::from(vec![
            span("         ", Style::new()),
            span("━".repeat(filled), fg(GREEN)),
            span("━".repeat(bar_w - filled), fg(DIM)),
            span(format!(" 다음 띠까지 {remain}과"), fg(SUB)),
        ]),
        Line::from(vec![
            span(" 배치    ", fg(SUB)),
            span(b.trigger("nav").unwrap_or("—").to_string(), bold(HOLD)),
            span(" nav · ", fg(SUB)),
            span(b.trigger("mouse").unwrap_or("—").to_string(), bold(HOLD)),
            span(" mouse", fg(SUB)),
        ]),
        Line::from(vec![
            span(" 연속    ", fg(SUB)),
            span("▮".repeat(app.streak.days.min(7) as usize), bold(GOLD)),
            span("▯".repeat(7 - app.streak.days.min(7) as usize), fg(DIM)),
            span(format!(" {}일 (검은띠 조건 3일)", app.streak.days), fg(SUB)),
        ]),
        Line::from(vec![
            span(" 인장    ", fg(SUB)),
            seal("macOS", "macos"),
            seal("Windows", "windows"),
            seal("Linux", "linux"),
        ]),
        Line::from(""),
    ];
    f.render_widget(
        Paragraph::new(prof).block(rounded(Line::from(span(" 수련생 ", bold(ACCENT))))),
        sub(body, 37, 6, 43, 9),
    );
    f.render_widget(Paragraph::new(sensei_line(&app.sensei_line)), row(body, 17));
}

// ── S3 수련 지도 ──────────────────────────────────────────────────────────────

fn access_tag(access: &Access) -> Span<'static> {
    match access {
        Access::Open => span("", Style::new()),
        Access::LockedBelt | Access::LockedBoss => span("잠김", fg(DIM)),
        Access::Building => span("공사 중", fg(DIM)),
        Access::Missing(_) => span("수 없음", fg(YELLOW)),
    }
}

pub fn map(f: &mut Frame, stage: Rect, app: &App) {
    let body = chrome(
        f,
        stage,
        "수련 지도",
        stars_total(app),
        "↑↓ 과 선택   ←→ 띠 이동   Enter 입문   Esc 문으로",
        app,
    );
    let belts = &app.content.curriculum.belt;

    // 왼쪽 — 띠 사다리 (위가 높은 띠)
    let mut ladder = Vec::new();
    for (i, b) in belts.iter().enumerate().rev() {
        let selected = app.map_belt == i;
        let st = &app.standing.belts[i];
        let status = if st.completed {
            format!("★ {}", st.stars)
        } else if i == app.standing.current {
            "수련 중".into()
        } else if i > app.standing.current {
            "잠김".into()
        } else {
            format!("★ {}", st.stars)
        };
        let status_style = if i == app.standing.current {
            fg(TEXT)
        } else if st.completed {
            fg(GOLD)
        } else {
            fg(DIM)
        };
        ladder.push(Line::from(vec![
            span(if selected { "▶" } else { " " }, bold(ACCENT)),
            span("■ ", fg(belt_color(&b.id))),
            span(fit(&b.name, 7), if selected { sel() } else { fg(TEXT) }),
            span(fit(&status, 8), if selected { sel() } else { status_style }),
        ]));
    }
    f.render_widget(
        Paragraph::new(ladder).block(panel(Line::from(span(" 띠 ", bold(ACCENT))))),
        sub(body, 1, 1, 20, 11),
    );

    // 오른쪽 — 선택한 띠의 과
    let right = sub(body, 22, 1, 57, 15);
    let Some(belt) = belts.get(app.map_belt) else {
        return;
    };

    let mut lines = vec![
        Line::from(span(
            format!(" \"{}\"", app.bindings.render(&belt.motto)),
            fg(SAY),
        )),
        Line::from(""),
    ];
    let lessons = app.map_lessons();
    for (i, l) in lessons.iter().enumerate() {
        let selected = i == app.map_sel;
        let access = app.access(app.map_belt, l);
        let label = if l.is_boss() {
            format!("⚔ 관문  {}", l.title)
        } else {
            format!("{}과  {}", i + 1, l.title)
        };
        if l.is_boss() {
            lines.push(Line::from(""));
        }
        let mut row = vec![
            span(if selected { " ▶ " } else { "   " }, bold(ACCENT)),
            span(
                fit(&label, 34),
                if selected {
                    sel()
                } else if l.is_boss() {
                    fg(RED)
                } else {
                    fg(TEXT)
                },
            ),
            span(" ", Style::new()),
        ];
        match access {
            Access::Open => {
                row.extend(stars(app.stars(&l.id)));
                if let Some(best) = app.progress.get(&l.id).and_then(|p| p.best_strokes) {
                    row.push(span(format!("  {best}타"), fg(SUB)));
                }
            }
            other => row.push(access_tag(&other)),
        }
        lines.push(Line::from(row));
    }
    let st = &app.standing.belts[app.map_belt];
    lines.push(Line::from(""));
    let rule = match belt.id.as_str() {
        "black" => format!(
            " 승급: 오늘의 수련 3일 연속 — 지금 연속 {}일 · 최고 {}일",
            app.streak.days, app.streak.best
        ),
        "dan" => format!(
            " 인장 {}/3 — 세 OS 의 인장을 모은 뒤 삼두마를 꺾어라",
            app.seals.len()
        ),
        _ => format!(" 승급: 과 {}/{} ★1 이상 + 관문 격파", st.done, st.required),
    };
    lines.push(Line::from(span(rule, fg(SUB))));
    let title = Line::from(vec![
        span(" ■ ", fg(belt_color(&belt.id))),
        span(format!("{} · {} ", belt.name, belt.theme), bold(ACCENT)),
    ]);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(title)),
        right,
    );

    // 아래 — 미리보기
    if let Some(l) = lessons.get(app.map_sel) {
        let preview = Paragraph::new(Line::from(span(
            format!(" {}", app.bindings.render(&l.why)),
            fg(TEXT),
        )))
        .wrap(Wrap { trim: false })
        .block(panel(Line::from(span(" 미리보기 ", fg(SUB)))));
        f.render_widget(preview, sub(body, 1, 17, 78, 4));
    }
}

// ── S4 과 입문 ────────────────────────────────────────────────────────────────

/// 과에 필요한 수의 쥐는 키·누르는 키
fn lesson_keys(app: &App, actions: &[Action]) -> (Vec<String>, Vec<String>) {
    let mut hold = Vec::new();
    let mut act = Vec::new();
    for &a in actions {
        // 선택 수 = 바탕 이동 + Shift
        let (a, shift) = match a.unshifted() {
            Some(base) => (base, true),
            None => (a, false),
        };
        if shift && !hold.iter().any(|h: &String| h == "LShift") {
            hold.push("LShift".into());
        }
        if let Some(b) = app.bindings.primary(a) {
            if b.tap_trigger {
                act.push(b.key.clone());
                continue;
            }
            if let Some(h) = &b.hold {
                if !hold.contains(h) {
                    hold.push(h.clone());
                }
            }
            act.push(b.key.clone());
        }
    }
    (hold, act)
}

/// 과가 쓰는 수 — 관문은 따로 적지 않으면 그 띠의 과들이 쓰는 수 전부.
/// 선택 코스면 Shift 도 쥐는 손으로 칠한다.
fn lesson_actions(app: &App, belt: &Belt, lesson: &Lesson) -> (Vec<Action>, bool) {
    let mut v = lesson.needed_actions();
    // 대련 카드의 수 묶음도 손에 칠한다
    v.extend(lesson.moves.iter().filter_map(|m| Action::from_id(m)));
    if v.is_empty() && lesson.is_boss() {
        for l in &belt.lesson {
            v.extend(l.needed_actions());
        }
    }
    let selecting = lesson.kind == "golf"
        && lesson
            .set
            .as_deref()
            .and_then(|s| app.content.golf_course(s))
            .is_some_and(|c| {
                c.moves
                    .iter()
                    .filter_map(|m| Action::from_id(m))
                    .any(|a| a.unshifted().is_some())
            });
    let mut seen = Vec::new();
    v.retain(|a| {
        let new = !seen.contains(a);
        seen.push(*a);
        new
    });
    (v, selecting)
}

pub fn intro(f: &mut Frame, stage: Rect, app: &App, id: &str) {
    let Some((belt_idx, belt, lesson)) = app.content.lesson(id) else {
        return;
    };
    let scroll = lesson
        .scroll
        .as_deref()
        .and_then(|s| app.content.scroll(s))
        .map(|s| format!("비급 卷{} · {}", s.id, s.title))
        .unwrap_or_default();
    let access = app.access(belt_idx, lesson);
    let hint = match access {
        Access::Open if lesson.kind == "summon" => {
            "Enter 수련 시작   R 도장 밖 실전 (내 인덱스)   Esc 지도로"
        }
        Access::Open if lesson.kind == "seal" => "Enter 인장 화면   Esc 지도로",
        Access::Open => "Enter 수련 시작   Esc 지도로",
        _ => "Esc 지도로",
    };
    let body = chrome(f, stage, "입문", vec![span(scroll, fg(SUB))], hint, app);

    let kind = if lesson.is_boss() {
        "관문".to_string()
    } else {
        let n = belt
            .lesson
            .iter()
            .position(|l| l.id == lesson.id)
            .unwrap_or(0);
        format!("{}과", n + 1)
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            span("  ■ ", fg(belt_color(&belt.id))),
            span(
                format!("{} · {}   {}", belt.name, kind, lesson.title),
                bold(TEXT),
            ),
        ])),
        row(body, 1),
    );

    let mut teach = vec![
        Line::from(""),
        Line::from(span(
            format!("  \"{}\"", app.bindings.render(&lesson.why)),
            fg(SAY),
        )),
    ];
    if let Some(line) = &lesson.line {
        teach.push(Line::from(span(
            format!("  {} : \"{line}\"", lesson.title),
            fg(RED),
        )));
    }
    teach.push(Line::from(""));
    for t in &lesson.teach {
        teach.push(Line::from(vec![
            span("  · ", fg(ACCENT)),
            span(app.bindings.render(t), fg(TEXT)),
        ]));
    }
    let teach_h = teach.len() as u16 + 2;
    f.render_widget(
        Paragraph::new(teach)
            .wrap(Wrap { trim: false })
            .block(rounded(Line::from(span(" 사부의 가르침 ", bold(GOLD))))),
        sub(body, 1, 3, 78, teach_h),
    );

    let y = 3 + teach_h;
    let (actions, selecting) = lesson_actions(app, belt, lesson);
    let (mut hold, act) = lesson_keys(app, &actions);
    if selecting {
        hold.push("LShift".into());
    }
    if let Some((title, lines)) = special_panel(app, lesson) {
        let h = lines.len() as u16 + 2;
        f.render_widget(
            Paragraph::new(lines)
                .block(panel(Line::from(span(format!(" {title} "), bold(ACCENT))))),
            sub(body, 1, y, 78, h),
        );
    } else if lesson.kind == "summon" {
        // 소환술 — 키보드 대신 루프를 보여 준다
        let lines = vec![
            Line::from(""),
            Line::from(vec![
                span("   [런처 소환]", bold(HOLD)),
                span("  ──▶  ", fg(DIM)),
                span("[몇 글자]", bold(ACT)),
                span("  ──▶  ", fg(DIM)),
                span("[1순위 확인]", fg(TEXT)),
                span("  ──▶  ", fg(DIM)),
                span("[Enter]", bold(GREEN)),
            ]),
            Line::from(""),
            Line::from(span(
                "   타수 = 친 글자 + ↓/↑ + Enter. 사부의 수와 Par 는 결과 화면에서 공개된다.",
                fg(SUB),
            )),
        ];
        f.render_widget(
            Paragraph::new(lines).block(panel(Line::from(span(" 소환 루프 ", bold(ACCENT))))),
            sub(body, 1, y, 78, 6),
        );
    } else if app.bindings.trust == Trust::Unknown {
        let lines = vec![Line::from(span(
            "  배치를 해석할 수 없다(직접 고친 kanata 배치) — 키 대신 수 이름으로 안내한다.",
            fg(YELLOW),
        ))];
        f.render_widget(
            Paragraph::new(lines).block(panel(Line::from(span(" 두 손 ", bold(ACCENT))))),
            sub(body, 1, y, 78, 3),
        );
    } else {
        let mut lines = keyboard(&hold, &act);
        lines.push(Line::from(vec![
            span("   ■ 쥐는 손", bold(HOLD)),
            span("           ", Style::new()),
            span("■ 움직이는 손", bold(ACT)),
        ]));
        let h = lines.len() as u16 + 2;
        f.render_widget(
            Paragraph::new(lines).block(panel(Line::from(span(" 두 손 ", bold(ACCENT))))),
            sub(body, 1, y, 78, h),
        );
    }

    let status = match access {
        Access::Open => None,
        Access::LockedBelt => Some("앞 띠를 먼저 넘어라.".to_string()),
        Access::LockedBoss => Some("이 띠의 과를 모두 ★1 이상 받으면 관문이 열린다.".to_string()),
        Access::Building => {
            Some("짓는 중인 수련장이다 — 다음 판에 열린다. 안내는 네 배치로 미리 본다.".to_string())
        }
        Access::Missing(miss) => Some(format!(
            "네 배치엔 {} 수가 없다 — 승급 조건에서 빠진다.",
            miss.iter()
                .map(|a| a.label())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    };
    let last = body.height.saturating_sub(2);
    match status {
        Some(s) => f.render_widget(
            Paragraph::new(Line::from(span(format!("  {s}"), fg(YELLOW)))),
            row(body, last),
        ),
        None => f.render_widget(
            Paragraph::new(sensei_line(intro_tip(lesson))),
            row(body, last),
        ),
    }
}

/// 키보드 대신 보여 줄 판 — 이어 붙이기(단계)·시험·오늘의 수련·인장
fn special_panel(app: &App, lesson: &Lesson) -> Option<(&'static str, Vec<Line<'static>>)> {
    let stage_line = |i: usize, label: &str, kind: &str| {
        Line::from(vec![
            span(format!("   {}  ", i + 1), bold(GOLD)),
            span(fit(label, 40), fg(TEXT)),
            span(kind_name(kind), fg(SUB)),
        ])
    };
    match lesson.kind.as_str() {
        "mixed" => {
            let lines = lesson
                .stage
                .iter()
                .enumerate()
                .map(|(i, st)| {
                    let src = app.content.lesson(&st.from).map(|(_, _, l)| l);
                    let label = st
                        .label
                        .clone()
                        .or_else(|| src.map(|l| l.title.clone()))
                        .unwrap_or_default();
                    stage_line(i, &label, src.map_or("", |l| l.kind.as_str()))
                })
                .collect();
            Some(("단계", lines))
        }
        "daily" => {
            let st = app.streak;
            let mut lines = vec![Line::from(vec![
                span("   연속 ", fg(SUB)),
                span(format!("{}일", st.days), bold(GOLD)),
                span(
                    format!("   최고 {}일   완주한 날 {}", st.best, st.total),
                    fg(SUB),
                ),
                span(
                    if st.today_done {
                        "   오늘 ✓ 마침"
                    } else {
                        "   오늘 아직"
                    },
                    if st.today_done { fg(GREEN) } else { fg(YELLOW) },
                ),
            ])];
            for (i, l) in app.daily_preview().iter().enumerate() {
                lines.push(stage_line(i, &l.title, &l.kind));
            }
            Some(("오늘의 수련", lines))
        }
        "quiz" => Some((
            "시험",
            vec![
                Line::from(span(
                    format!(
                        "   {}문 · 6문 이상 맞히면 통과 · 연 비급 {}권 중 숙련이 낮은 권부터",
                        lesson.count.unwrap_or(10),
                        app.scrolls.len()
                    ),
                    fg(TEXT),
                )),
                Line::from(span(
                    "   ↑↓ 고르기 · 1~4 바로 답 · Enter 제출 · 권별 시험은 비급 도감에서",
                    fg(SUB),
                )),
            ],
        )),
        "seal" => {
            let mut lines = vec![];
            for (id, name) in [
                ("macos", "macOS"),
                ("windows", "Windows"),
                ("linux", "Linux"),
            ] {
                let got = app.seals.iter().find(|s| s.os == id);
                lines.push(Line::from(vec![
                    span(
                        format!(
                            "   {} {}",
                            if got.is_some() { "◆" } else { "◇" },
                            fit(name, 9)
                        ),
                        if got.is_some() { bold(GOLD) } else { fg(DIM) },
                    ),
                    span(
                        match got {
                            Some(s) if s.source == "imported" => "다른 기기에서 들임",
                            Some(_) => "이 기기에서 땀",
                            None => "아직",
                        },
                        fg(SUB),
                    ),
                ]));
            }
            Some(("인장", lines))
        }
        _ => None,
    }
}

fn kind_name(kind: &str) -> &'static str {
    match kind {
        "summon" => "소환술",
        "golf" => "키 골프",
        "spar" => "대련",
        "maze" => "미로",
        "mouse" => "포인터",
        "ime" => "한영 리듬",
        _ => "",
    }
}

/// 입문 화면 사부 한마디 — 수련 종류별 요령
fn intro_tip(lesson: &Lesson) -> &'static str {
    match (
        lesson.kind.as_str(),
        lesson.style.as_deref(),
        lesson.set.as_deref(),
    ) {
        ("summon", ..) => "타수 = 친 글자 + ↓/↑ + Enter. 사부의 수는 결과에서 공개된다.",
        ("spar", Some("rain"), _) => "시작하면 3초 뒤 화살이 떨어진다. 눈은 판정선에 둬라.",
        ("spar", ..) => "처음 여섯 장은 키를 보여 준다. 그다음은 몸이 기억해야 한다.",
        ("maze", ..) => "Tab 은 사부의 길을 비춘다 — 대신 별 하나를 내놓아라.",
        ("golf", _, Some("select")) => {
            "Shift 를 함께 쥐고 움직이면 선택이 된다. 금빛 밑줄을 정확히 감싸라."
        }
        ("golf", _, Some("surgery" | "typo" | "paragraph" | "boss3")) => {
            "감싸고 한 번에 벤다. 성한 글자가 섞인 선택은 칼이 튕긴다."
        }
        ("golf", ..) => "타수 = 도착한 결과 키 수. 두 번 톡톡 수는 2타다.",
        ("mouse", _, Some("leak")) => {
            "이 수련은 이 창의 마우스 캡처로 판정한다. 포인터를 창 안에 둬라."
        }
        ("mouse", ..) => "포인터를 이 창 안으로 데려와라. 과녁장 밖은 판정하지 않는다.",
        ("ime", _, Some("taphold")) => {
            "줄 사이 과녁은 mouse 레이어를 쥐고 클릭 — 놓은 뒤에도 한/영은 그대로여야 한다."
        }
        ("ime", _, Some("guard")) => {
            "톡 톡을 너무 빨리 치면 두 번째는 무시된다(300ms). 한 번이면 충분하다."
        }
        ("ime", ..) => "^ 표시가 있는 글자 앞에서 톡. 틀린 언어로 쳤다면 사부가 되짚어 준다.",
        ("mixed", ..) if lesson.is_boss() => {
            "실수와 파 초과가 기를 깎는다. 단계를 넘길 때마다 마귀가 꺾인다."
        }
        ("mixed", ..) => "단계가 끝나면 바로 다음 단계다. 숨은 단계 사이에서 골라라.",
        ("quiz", ..) => "보기 순서는 판마다 섞인다. 자리를 외우지 말고 이치를 떠올려라.",
        ("daily", ..) => "하루 2분. 거르지 않는 것이 실력이다.",
        ("seal", ..) => "인장은 코드로 옮긴다. 위조는 막지 않는다 — 정직 시스템이다.",
        _ => "아직 짓는 중이다. 안내만 먼저 읽어 두어라.",
    }
}

// ── S15 비급 도감 ─────────────────────────────────────────────────────────────

pub fn codex(f: &mut Frame, stage: Rect, app: &App) {
    let total = app.content.scrolls.scroll.len();
    let right = vec![span(
        format!("{} / {total} 해금", app.scrolls.len()),
        fg(GOLD),
    )];
    let body = chrome(
        f,
        stage,
        "비급 도감",
        right,
        "↑↓ 권 선택   Enter 비급 시험   Esc 문으로",
        app,
    );

    let visible = 18usize;
    let start = app.codex_sel.saturating_sub(visible - 1);
    let mut list = Vec::new();
    for (i, s) in app
        .content
        .scrolls
        .scroll
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
    {
        let selected = i == app.codex_sel;
        let state = app.scrolls.get(&s.id);
        let mut spans = vec![
            span(if selected { "▶" } else { " " }, bold(ACCENT)),
            span(format!("{} ", fit(&s.id, 4)), fg(GOLD)),
        ];
        match state {
            Some(st) => {
                spans.push(span(
                    fit(&s.title, 14),
                    if selected { sel() } else { fg(TEXT) },
                ));
                spans.extend(stars(st.mastery));
            }
            None => spans.push(span(fit("???", 17), if selected { sel() } else { fg(DIM) })),
        }
        list.push(Line::from(spans));
    }
    f.render_widget(
        Paragraph::new(list).block(panel(Line::from(span(" 권 ", bold(ACCENT))))),
        sub(body, 1, 1, 27, 20),
    );

    let s = &app.content.scrolls.scroll[app.codex_sel];
    let detail_area = sub(body, 29, 1, 50, 20);
    match app.scrolls.get(&s.id) {
        Some(st) => {
            let mut lines = vec![Line::from("")];
            lines.extend(
                s.body
                    .trim()
                    .lines()
                    .map(|l| Line::from(span(format!(" {l}"), fg(TEXT)))),
            );
            lines.push(Line::from(""));
            lines.push(Line::from(span(format!(" 출처 {}", s.source), fg(DIM))));
            lines.push(Line::from(span(
                format!(" 숙련 {}/3 — 비급 시험은 다음 판에서 열린다", st.mastery),
                fg(DIM),
            )));
            let title = Line::from(vec![
                span(format!(" 卷{} ", s.id), bold(GOLD)),
                span(format!("{} ", s.title), bold(TEXT)),
            ]);
            f.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .block(panel(title)),
                detail_area,
            );
        }
        None => {
            let unlock = app
                .content
                .curriculum
                .belt
                .iter()
                .flat_map(|b| b.lesson.iter().chain(b.boss.iter()).map(move |l| (b, l)))
                .find(|(_, l)| l.scroll.as_deref() == Some(s.id.as_str()))
                .map(|(b, l)| format!("{} · {}", b.name, l.title))
                .unwrap_or_default();
            let lines = vec![
                Line::from(""),
                Line::from(span(" 아직 펼치지 못한 비급이다.", fg(SUB))),
                Line::from(""),
                Line::from(span(format!(" 해금: {unlock}"), fg(DIM))),
            ];
            f.render_widget(
                Paragraph::new(lines).block(panel(Line::from(span(
                    format!(" 卷{} ??? ", s.id),
                    bold(GOLD),
                )))),
                detail_area,
            );
        }
    }
}

// ── 수련 기록 ─────────────────────────────────────────────────────────────────

pub fn records(f: &mut Frame, stage: Rect, app: &App) {
    let body = chrome(f, stage, "수련 기록", stars_total(app), "Esc 문으로", app);
    let runs = app
        .db
        .as_ref()
        .map(|d| d.dojo_recent_runs(16))
        .unwrap_or_default();
    let mut lines = vec![Line::from(span(
        format!(
            " {}  {}  {}  {}",
            fit("과", 30),
            fit("별", 6),
            fit("타수/Par", 10),
            "점수"
        ),
        fg(SUB),
    ))];
    if runs.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(span(
            " 아직 기록이 없다. 첫 과부터 시작하라.",
            fg(DIM),
        )));
    }
    for r in runs {
        let title = app
            .content
            .lesson(&r.lesson)
            .map(|(_, b, l)| format!("{} · {}", b.name, l.title))
            .unwrap_or(r.lesson.clone());
        let strokes = match (r.strokes, r.par) {
            (Some(s), Some(p)) => format!("{s}/{p}"),
            _ => "—".into(),
        };
        let mut spans = vec![span(format!(" {}  ", fit(&title, 30)), fg(TEXT))];
        spans.extend(stars(r.stars));
        spans.push(span(
            format!("     {}  {}", fit(&strokes, 10), r.score),
            fg(SUB),
        ));
        lines.push(Line::from(spans));
    }
    f.render_widget(
        Paragraph::new(lines).block(panel(Line::from(span(" 최근 ", bold(ACCENT))))),
        sub(body, 1, 1, 78, 20),
    );
}

// ── S19 배치 진단 ─────────────────────────────────────────────────────────────

pub fn diagnose(f: &mut Frame, stage: Rect, app: &App, d: &Diagnosis, scroll: usize) {
    let changed = d.changed();
    let right = vec![span(
        if app.keymap_changed {
            "배치가 바뀌었다".to_string()
        } else {
            "네 배치와 표준".to_string()
        },
        fg(if app.keymap_changed { YELLOW } else { SUB }),
    )];
    let body = chrome(
        f,
        stage,
        "배치 진단",
        right,
        "Enter 입문 의식 다시 (권장)   Space 그대로 수련   ↑↓ 표 넘기기",
        app,
    );
    let say = if app.keymap_changed {
        "배치가 바뀌었구나. 도장은 네 손에 맞춰 다시 차린다."
    } else if changed == 0 {
        "표준 그대로다. 비급의 이치가 네 손에 그대로 있다."
    } else {
        "네 배치다. 이치에 비추어 보마 — 무엇도 막지는 않는다."
    };
    f.render_widget(Paragraph::new(sensei_line(say)), row(body, 1));

    // 수 대 수 표 — 바뀐 줄이 먼저
    let view = 8usize;
    let start = scroll.min(d.rows.len().saturating_sub(view));
    let mut lines = vec![Line::from(vec![
        span(format!(" {}", fit("수", 14)), fg(DIM)),
        span(fit("도장 표준", 20), fg(DIM)),
        span(fit("네 배치", 22), fg(DIM)),
    ])];
    for r in d.rows.iter().skip(start).take(view) {
        let mark = if r.changed { "•" } else { " " };
        let mut v = vec![
            span(mark, bold(YELLOW)),
            span(fit(&r.name, 14), fg(if r.changed { TEXT } else { SUB })),
            span(fit(&r.standard, 20), fg(SUB)),
            span(
                fit(&r.mine, 22),
                if r.changed { bold(ACT) } else { fg(SUB) },
            ),
        ];
        if let Some(n) = &r.note {
            v.push(span(fit(n, 18), fg(YELLOW)));
        }
        lines.push(Line::from(v));
    }
    let more = if d.rows.len() > view {
        format!(
            " {}–{}/{} ",
            start + 1,
            (start + view).min(d.rows.len()),
            d.rows.len()
        )
    } else {
        String::new()
    };
    let title = Line::from(vec![
        span(
            format!(" 바뀐 수  {changed} / {} ", d.rows.len()),
            bold(ACCENT),
        ),
        span(more, fg(DIM)),
    ]);
    let table_h = lines.len() as u16 + 2;
    f.render_widget(
        Paragraph::new(lines).block(panel(title)),
        sub(body, 1, 2, 78, table_h),
    );

    // 사부의 진단 — 경고 · 정보 · 칭찬
    let y = 2 + table_h;
    let room = body.height.saturating_sub(y + 4) as usize;
    let mut diag = Vec::new();
    if d.findings.is_empty() {
        diag.push(Line::from(span(
            "  배치를 해석할 수 없어 진단은 쉰다 — 판정과 보정은 그대로 동작한다.",
            fg(SUB),
        )));
    }
    for fd in d.findings.iter().take(room.max(1)) {
        let (mark, color) = match fd.level {
            Level::Warn => ("!", YELLOW),
            Level::Info => ("i", ACCENT),
            Level::Good => ("✓", GREEN),
        };
        diag.push(Line::from(vec![
            span(format!(" {mark} "), bold(color)),
            span(format!("卷{} ", fd.scroll), bold(GOLD)),
            span(fit(&fd.text, 66), fg(TEXT)),
        ]));
    }
    let diag_h = diag.len() as u16 + 2;
    f.render_widget(
        Paragraph::new(diag).block(panel(Line::from(span(" 사부의 진단 ", bold(GOLD))))),
        sub(body, 1, y, 78, diag_h),
    );
    f.render_widget(
        Paragraph::new(Line::from(span(
            "  안내 문구 · 시연 · Par 는 네 배치로 다시 계산했다. 예전 기록은 남는다.",
            fg(SUB),
        ))),
        row(body, y + diag_h),
    );
}

// ── S17 결과 카드 ─────────────────────────────────────────────────────────────

pub fn card(f: &mut Frame, stage: Rect, app: &App) {
    let body = chrome(
        f,
        stage,
        "결과 카드",
        Vec::new(),
        "C 클립보드로 복사 (텍스트)   Esc 닫기",
        app,
    );
    let (_, belt_id) = card::belt_name(app.content, &app.standing);
    let lines: Vec<Line> = card::app_lines(app)
        .into_iter()
        .map(|(text, tone)| {
            let style = match tone {
                Tone::Title => bold(belt_color(&belt_id)),
                Tone::Text => fg(TEXT),
                Tone::Sub => fg(SUB),
                Tone::Gold => fg(GOLD),
                Tone::Bar => fg(ACT),
            };
            Line::from(span(format!(" {text}"), style))
        })
        .collect();
    let h = (lines.len() as u16 + 2).min(body.height);
    let w = card::WIDTH as u16 + 4;
    f.render_widget(
        Paragraph::new(lines).block(rounded(Line::from(span(" kmd dojo ", bold(PEACH))))),
        sub(body, (80 - w) / 2, 1, w, h),
    );
}

// ── 인장 (docs/10 §8-4) ───────────────────────────────────────────────────────

pub fn seals(f: &mut Frame, stage: Rect, app: &App, v: &SealView) {
    let hint = if v.input.is_some() {
        "코드를 붙여 넣고 Enter   Esc 취소"
    } else {
        "E 내 인장 코드 복사   I 코드 붙여 넣기   Esc 지도로"
    };
    let right = vec![span(format!("인장 {} / 3", app.seals.len()), fg(GOLD))];
    let body = chrome(f, stage, "인장", right, hint, app);

    let mut lines = vec![Line::from("")];
    for (id, name) in [
        ("macos", "macOS"),
        ("windows", "Windows"),
        ("linux", "Linux"),
    ] {
        let got = app.seals.iter().find(|s| s.os == id);
        let here = std::env::consts::OS == id;
        lines.push(Line::from(vec![
            span(
                format!(
                    "   {}  {}",
                    if got.is_some() { "◆" } else { "◇" },
                    fit(name, 9)
                ),
                if got.is_some() { bold(GOLD) } else { fg(DIM) },
            ),
            span(
                match got {
                    Some(s) if s.source == "imported" => "다른 기기에서 들임".to_string(),
                    Some(_) => "이 기기에서 땀".to_string(),
                    None if here => "이 기기 — 검은띠를 따면 찍힌다".to_string(),
                    None => "그 기기에서 내보낸 코드를 붙여 넣어라".to_string(),
                },
                fg(SUB),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(span(
        format!(
            "   이 기기는 {} — 코드는 kmd dojo seal export / import 로도 옮긴다",
            os_name(std::env::consts::OS)
        ),
        fg(SUB),
    )));
    f.render_widget(
        Paragraph::new(lines).block(rounded(Line::from(span(" 삼국의 인장 ", bold(GOLD))))),
        sub(body, 1, 1, 78, 9),
    );

    let mut code = vec![Line::from("")];
    match (&v.input, &v.code) {
        (Some(input), _) => {
            code.push(Line::from(vec![
                span("   붙여 넣기 ▸ ", fg(SUB)),
                span(fit(input, 58), bold(TEXT)),
            ]));
        }
        (None, Some(c)) => {
            code.push(Line::from(vec![
                span("   내 코드  ", fg(SUB)),
                span(fit(c, 62), bold(ACCENT)),
            ]));
        }
        (None, None) => code.push(Line::from(span("   이 기기의 인장이 아직 없다.", fg(DIM)))),
    }
    if let Some((ok, msg)) = &v.msg {
        code.push(Line::from(""));
        code.push(Line::from(span(
            format!("   {msg}"),
            if *ok { fg(GREEN) } else { fg(RED) },
        )));
    }
    f.render_widget(
        Paragraph::new(code)
            .wrap(Wrap { trim: false })
            .block(panel(Line::from(span(" 코드 ", bold(ACCENT))))),
        sub(body, 1, 11, 78, 7),
    );
    let all = app.seals.len() >= 3;
    f.render_widget(
        Paragraph::new(sensei_line(if all {
            "세 나라가 하나의 손버릇 아래 모였다. 삼두마가 기다린다."
        } else {
            "기기는 셋이어도 손은 하나다. 인장으로 그것을 증명하라."
        })),
        row(body, body.height - 1),
    );
}
