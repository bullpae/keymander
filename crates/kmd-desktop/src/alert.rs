//! 알림 창 — `kmd-desktop --alert <제목> <내용> <종류>` (docs/23 N3).
//!
//! 데몬이 타이머·리마인더가 끝날 때 띄운다. 런처와 별개 프로세스라 단일 실행 잠금을 잡지 않고,
//! 알림이 여러 개면 창도 여러 개다. 치던 글이 Enter 로 창을 닫아 버리지 않도록 Esc 와 클릭으로만 닫는다.
//! 모양은 런처와 같은 테마·카드다. 그림자는 그리지 않는다 — 창 둘레에서 잘려 테두리처럼 보이고,
//! macOS 는 투명 창의 불투명한 부분을 따라 그림자를 알아서 넣는다.

use iced::widget::{column, container, mouse_area, row, text, Space};
use iced::{
    alignment, font, keyboard, window, Border, Color, Element, Font, Length, Point, Size, Task,
    Theme,
};

use crate::theme::DesktopTheme;

const CARD_WIDTH: f32 = 420.0;
const CARD_HEIGHT: f32 = 104.0;
const TOP_MARGIN: f32 = 28.0;
const BADGE_SIZE: f32 = 52.0;
const TITLE_CHARS: usize = 28;
const BODY_CHARS: usize = 44;

/// `--alert` 인자
#[derive(Debug, PartialEq)]
pub struct AlertArgs {
    pub title: String,
    pub body: String,
    pub kind: String,
}

/// `--alert 제목 [내용] [종류]` 이면 그 값
pub fn parse_args(mut args: impl Iterator<Item = String>) -> Option<AlertArgs> {
    args.find(|a| a == kmd_core::notes::ALERT_ARG)?;
    Some(AlertArgs {
        title: args.next().unwrap_or_default(),
        body: args.next().unwrap_or_default(),
        kind: args.next().unwrap_or_default(),
    })
}

/// 넘치는 글은 줄임표로 — 창 크기는 고정이다
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// 종류별 (아이콘, 색)
fn look(kind: &str, t: &DesktopTheme) -> (&'static str, Color) {
    use kmd_core::notes::timers::{ALERT_BREAK, ALERT_FOCUS, ALERT_REMINDER};
    match kind {
        ALERT_FOCUS => ("\u{1F345}", t.red),
        ALERT_BREAK => ("\u{2615}\u{FE0F}", t.green),
        ALERT_REMINDER => ("\u{1F514}", t.yellow),
        _ => ("\u{23F1}\u{FE0F}", t.teal),
    }
}

#[derive(Debug, Clone)]
enum Message {
    Close,
}

struct Alert {
    title: String,
    body: String,
    icon: &'static str,
    accent: Color,
    theme: DesktopTheme,
}

impl Alert {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Close => iced::exit(),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let t = &self.theme;
        let accent = self.accent;
        let transparent = crate::window_transparent();

        let badge = container(text(self.icon).size(26))
            .width(BADGE_SIZE)
            .height(BADGE_SIZE)
            .align_x(alignment::Horizontal::Center)
            .align_y(alignment::Vertical::Center)
            .style(move |_: &_| container::Style {
                background: Some(Color { a: 0.16, ..accent }.into()),
                border: Border {
                    radius: 14.0.into(),
                    width: 1.0,
                    color: Color { a: 0.35, ..accent },
                },
                ..Default::default()
            });

        let title = text(self.title.clone()).size(17).color(t.text).font(Font {
            weight: font::Weight::Semibold,
            ..Font::DEFAULT
        });
        let mut lines = column![title].spacing(5);
        if !self.body.is_empty() {
            lines = lines.push(text(self.body.clone()).size(13).color(t.subtext));
        }
        let hint = text("Esc 또는 클릭으로 닫기").size(11).color(t.overlay);
        let content = row![
            badge,
            column![
                Space::new().height(Length::Fill),
                lines,
                Space::new().height(Length::Fill),
                row![Space::new().width(Length::Fill), hint],
            ]
            .width(Length::Fill)
        ]
        .spacing(14)
        .align_y(alignment::Vertical::Center);

        let background = t.background;
        let border_color = Color { a: 0.9, ..t.border };
        let card = container(content)
            .padding(iced::Padding {
                top: 14.0,
                right: 16.0,
                bottom: 10.0,
                left: 16.0,
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_: &_| container::Style {
                background: Some(background.into()),
                border: Border {
                    radius: if transparent { 16.0 } else { 0.0 }.into(),
                    width: 1.0,
                    color: border_color,
                },
                ..Default::default()
            });

        mouse_area(card).on_press(Message::Close).into()
    }
}

/// 알림 창을 띄우고 닫힐 때까지 기다린다
pub fn run(args: AlertArgs, theme: DesktopTheme) -> iced::Result {
    let (icon, accent) = look(&args.kind, &theme);
    let background = theme.background;
    let state = std::sync::Mutex::new(Some(Alert {
        title: clip(&args.title, TITLE_CHARS),
        body: clip(&args.body, BODY_CHARS),
        icon,
        accent,
        theme,
    }));
    let transparent = crate::window_transparent();
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
        size: Size::new(CARD_WIDTH, CARD_HEIGHT),
        decorations: false,
        transparent,
        level: window::Level::AlwaysOnTop,
        position: window::Position::SpecificWith(|win, monitor| {
            Point::new((monitor.width - win.width) / 2.0, TOP_MARGIN)
        }),
        resizable: false,
        ..Default::default()
    })
    .theme(|_: &Alert| Theme::Dark)
    .style(move |_: &Alert, _: &Theme| iced::theme::Style {
        background_color: if transparent {
            Color::TRANSPARENT
        } else {
            background
        },
        text_color: Color::WHITE,
    })
    .subscription(|_: &Alert| {
        keyboard::listen().filter_map(|event| match event {
            keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            } => Some(Message::Close),
            _ => None,
        })
    })
    .antialiasing(true)
    .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Option<AlertArgs> {
        parse_args(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn 인자_읽기() {
        assert_eq!(args(&["kmd-desktop"]), None);
        assert_eq!(args(&["kmd-desktop", "--memo"]), None);
        assert_eq!(
            args(&["kmd-desktop", "--alert", "라면 끝", "12분 전", "timer"]),
            Some(AlertArgs {
                title: "라면 끝".into(),
                body: "12분 전".into(),
                kind: "timer".into(),
            })
        );
        let only_title = args(&["kmd-desktop", "--alert", "휴식 끝"]).unwrap();
        assert_eq!(only_title.body, "");
        assert_eq!(only_title.kind, "");
    }

    #[test]
    fn 긴_글은_줄임표() {
        assert_eq!(clip("짧은 글", 10), "짧은 글");
        assert_eq!(clip("가나다라마", 3), "가나…");
    }
}
