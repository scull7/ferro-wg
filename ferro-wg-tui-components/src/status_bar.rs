//! Status bar: bottom-of-screen help text or search input.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use ferro_wg_tui_core::{Action, AppState, Component, InputMode};

/// Bottom-of-screen bar that displays help text in normal mode or a
/// search input field in search mode.
///
/// In search mode, key events are routed here and converted to search
/// actions (`SearchInput`, `SearchBackspace`, `ExitSearch`,
/// `ClearSearch`).
pub struct StatusBarComponent;

impl StatusBarComponent {
    /// Create a new status bar component.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for StatusBarComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for StatusBarComponent {
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action> {
        if state.input_mode != InputMode::Search {
            return None;
        }

        match key.code {
            KeyCode::Esc => Some(Action::ClearSearch),
            KeyCode::Enter => Some(Action::ExitSearch),
            KeyCode::Backspace => Some(Action::SearchBackspace),
            KeyCode::Char(c) => Some(Action::SearchInput(c)),
            _ => None,
        }
    }

    fn update(&mut self, _action: &Action, _state: &AppState) {
        // No local state.
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        let theme = &state.theme;

        let content = match state.input_mode {
            InputMode::Search => Line::from(vec![
                Span::styled(" /", Style::default().fg(theme.warning)),
                Span::raw(&state.search_query),
                Span::styled("_", Style::default().fg(theme.muted)),
            ]),
            InputMode::Normal => {
                let hotkey = theme.hotkey_style();
                Line::from(vec![
                    Span::styled(" q", hotkey),
                    Span::raw(" quit  "),
                    Span::styled("/", hotkey),
                    Span::raw(" search  "),
                    Span::styled("1-5", hotkey),
                    Span::raw(" tabs  "),
                    Span::styled("j/k", hotkey),
                    Span::raw(" navigate"),
                ])
            }
        };

        let block = Block::default().borders(Borders::ALL);
        let paragraph = Paragraph::new(content).block(block);
        frame.render_widget(paragraph, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_wg_core::config::{InterfaceConfig, WgConfig};
    use ferro_wg_core::key::PrivateKey;

    fn search_state() -> AppState {
        let mut state = AppState::new(WgConfig {
            interface: InterfaceConfig {
                private_key: PrivateKey::generate(),
                listen_port: 51820,
                addresses: Vec::new(),
                dns: Vec::new(),
                mtu: 1420,
                fwmark: 0,
                pre_up: Vec::new(),
                post_up: Vec::new(),
                pre_down: Vec::new(),
                post_down: Vec::new(),
            },
            peers: Vec::new(),
        });
        state.dispatch(&Action::EnterSearch);
        state
    }

    #[test]
    fn emits_search_input() {
        let mut comp = StatusBarComponent::new();
        let state = search_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('a')), &state),
            Some(Action::SearchInput('a'))
        );
    }

    #[test]
    fn emits_backspace() {
        let mut comp = StatusBarComponent::new();
        let state = search_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Backspace), &state),
            Some(Action::SearchBackspace)
        );
    }

    #[test]
    fn emits_clear_on_esc() {
        let mut comp = StatusBarComponent::new();
        let state = search_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Esc), &state),
            Some(Action::ClearSearch)
        );
    }

    #[test]
    fn emits_exit_on_enter() {
        let mut comp = StatusBarComponent::new();
        let state = search_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Enter), &state),
            Some(Action::ExitSearch)
        );
    }

    #[test]
    fn ignores_keys_in_normal_mode() {
        let mut comp = StatusBarComponent::new();
        let state = AppState::new(WgConfig {
            interface: InterfaceConfig {
                private_key: PrivateKey::generate(),
                listen_port: 51820,
                addresses: Vec::new(),
                dns: Vec::new(),
                mtu: 1420,
                fwmark: 0,
                pre_up: Vec::new(),
                post_up: Vec::new(),
                pre_down: Vec::new(),
                post_down: Vec::new(),
            },
            peers: Vec::new(),
        });
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('a')), &state),
            None
        );
    }
}
