use ratatui::style::{Color, Modifier, Style};

/// Presentation preference only: no terminal queries or profile changes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum Palette {
    #[default]
    Terminal,
    Graphite,
    Light,
    Monochrome,
}

impl Palette {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal (adaptive)",
            Self::Graphite => "Graphite",
            Self::Light => "Light",
            Self::Monochrome => "Monochrome",
        }
    }

    pub fn cycle(self, forward: bool) -> Self {
        let choices = [
            Self::Terminal,
            Self::Graphite,
            Self::Light,
            Self::Monochrome,
        ];
        let index = choices.iter().position(|p| *p == self).unwrap_or(0);
        choices[(index + if forward { 1 } else { choices.len() - 1 }) % choices.len()]
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ColorDepth {
    TrueColor,
    Indexed,
    Basic,
}

impl ColorDepth {
    pub fn detect() -> Self {
        if std::env::var("COLORTERM")
            .is_ok_and(|s| matches!(s.to_ascii_lowercase().as_str(), "truecolor" | "24bit"))
        {
            Self::TrueColor
        } else if std::env::var("TERM").is_ok_and(|s| s.contains("256color")) {
            Self::Indexed
        } else {
            Self::Basic
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Theme {
    pub background: Color,
    pub focus: Color,
    pub text: Color,
    pub muted: Color,
    pub line: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
}

impl Theme {
    pub fn resolve(palette: Palette, depth: ColorDepth) -> Self {
        match (palette, depth) {
            (Palette::Terminal, _) => Self::ansi(),
            (Palette::Monochrome, _) => Self::monochrome(),
            (Palette::Graphite, ColorDepth::TrueColor) => Self::rgb(),
            (Palette::Graphite, ColorDepth::Indexed) => Self::indexed(),
            (Palette::Light, ColorDepth::TrueColor) => Self::light(),
            (Palette::Light, ColorDepth::Indexed) => Self {
                background: Color::Indexed(231),
                text: Color::Indexed(234),
                muted: Color::Indexed(239),
                line: Color::Indexed(243),
                focus: Color::Indexed(24),
                success: Color::Indexed(22),
                warning: Color::Indexed(94),
                error: Color::Indexed(124),
            },
            // A limited-color terminal's own foreground/background is safer than
            // guessing whether its "white" and "black" slots are light or dark.
            (_, ColorDepth::Basic) => Self::ansi(),
        }
    }

    /// Native defaults and the user's ANSI palette work on light/dark terminals
    /// on every platform, including SSH/tmux. Capability is not theme detection.
    pub const fn ansi() -> Self {
        Self {
            background: Color::Reset,
            text: Color::Reset,
            muted: Color::Reset,
            line: Color::Reset,
            focus: Color::Cyan,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
        }
    }
    pub const fn monochrome() -> Self {
        Self {
            focus: Color::Reset,
            success: Color::Reset,
            warning: Color::Reset,
            error: Color::Reset,
            ..Self::ansi()
        }
    }
    pub const fn rgb() -> Self {
        Self {
            background: Color::Rgb(13, 20, 29),
            focus: Color::Rgb(115, 220, 236),
            text: Color::Rgb(232, 240, 245),
            muted: Color::Rgb(184, 198, 210),
            line: Color::Rgb(108, 129, 147),
            success: Color::Rgb(140, 226, 189),
            warning: Color::Rgb(244, 199, 112),
            error: Color::Rgb(246, 139, 142),
        }
    }
    pub const fn indexed() -> Self {
        Self {
            background: Color::Indexed(233),
            focus: Color::Indexed(117),
            text: Color::Indexed(255),
            muted: Color::Indexed(251),
            line: Color::Indexed(245),
            success: Color::Indexed(115),
            warning: Color::Indexed(222),
            error: Color::Indexed(210),
        }
    }
    pub const fn light() -> Self {
        Self {
            background: Color::Rgb(248, 250, 252),
            text: Color::Rgb(28, 38, 50),
            muted: Color::Rgb(61, 77, 95),
            line: Color::Rgb(104, 120, 137),
            focus: Color::Rgb(0, 86, 112),
            success: Color::Rgb(24, 104, 62),
            warning: Color::Rgb(135, 79, 0),
            error: Color::Rgb(172, 37, 44),
        }
    }
    pub fn base(self) -> Style {
        Style::default().bg(self.background).fg(self.text)
    }
    pub fn strong(self) -> Style {
        self.base().add_modifier(Modifier::BOLD)
    }
    pub fn focus(self) -> Style {
        self.strong().fg(self.focus)
    }
    pub fn muted(self) -> Style {
        self.base().fg(self.muted)
    }
    pub fn selected(self) -> Style {
        // Reverse the chosen foreground/background pair, not an arbitrary
        // low-contrast RGB fill. The explicit > marker carries focus too.
        self.strong().add_modifier(Modifier::REVERSED)
    }
}
