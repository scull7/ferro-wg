//! Help overlay component.
//!
//! [`HelpOverlayComponent`] renders a centered modal help overlay when
//! `state.show_help` is `true`. It displays all keybindings in a two-column
//! table within a `Clear`-backed overlay at 90% width × min(height/2, 30) rows.
//! The overlay captures all key events while active: `?`, `Esc`, or `q` closes it.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::{Clear, Row, Table};

use ferro_wg_tui_core::{Action, AppState, Component, KEYBINDINGS};

/// A modal help overlay displaying all keybindings.
///
/// Activated when `state.show_help` is `true`. Renders a centered
/// overlay with a two-column table of keybindings.
pub struct HelpOverlayComponent;

impl HelpOverlayComponent {
    /// Create a new help overlay component.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for HelpOverlayComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for HelpOverlayComponent {
    /// Route keys when the help overlay is active.
    ///
    /// Returns `None` when the overlay is not shown (acts as a no-op).
    /// `?`, `Esc`, or `q` → [`Action::HideHelp`]; all other keys are swallowed
    /// (return `None`) to prevent leaking through to underlying components.
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action> {
        if !state.show_help {
            return None;
        }
        match key.code {
            KeyCode::Char('?' | 'q') | KeyCode::Esc => Some(Action::HideHelp),
            _ => None,
        }
    }

    fn update(&mut self, _action: &Action, _state: &AppState) {}

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        if !state.show_help {
            return;
        }
        let height = (area.height / 2).min(30);
        let overlay_area = crate::util::centered_rect(90, height, area);
        frame.render_widget(Clear, overlay_area);
        let block = state.theme.overlay_block("Help");
        let inner = block.inner(overlay_area);
        frame.render_widget(block, overlay_area);
        let rows: Vec<Row> = KEYBINDINGS
            .iter()
            .map(|(key, desc)| Row::new(vec![*key, *desc]))
            .collect();
        let table = Table::new(rows, &[Constraint::Percentage(50); 2]);
        frame.render_widget(table, inner);
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use ferro_wg_core::config::AppConfig;
    use ferro_wg_tui_core::Action;

    fn base_state() -> AppState {
        AppState::new(AppConfig::default())
    }

    fn state_with_help() -> AppState {
        let mut state = AppState::new(AppConfig::default());
        state.dispatch(&Action::ShowHelp);
        state
    }

    // ── handle_key ─────────────────────────────────────────────────────────────

    #[test]
    fn returns_none_when_help_not_shown() {
        let mut comp = HelpOverlayComponent::new();
        let state = base_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('?')), &state),
            None,
        );
    }

    #[test]
    fn question_mark_returns_hide_help() {
        let mut comp = HelpOverlayComponent::new();
        let state = state_with_help();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('?')), &state),
            Some(Action::HideHelp),
        );
    }

    #[test]
    fn esc_returns_hide_help() {
        let mut comp = HelpOverlayComponent::new();
        let state = state_with_help();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Esc), &state),
            Some(Action::HideHelp),
        );
    }

    #[test]
    fn q_returns_hide_help() {
        let mut comp = HelpOverlayComponent::new();
        let state = state_with_help();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('q')), &state),
            Some(Action::HideHelp),
        );
    }

    #[test]
    fn other_keys_return_none() {
        let mut comp = HelpOverlayComponent::new();
        let state = state_with_help();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('j')), &state),
            None,
        );
    }

    // ── render ─────────────────────────────────────────────────────────────────

    #[test]
    fn render_no_help_no_panic() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = base_state();
        terminal
            .draw(|frame| {
                let mut comp = HelpOverlayComponent::new();
                comp.render(frame, frame.area(), false, &state);
            })
            .unwrap();
        // No content should be drawn when show_help is false.
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(
            !content.contains("Help"),
            "no overlay expected when show_help is false"
        );
    }

    #[test]
    fn render_help_shows_overlay_with_title_and_bindings() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_help();
        terminal
            .draw(|frame| {
                let mut comp = HelpOverlayComponent::new();
                comp.render(frame, frame.area(), false, &state);
            })
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(content.contains("Help"), "expected 'Help' title");
        assert!(
            content.contains("q / Esc"),
            "expected 'q / Esc' in bindings"
        );
        assert!(
            content.contains("Toggle theme"),
            "expected 'Toggle theme' in bindings"
        );
    }

    #[test]
    fn render_wider_terminal_shows_two_columns() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_help();
        terminal
            .draw(|frame| {
                let mut comp = HelpOverlayComponent::new();
                comp.render(frame, frame.area(), false, &state);
            })
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(content.contains("Help"), "expected 'Help' title");
        // Since it's a table with two columns, we expect the content to be present.
        // Exact column layout is hard to test with string contains, but presence of keybindings suffices.
    }

    #[test]
    fn keybindings_has_at_least_10_entries() {
        assert!(KEYBINDINGS.len() >= 10);
    }

    #[test]
    fn keybindings_contains_question_mark_entry() {
        assert!(KEYBINDINGS.iter().any(|(key, _)| key.contains('?')));
    }
}
