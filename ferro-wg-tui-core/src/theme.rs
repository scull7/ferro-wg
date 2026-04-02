//! Theme system with Catppuccin Mocha (dark) and Latte (light) palettes.
//!
//! Every styled element in the TUI references a [`Theme`] field — no
//! hardcoded colors elsewhere. Phase 0 maps to the same `Color::*`
//! constants used in the original code; true Catppuccin hex values
//! will be introduced in Phase 7.

use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders};

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
    ///
    /// Phase 0: maps to the `Color::*` constants from the original TUI
    /// so visual output is identical.
    #[must_use]
    pub fn mocha() -> Self {
        Self {
            base: Color::Reset,
            surface: Color::Reset,
            text: Color::Reset,
            subtext: Color::Reset,
            accent: Color::Cyan,
            success: Color::Green,
            error: Color::Red,
            warning: Color::Yellow,
            muted: Color::DarkGray,
            highlight_bg: Color::DarkGray,
        }
    }

    /// Catppuccin Latte (light) palette.
    #[must_use]
    pub fn latte() -> Self {
        Self {
            base: Color::Reset,
            surface: Color::Reset,
            text: Color::Reset,
            subtext: Color::Reset,
            accent: Color::Blue,
            success: Color::Green,
            error: Color::Red,
            warning: Color::Yellow,
            muted: Color::Gray,
            highlight_bg: Color::Gray,
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
        assert_eq!(theme.accent, Color::Cyan);
        assert_eq!(theme.success, Color::Green);
        assert_eq!(theme.muted, Color::DarkGray);
        assert_eq!(theme.highlight_bg, Color::DarkGray);
        assert_eq!(theme.warning, Color::Yellow);
        assert_eq!(theme.error, Color::Red);
    }

    #[test]
    fn header_style_is_accent_bold() {
        let theme = Theme::mocha();
        let style = theme.header_style();
        assert_eq!(style.fg, Some(Color::Cyan));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }
}
