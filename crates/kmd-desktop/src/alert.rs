//! 알림 창 — `kmd-desktop --alert <제목> <내용>` (docs/23 N3).
//!
//! 데몬이 타이머·리마인더가 끝날 때 띄운다. 런처와 별개 프로세스라 단일 실행 잠금을 잡지 않고,
//! 알림이 여러 개면 창도 여러 개다. 치던 글이 Enter 로 창을 닫아 버리지 않도록 Esc 와 클릭으로만 닫는다.

use iced::widget::{button, column, container, row, text, Space};
use iced::{keyboard, window, Element, Length, Point, Size, Task, Theme};

const WIDTH: f32 = 440.0;
const HEIGHT: f32 = 130.0;
const TOP_MARGIN: f32 = 48.0;

/// `--alert 제목 [내용]` 이면 (제목, 내용)
pub fn parse_args(mut args: impl Iterator<Item = String>) -> Option<(String, String)> {
    args.find(|a| a == kmd_core::notes::ALERT_ARG)?;
    let title = args.next().unwrap_or_default();
    Some((title, args.next().unwrap_or_default()))
}

#[derive(Debug, Clone)]
enum Message {
    Close,
}

struct Alert {
    title: String,
    body: String,
}

impl Alert {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Close => iced::exit(),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let mut lines = column![text(self.title.clone()).size(20)].spacing(6);
        if !self.body.is_empty() {
            lines = lines.push(text(self.body.clone()).size(14));
        }
        let close = button(text("확인")).on_press(Message::Close);
        container(column![
            lines,
            Space::new().height(Length::Fill),
            row![Space::new().width(Length::Fill), close]
        ])
        .padding(16)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}

/// 알림 창을 띄우고 닫힐 때까지 기다린다
pub fn run(title: String, body: String) -> iced::Result {
    let state = std::sync::Mutex::new(Some(Alert { title, body }));
    iced::application(
        move || {
            let alert = state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
                .expect("iced 초기화가 두 번 이상 호출됨 — 프레임워크 버그");
            (alert, Task::none())
        },
        Alert::update,
        Alert::view,
    )
    .window(window::Settings {
        size: Size::new(WIDTH, HEIGHT),
        decorations: false,
        level: window::Level::AlwaysOnTop,
        position: window::Position::SpecificWith(|win, monitor| {
            Point::new((monitor.width - win.width) / 2.0, TOP_MARGIN)
        }),
        resizable: false,
        ..Default::default()
    })
    .theme(|_: &Alert| Theme::Dark)
    .subscription(|_: &Alert| {
        keyboard::listen().filter_map(|event| match event {
            keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            } => Some(Message::Close),
            _ => None,
        })
    })
    .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Option<(String, String)> {
        parse_args(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn 인자_읽기() {
        assert_eq!(args(&["kmd-desktop"]), None);
        assert_eq!(args(&["kmd-desktop", "--memo"]), None);
        assert_eq!(
            args(&["kmd-desktop", "--alert", "라면 끝", "12분 전"]),
            Some(("라면 끝".into(), "12분 전".into()))
        );
        assert_eq!(
            args(&["kmd-desktop", "--alert", "휴식 끝"]),
            Some(("휴식 끝".into(), String::new()))
        );
    }
}
