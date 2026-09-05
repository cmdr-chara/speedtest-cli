use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy)]
pub(super) struct Theme {
    pub background: Color,
    pub surface: Color,
    pub focus: Color,
    pub text: Color,
    pub muted: Color,
    pub line: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
}

impl Theme {
    pub fn detect() -> Self {
        let truecolor =
            std::env::var("COLORTERM").is_ok_and(|s| matches!(s.as_str(), "truecolor" | "24bit"));
        let indexed = std::env::var("TERM").is_ok_and(|s| s.contains("256color"));
        if truecolor {
            Self::rgb()
        } else if indexed {
            Self::indexed()
        } else {
            Self::ansi()
        }
    }

    pub const fn rgb() -> Self {
        Self {
            background: Color::Rgb(13, 20, 29),
            surface: Color::Rgb(25, 42, 56),
            focus: Color::Rgb(115, 220, 236),
            text: Color::Rgb(232, 240, 245),
            muted: Color::Rgb(158, 177, 192),
            line: Color::Rgb(57, 76, 91),
            success: Color::Rgb(140, 226, 189),
            warning: Color::Rgb(244, 199, 112),
            error: Color::Rgb(246, 139, 142),
        }
    }
    pub const fn indexed() -> Self {
        Self {
            background: Color::Indexed(233),
            surface: Color::Indexed(235),
            focus: Color::Indexed(117),
            text: Color::Indexed(255),
            muted: Color::Indexed(248),
            line: Color::Indexed(240),
            success: Color::Indexed(115),
            warning: Color::Indexed(222),
            error: Color::Indexed(210),
        }
    }
    pub const fn ansi() -> Self {
        Self {
            background: Color::Black,
            surface: Color::Black,
            focus: Color::Cyan,
            text: Color::White,
            muted: Color::Gray,
            line: Color::DarkGray,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
        }
    }
    pub fn base(self) -> Style {
        Style::default().bg(self.background).fg(self.text)
    }
    pub fn focus(self) -> Style {
        self.base().fg(self.focus).add_modifier(Modifier::BOLD)
    }
    pub fn muted(self) -> Style {
        self.base().fg(self.muted)
    }
    pub fn selected(self) -> Style {
        self.focus().bg(self.surface)
    }
}
