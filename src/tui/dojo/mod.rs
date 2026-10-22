//! kmd dojo — 무협 도장 키맵 트레이너 (docs/10)
//!
//! `kmd dojo`로 들어온다. 런처 TUI와 별개의 터미널 세션이며 런처의 단일 인스턴스
//! 가드를 잡지 않는다.

pub mod action;
mod app;
pub mod binding;
pub mod content;
mod precheck;
pub mod progress;
pub mod score;
pub mod summon;
mod ui;

use std::time::{Duration, Instant};

use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

/// 쉬는 동안의 대기 — 키 훅 데몬이 도는 중에 CPU 를 태우지 않는다 (사고 수칙)
const IDLE_POLL: Duration = Duration::from_millis(250);

pub fn run(
    config: &kmd_core::Config,
    db: Option<kmd_core::Database>,
    lesson: Option<String>,
) -> color_eyre::Result<()> {
    let mut app = app::App::new(config, db, lesson);

    enable_raw_mode()?;
    let _guard = TerminalSession;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    terminal.clear()?;

    let mut dirty = true;
    while !app.quit {
        if dirty {
            terminal.draw(|f| ui::render(f, &app))?;
            dirty = false;
        }
        if event::poll(IDLE_POLL)? {
            match event::read()? {
                // Windows 는 Press/Release 를 모두 보낸다 — Press 만 처리
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    app.on_key(key);
                    dirty = true;
                }
                Event::Paste(text) => {
                    app.on_paste(&text);
                    dirty = true;
                }
                Event::Resize(..) => dirty = true,
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
        let _ = crossterm::execute!(out, DisableBracketedPaste);
        let _ = crossterm::execute!(out, LeaveAlternateScreen);
        let _ = crossterm::execute!(out, crossterm::cursor::Show);
        let _ = disable_raw_mode();
    }
}
