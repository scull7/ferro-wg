//! Logs tab: scrollable log viewer.

use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use ferro_wg_tui_core::{Action, AppState, Component};

/// Live log viewer displaying daemon output.
///
/// Shows real-time daemon logs as they are emitted. Displays
/// "(no log entries yet)" when empty.
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

        let log_lines = match state.log_lines.lock() {
            Ok(guard) => guard,
            Err(_) => {
                warn!("Log buffer mutex poisoned, showing empty logs");
                return;
            }
        };
        let lines: Vec<Line> = if log_lines.is_empty() {
            vec![Line::from(Span::styled(
                "(no log entries yet)",
                Style::default().fg(theme.muted),
            ))]
        } else {
            log_lines.iter().map(|l| Line::from(l.clone())).collect()
        };
        drop(log_lines); // Release lock

        let paragraph = Paragraph::new(lines).block(theme.panel_block("Logs"));
        frame.render_widget(paragraph, area);
    }
}
