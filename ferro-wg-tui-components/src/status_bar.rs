//! Status bar: bottom-of-screen help text or search input.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use ferro_wg_tui_core::{Action, AppState, Component, InputMode, Tab};

/// Number of rows this component occupies in the layout.
///
/// One row for the [`Borders::ALL`] top border, one for the content line, and
/// one for the bottom border.
pub const STATUS_BAR_HEIGHT: u16 = 3;

/// Bottom-of-screen bar that displays help text in normal mode or a
/// search input field in search mode.
///
/// In search mode, key events are routed here and converted to search
/// actions (`SearchInput`, `SearchBackspace`, `ExitSearch`).
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
            KeyCode::Esc | KeyCode::Enter => Some(Action::ExitSearch),
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

        let content = if let Some(fb) = &state.feedback {
            // Show feedback message (success or error).
            let (indicator, style) = if fb.is_error {
                ("x ", Style::default().fg(theme.error))
            } else {
                ("* ", Style::default().fg(theme.success))
            };
            let daemon_dot = daemon_indicator(state, theme);
            Line::from(vec![
                daemon_dot,
                Span::styled(indicator, style),
                Span::styled(&fb.message, style),
            ])
        } else {
            match state.input_mode {
                InputMode::Search => Line::from(vec![
                    Span::styled(" /", Style::default().fg(theme.warning)),
                    Span::raw(&state.search_query),
                    Span::styled("_", Style::default().fg(theme.muted)),
                ]),
                InputMode::Normal => {
                    let hotkey = theme.hotkey_style();
                    let daemon_dot = daemon_indicator(state, theme);
                    let mut spans = vec![
                        daemon_dot,
                        Span::styled("q", hotkey),
                        Span::raw(" quit  "),
                        Span::styled("/", hotkey),
                        Span::raw(" search  "),
                    ];
                    if state.active_tab == Tab::Status {
                        spans.extend([
                            Span::styled("u", hotkey),
                            Span::raw(" up  "),
                            Span::styled("d", hotkey),
                            Span::raw(" down  "),
                            Span::styled("b", hotkey),
                            Span::raw(" backend  "),
                        ]);
                    }
                    spans.extend([Span::styled("j/k", hotkey), Span::raw(" nav")]);
                    Line::from(spans)
                }
            }
        };

        let block = Block::default().borders(Borders::ALL);
        let paragraph = Paragraph::new(content).block(block);
        frame.render_widget(paragraph, area);
    }
}

/// Create a daemon connectivity indicator span.
fn daemon_indicator<'a>(state: &AppState, theme: &ferro_wg_tui_core::Theme) -> Span<'a> {
    if state.daemon_connected {
        Span::styled(" [*] ", Style::default().fg(theme.success))
    } else {
        Span::styled(" [o] ", Style::default().fg(theme.muted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_wg_core::config::AppConfig;

    fn search_state() -> AppState {
        let mut state = AppState::new(AppConfig::default());
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
    fn emits_exit_on_esc() {
        let mut comp = StatusBarComponent::new();
        let state = search_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Esc), &state),
            Some(Action::ExitSearch)
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
        let state = AppState::new(AppConfig::default());
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('a')), &state),
            None
        );
    }
}
