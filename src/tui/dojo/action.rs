//! 수(手, Action) — dojo가 가르치고 판정하는 단위.
//!
//! 콘텐츠는 물리 키가 아니라 수로 쓴다(docs/10 §6-5). 같은 수라도 사용자의
//! 배치·OS에 따라 다른 키로 눌리고 다른 키로 도착하기 때문이다.

/// 대상 OS — 키맵 대상 문자열의 의미가 OS마다 다르다 (macOS 단어← = Alt+Left)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

macro_rules! actions {
    ($($variant:ident => $label:literal),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Action { $($variant),* }

        impl Action {
            #[cfg(test)]
            pub const ALL: &'static [Action] = &[$(Action::$variant),*];

            /// 콘텐츠 자리표시자 이름 — `{WordLeft}`
            pub fn id(self) -> &'static str {
                match self { $(Action::$variant => stringify!($variant)),* }
            }

            /// 화면에 보이는 수 이름
            pub fn label(self) -> &'static str {
                match self { $(Action::$variant => $label),* }
            }

            pub fn from_id(id: &str) -> Option<Self> {
                match id { $(stringify!($variant) => Some(Action::$variant),)* _ => None }
            }
        }
    };
}

actions! {
    Left => "←",
    Down => "↓",
    Up => "↑",
    Right => "→",
    WordLeft => "단어 ←",
    WordRight => "단어 →",
    LineStart => "줄 처음",
    LineEnd => "줄 끝",
    PageUp => "페이지 ↑",
    PageDown => "페이지 ↓",
    Backspace => "앞 글자 지우기",
    Delete => "뒷 글자 지우기",
    PointerUp => "포인터 ↑",
    PointerDown => "포인터 ↓",
    PointerLeft => "포인터 ←",
    PointerRight => "포인터 →",
    ClickLeft => "좌클릭",
    ClickRight => "우클릭",
    ClickMiddle => "중클릭",
    WheelUp => "휠 ↑",
    WheelDown => "휠 ↓",
    Precision => "정밀 이동",
    ToggleHangul => "한/영",
    Launcher => "런처",
}

impl Action {
    /// 키맵 대상 문자열(`"Ctrl+Left"`, `"mouse:click"`, `"Hangul"` …)을 수로 읽는다.
    /// 수가 아닌 대상(`"Ctrl+V"` 등)은 `None` — 배치 진단에서만 쓴다.
    pub fn from_target(target: &str, os: Os) -> Option<Self> {
        let t = normalize(target);
        let mac = os == Os::Mac;
        Some(match t.as_str() {
            "left" => Action::Left,
            "down" => Action::Down,
            "up" => Action::Up,
            "right" => Action::Right,
            "pageup" | "pgup" => Action::PageUp,
            "pagedown" | "pgdn" => Action::PageDown,
            "backspace" | "bksp" => Action::Backspace,
            "delete" | "del" => Action::Delete,
            "home" => Action::LineStart,
            "end" => Action::LineEnd,
            "alt+left" if mac => Action::WordLeft,
            "alt+right" if mac => Action::WordRight,
            "cmd+left" if mac => Action::LineStart,
            "cmd+right" if mac => Action::LineEnd,
            "ctrl+left" if !mac => Action::WordLeft,
            "ctrl+right" if !mac => Action::WordRight,
            "mouse:up" => Action::PointerUp,
            "mouse:down" => Action::PointerDown,
            "mouse:left" => Action::PointerLeft,
            "mouse:right" => Action::PointerRight,
            "mouse:click" => Action::ClickLeft,
            "mouse:rclick" => Action::ClickRight,
            "mouse:mclick" => Action::ClickMiddle,
            "mouse:wheel-up" => Action::WheelUp,
            "mouse:wheel-down" => Action::WheelDown,
            "mouse:slow" => Action::Precision,
            "hangul" => Action::ToggleHangul,
            "launch:kmd-desktop" => Action::Launcher,
            _ => return None,
        })
    }
}

/// 대상 문자열 정규화 — 소문자, 공백 제거, 수정자 별칭 통일(`Command`→`cmd`, `Option`→`alt`)
fn normalize(target: &str) -> String {
    target
        .split('+')
        .map(|part| {
            let p = part.trim().to_ascii_lowercase();
            match p.as_str() {
                "command" | "meta" | "super" | "win" => "cmd".to_string(),
                "option" | "opt" | "lalt" | "ralt" => "alt".to_string(),
                "control" | "lctrl" | "rctrl" => "ctrl".to_string(),
                _ => p,
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 단어_이동은_os마다_다른_대상에서_온다() {
        assert_eq!(
            Action::from_target("Alt+Left", Os::Mac),
            Some(Action::WordLeft)
        );
        assert_eq!(Action::from_target("Alt+Left", Os::Windows), None);
        assert_eq!(
            Action::from_target("Ctrl+Left", Os::Windows),
            Some(Action::WordLeft)
        );
        assert_eq!(Action::from_target("Ctrl+Left", Os::Mac), None);
        assert_eq!(
            Action::from_target("Cmd+Right", Os::Mac),
            Some(Action::LineEnd)
        );
        assert_eq!(Action::from_target("End", Os::Linux), Some(Action::LineEnd));
    }

    #[test]
    fn 수가_아닌_대상은_없음() {
        assert_eq!(Action::from_target("Ctrl+V", Os::Windows), None);
        assert_eq!(Action::from_target("macro:Home;Shift+End", Os::Linux), None);
    }

    #[test]
    fn id는_왕복한다() {
        for &a in Action::ALL {
            assert_eq!(Action::from_id(a.id()), Some(a));
        }
    }

    #[test]
    fn 수정자_별칭과_대소문자를_가리지_않는다() {
        assert_eq!(
            Action::from_target("option + LEFT", Os::Mac),
            Some(Action::WordLeft)
        );
        assert_eq!(
            Action::from_target("Mouse:Click", Os::Linux),
            Some(Action::ClickLeft)
        );
    }
}
