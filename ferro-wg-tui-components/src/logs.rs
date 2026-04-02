//! Logs tab: scrollable log viewer.

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use ferro_wg_tui_core::{Action, AppState, Component};

/// Live log viewer displaying daemon output.
///
/// Currently a placeholder that shows "(no log entries yet)" when
/// empty. Log streaming will be implemented in Phase 3.
pub struct LogsComponent;

impl LogsComponent {
    /// Create a new logs component.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for LogsComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for LogsComponent {
    fn handle_key(&mut self, _key: KeyEvent, _state: &AppState) -> Option<Action> {
        // No interactive elements yet (future: scroll, filter).
        None
    }

    fn update(&mut self, _action: &Action, _state: &AppState) {
        // No local state to update.
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        let theme = &state.theme;

        let lines: Vec<Line<'_>> = if state.log_lines.is_empty() {
            vec![Line::from(Span::styled(
                "(no log entries yet)",
                Style::default().fg(theme.muted),
            ))]
        } else {
            state
                .log_lines
                .iter()
                .map(|l| Line::from(l.as_str()))
                .collect()
        };

        let paragraph = Paragraph::new(lines).block(theme.panel_block("Logs"));
        frame.render_widget(paragraph, area);
    }
}
