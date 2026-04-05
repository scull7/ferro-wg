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
///
/// In import mode, key events are routed here and converted to import
/// actions (`ImportKey`, `SubmitImport`, `ExitImport`).
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
        match &state.input_mode {
            InputMode::Search => match key.code {
                KeyCode::Esc | KeyCode::Enter => Some(Action::ExitSearch),
                KeyCode::Backspace => Some(Action::SearchBackspace),
                KeyCode::Char(c) => Some(Action::SearchInput(c)),
                _ => None,
            },
            InputMode::Import(_) => match key.code {
                KeyCode::Esc => Some(Action::ExitImport),
                KeyCode::Enter => Some(Action::SubmitImport),
                _ => Some(Action::ImportKey(key)),
            },
            InputMode::Normal => None,
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
            match &state.input_mode {
                InputMode::Search => Line::from(vec![
                    Span::styled(" /", Style::default().fg(theme.warning)),
                    Span::raw(state.search_query.clone()),
                    Span::styled("_", Style::default().fg(theme.muted)),
                ]),
                InputMode::Import(buf) => Line::from(vec![
                    Span::styled(" Import path: ", Style::default().fg(theme.warning)),
                    Span::raw(buf.clone()),
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
                        Span::styled("i", hotkey),
                        Span::raw(" import  "),
                    ];
                    match state.active_tab {
                        Tab::Overview => {
                            spans.extend([
                                Span::styled("u", hotkey),
                                Span::raw(" up-all  "),
                                Span::styled("d", hotkey),
                                Span::raw(" down-all  "),
                            ]);
                            if state.daemon_connected {
                                spans.extend([Span::styled("S", hotkey), Span::raw(" stop  ")]);
                            } else {
                                spans.extend([Span::styled("s", hotkey), Span::raw(" start  ")]);
                            }
                        }
                        Tab::Status => spans.extend([
                            Span::styled("u", hotkey),
                            Span::raw(" up  "),
                            Span::styled("d", hotkey),
                            Span::raw(" down  "),
                            Span::styled("b", hotkey),
                            Span::raw(" backend  "),
                        ]),
                        _ => {}
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

    fn import_state() -> AppState {
        let mut state = AppState::new(AppConfig::default());
        state.dispatch(&Action::EnterImport);
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

    #[test]
    fn import_esc_emits_exit_import() {
        let mut comp = StatusBarComponent::new();
        let state = import_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Esc), &state),
            Some(Action::ExitImport)
        );
    }

    #[test]
    fn import_enter_emits_submit_import() {
        let mut comp = StatusBarComponent::new();
        let state = import_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Enter), &state),
            Some(Action::SubmitImport)
        );
    }

    #[test]
    fn import_char_emits_import_key() {
        let mut comp = StatusBarComponent::new();
        let state = import_state();
        let key = KeyEvent::from(KeyCode::Char('a'));
        assert_eq!(comp.handle_key(key, &state), Some(Action::ImportKey(key)));
    }
}
