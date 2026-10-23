//! kmd dojo — 무협 도장 키맵 트레이너 (docs/10)
//!
//! `kmd dojo`로 들어온다. 런처 TUI와 별개의 터미널 세션이며 런처의 단일 인스턴스
//! 가드를 잡지 않는다.

pub mod action;
mod app;
pub mod binding;
pub mod calib;
pub mod content;
pub mod golf;
pub mod maze;
pub mod mouse;
pub mod play;
mod precheck;
pub mod progress;
pub mod score;
pub mod spar;
pub mod summon;
mod ui;

use std::time::Instant;

use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyEventKind,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

pub fn run(
    config: &kmd_core::Config,
    db: Option<kmd_core::Database>,
    lesson: Option<String>,
    free: bool,
) -> color_eyre::Result<()> {
    let mut app = app::App::new(config, db, lesson, free);

    enable_raw_mode()?;
    let _guard = TerminalSession;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    terminal.clear()?;
    if let Ok(size) = terminal.size() {
        app.on_resize(size.width, size.height);
    }

    let mut dirty = true;
    // 마우스 캡처는 마우스 수련 중에만 켠다 — 메뉴에서는 터미널 글자 선택을 살려 둔다
    let mut mouse_on = false;
    while !app.quit {
        let want_mouse = app.wants_mouse();
        if want_mouse != mouse_on {
            if want_mouse {
                crossterm::execute!(terminal.backend_mut(), EnableMouseCapture)?;
            } else {
                crossterm::execute!(terminal.backend_mut(), DisableMouseCapture)?;
            }
            mouse_on = want_mouse;
        }
        if dirty {
            terminal.draw(|f| ui::render(f, &app))?;
            dirty = false;
        }
        // 움직이는 화면만 프레임 간격으로 돈다. 쉬는 화면은 250ms 대기 (사고 수칙)
        if event::poll(app.frame_interval(Instant::now()))? {
            match event::read()? {
                // Windows 는 Press/Release 를 모두 보낸다 — Press 만 처리
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    app.on_key(key);
                    dirty = true;
                }
                Event::Mouse(m) => dirty |= app.on_mouse(m),
                Event::Paste(text) => {
                    app.on_paste(&text);
                    dirty = true;
                }
                Event::Resize(w, h) => {
                    app.on_resize(w, h);
                    dirty = true;
                }
                _ => {}
            }
        }
        dirty |= app.on_tick(Instant::now());
    }
    Ok(())
}

/// 터미널 복구 RAII 가드 — 런처의 `TerminalSession`과 같은 방식.
/// 오류·패닉 어느 쪽으로 나가도 사용자 셸이 망가진 채 남지 않게 한다.
struct TerminalSession;

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let mut out = std::io::stdout();
        let _ = crossterm::execute!(out, DisableMouseCapture);
        let _ = crossterm::execute!(out, DisableBracketedPaste);
        let _ = crossterm::execute!(out, LeaveAlternateScreen);
        let _ = crossterm::execute!(out, crossterm::cursor::Show);
        let _ = disable_raw_mode();
    }
}
