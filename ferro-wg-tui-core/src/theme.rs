//! Theme system with Catppuccin Mocha (dark) and Latte (light) palettes.
//!
//! Every styled element in the TUI references a [`Theme`] field — no
//! hardcoded colors elsewhere. Phase 0 maps to the same `Color::*`
//! constants used in the original code; true Catppuccin hex values
//! will be introduced in Phase 7.

use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders};

/// Theme kind (dark or light).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeKind {
    /// Catppuccin Mocha (dark).
    #[default]
    Mocha,
    /// Catppuccin Latte (light).
    Latte,
}

impl ThemeKind {
    /// Convert to the full theme.
    #[must_use]
    pub fn into_theme(self) -> Theme {
        match self {
            Self::Mocha => Theme::mocha(),
            Self::Latte => Theme::latte(),
        }
    }

    /// Toggle between Mocha and Latte.
    #[must_use]
    pub fn toggle(self) -> Self {
        match self {
            Self::Mocha => Self::Latte,
            Self::Latte => Self::Mocha,
        }
    }
}

/// Semantic color palette for the TUI.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Background base color.
    pub base: Color,
    /// Surface color for elevated elements.
    pub surface: Color,
    /// Primary text color.
    pub text: Color,
    /// Secondary / dimmed text color.
    pub subtext: Color,
    /// Accent color for active elements and highlights.
    pub accent: Color,
    /// Success state (connected, available).
    pub success: Color,
    /// Error state (unavailable, failed).
    pub error: Color,
    /// Warning state (degraded, attention needed).
    pub warning: Color,
    /// Muted / inactive elements.
    pub muted: Color,
    /// Background color for highlighted rows.
    pub highlight_bg: Color,
}

impl Theme {
    /// Catppuccin Mocha (dark) palette.
    #[must_use]
    pub fn mocha() -> Self {
        Self {
            base: Color::Rgb(30, 30, 46),
            surface: Color::Rgb(49, 50, 68),
            text: Color::Rgb(205, 214, 244),
            subtext: Color::Rgb(186, 194, 222),
            accent: Color::Rgb(180, 190, 254),
            success: Color::Rgb(166, 227, 161),
            error: Color::Rgb(243, 139, 168),
            warning: Color::Rgb(249, 226, 175),
            muted: Color::Rgb(108, 112, 134),
            highlight_bg: Color::Rgb(69, 71, 90),
        }
    }

    /// Catppuccin Latte (light) palette.
    #[must_use]
    pub fn latte() -> Self {
        Self {
            base: Color::Rgb(239, 241, 245),
            surface: Color::Rgb(204, 208, 218),
            text: Color::Rgb(76, 79, 105),
            subtext: Color::Rgb(92, 95, 119),
            accent: Color::Rgb(114, 135, 253),
            success: Color::Rgb(64, 160, 43),
            error: Color::Rgb(210, 15, 57),
            warning: Color::Rgb(223, 142, 29),
            muted: Color::Rgb(156, 160, 176),
            highlight_bg: Color::Rgb(220, 224, 232),
        }
    }

    /// Style for table headers (accent + bold).
    #[must_use]
    pub fn header_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for the highlighted / selected row.
    #[must_use]
    pub fn highlight_style(&self) -> Style {
        Style::default()
            .bg(self.highlight_bg)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for the active tab title.
    #[must_use]
    pub fn active_tab_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for inactive tab titles.
    #[must_use]
    pub fn inactive_tab_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    /// Style for hotkey labels in the help bar.
    #[must_use]
    pub fn hotkey_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Create a bordered block for a main panel.
    #[must_use]
    pub fn panel_block<'a>(&self, title: &'a str) -> Block<'a> {
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" {title} "))
    }

    /// Create a bordered block for a modal overlay.
    #[must_use]
    pub fn overlay_block<'a>(&self, title: &'a str) -> Block<'a> {
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" {title} "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mocha_matches_original_colors() {
        let theme = Theme::mocha();
        // These match the hardcoded colors in the original ui.rs.
        assert_eq!(theme.accent, Color::Rgb(180, 190, 254));
        assert_eq!(theme.success, Color::Rgb(166, 227, 161));
        assert_eq!(theme.muted, Color::Rgb(108, 112, 134));
        assert_eq!(theme.highlight_bg, Color::Rgb(69, 71, 90));
        assert_eq!(theme.warning, Color::Rgb(249, 226, 175));
        assert_eq!(theme.error, Color::Rgb(243, 139, 168));
    }

    #[test]
    fn latte_matches_original_colors() {
        let theme = Theme::latte();
        assert_eq!(theme.accent, Color::Rgb(114, 135, 253));
        assert_eq!(theme.success, Color::Rgb(64, 160, 43));
        assert_eq!(theme.muted, Color::Rgb(156, 160, 176));
        assert_eq!(theme.highlight_bg, Color::Rgb(220, 224, 232));
        assert_eq!(theme.warning, Color::Rgb(223, 142, 29));
        assert_eq!(theme.error, Color::Rgb(210, 15, 57));
    }

    #[test]
    fn theme_kind_toggle() {
        assert_eq!(ThemeKind::Mocha.toggle(), ThemeKind::Latte);
        assert_eq!(ThemeKind::Latte.toggle(), ThemeKind::Mocha);
    }

    #[test]
    fn theme_kind_into_theme_mocha_accent() {
        assert_eq!(
            ThemeKind::Mocha.into_theme().accent,
            Color::Rgb(180, 190, 254)
        );
    }

    #[test]
    fn theme_kind_into_theme_latte_accent() {
        assert_eq!(
            ThemeKind::Latte.into_theme().accent,
            Color::Rgb(114, 135, 253)
        );
    }

    #[test]
    fn theme_mocha_base() {
        assert_eq!(Theme::mocha().base, Color::Rgb(30, 30, 46));
    }

    #[test]
    fn theme_latte_base() {
        assert_eq!(Theme::latte().base, Color::Rgb(239, 241, 245));
    }

    #[test]
    fn header_style_is_accent_bold() {
        let theme = Theme::mocha();
        let style = theme.header_style();
        assert_eq!(style.fg, Some(Color::Rgb(180, 190, 254)));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }
}
