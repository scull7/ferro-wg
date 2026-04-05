//! Status bar: bottom-of-screen help text or search input.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use ferro_wg_tui_core::state::CompareView;
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
            InputMode::Export(_) => match key.code {
                KeyCode::Esc => Some(Action::ExitExport),
                KeyCode::Enter => Some(Action::SubmitExport),
                _ => Some(Action::ExportKey(key)),
            },
            InputMode::Normal | InputMode::EditField => None,
        }
    }

    fn update(&mut self, _action: &Action, _state: &AppState) {
        // No local state.
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        let theme = &state.theme;
        let content = build_status_line(state, theme);
        let block = Block::default().borders(Borders::ALL);
        frame.render_widget(Paragraph::new(content).block(block), area);
    }
}

/// Build the status line content for the current app state.
fn build_status_line<'a>(state: &'a AppState, theme: &ferro_wg_tui_core::Theme) -> Line<'a> {
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
        InputMode::Export(buf) => Line::from(vec![
            Span::styled(" Export path: ", Style::default().fg(theme.warning)),
            Span::raw(buf.clone()),
            Span::styled("_", Style::default().fg(theme.muted)),
        ]),
        InputMode::Normal | InputMode::EditField => build_normal_hints(state, theme),
    }
}

/// Build the hint spans for normal/edit-field mode.
fn build_normal_hints<'a>(state: &AppState, theme: &ferro_wg_tui_core::Theme) -> Line<'a> {
    let hotkey = theme.hotkey_style();
    let mut spans = vec![
        daemon_indicator(state, theme),
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
        Tab::Compare => match state.compare_view {
            CompareView::Live => spans.extend([
                Span::styled("b", hotkey),
                Span::raw(" benchmark  "),
                Span::styled("Enter", hotkey),
                Span::raw(" run selected  "),
                Span::styled("w", hotkey),
                Span::raw(" use backend  "),
                Span::styled("h", hotkey),
                Span::raw(" history  "),
                Span::styled("e", hotkey),
                Span::raw(" export  "),
            ]),
            CompareView::Historical => spans.extend([
                Span::styled("h", hotkey),
                Span::raw(" live view  "),
                Span::styled("e", hotkey),
                Span::raw(" export  "),
            ]),
        },
        Tab::Config => spans.extend(config_hints(state, hotkey)),
        _ => {}
    }
    spans.extend([Span::styled("j/k", hotkey), Span::raw(" nav")]);
    Line::from(spans)
}

/// Build config-tab hint spans based on edit state.
fn config_hints<'a>(state: &AppState, hotkey: ratatui::style::Style) -> Vec<Span<'a>> {
    let editing = state
        .config_edit
        .as_ref()
        .is_some_and(|e| e.edit_buffer.is_some());
    if editing {
        vec![
            Span::styled("Enter", hotkey),
            Span::raw(" confirm  "),
            Span::styled("Esc", hotkey),
            Span::raw(" cancel  "),
            Span::raw("(type to edit)"),
        ]
    } else {
        vec![
            Span::styled("e", hotkey),
            Span::raw(" edit  "),
            Span::styled("j/k", hotkey),
            Span::raw(" nav  "),
            Span::styled("p", hotkey),
            Span::raw(" preview  "),
            Span::styled("+", hotkey),
            Span::raw(" add peer  "),
            Span::styled("x", hotkey),
            Span::raw(" delete peer"),
        ]
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
