use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use ferro_wg_tui_core::{Action, AppState, Component};

/// Logs tab: scrollable log viewer with parsed log lines.
pub struct LogsComponent;

impl LogsComponent {
    /// Parse a log line into styled spans for display.
    ///
    /// Supports timestamped logs with colored level badges.
    /// Falls back to plain text for legacy or malformed lines.
    #[must_use]
    pub fn parse_log_line(line: &str) -> Vec<Span<'static>> {
        /// Get the color for a log level.
        fn level_color(level: &str) -> Option<Color> {
            match level {
                "TRACE" => Some(Color::Gray),
                "DEBUG" => Some(Color::Blue),
                "INFO" => Some(Color::Green),
                "WARN" => Some(Color::Yellow),
                "ERROR" => Some(Color::Red),
                _ => None,
            }
        }

        let line = line.trim();
        if line.len() >= 9
            && line.as_bytes().get(2) == Some(&b':')
            && line.as_bytes().get(5) == Some(&b':')
            && line.as_bytes().get(8) == Some(&b' ')
        {
            let timestamp = &line[0..8];
            if timestamp.chars().all(|c| c.is_ascii_digit() || c == ':') {
                let after_timestamp = &line[9..];
                if let Some(space_pos) = after_timestamp.find(' ') {
                    let level = &after_timestamp[0..space_pos];
                    if let Some(color) = level_color(level) {
                        let message = &after_timestamp[space_pos + 1..];
                        return vec![
                            Span::styled(
                                format!("[{timestamp}]"),
                                Style::default().fg(Color::Cyan),
                            ),
                            Span::raw(" "),
                            Span::styled(format!("[{level}]"), Style::default().fg(color)),
                            Span::raw(" "),
                            Span::raw(message.to_string()),
                        ];
                    }
                }
            }
        }
        // Fallback for legacy or malformed lines
        vec![Span::raw(line.to_string())]
    }

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

        let log_lines = state.log_lines.lock().expect("Failed to lock log_lines");
        let lines: Vec<Line<'_>> = if log_lines.is_empty() {
            vec![Line::from(Span::styled(
                "(no log entries yet)",
                Style::default().fg(theme.muted),
            ))]
        } else {
            log_lines
                .iter()
                .map(|l| Line::from(Self::parse_log_line(l)))
                .collect()
        };

        let paragraph = Paragraph::new(lines).block(theme.panel_block("Logs"));
        frame.render_widget(paragraph, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn test_parse_log_line_with_timestamp_and_level() {
        let line = "12:34:56 INFO ferro_wg_core::tunnel::mod: Connection abc is up";
        let spans = LogsComponent::parse_log_line(line);

        assert_eq!(spans.len(), 5);
        assert_eq!(spans[0].content, "[12:34:56]");
        assert_eq!(spans[0].style.fg, Some(Color::Cyan));
        assert_eq!(spans[2].content, "[INFO]");
        assert_eq!(spans[2].style.fg, Some(Color::Green));
        assert_eq!(
            spans[4].content,
            "ferro_wg_core::tunnel::mod: Connection abc is up"
        );
    }

    #[test]
    fn test_parse_log_line_error_level() {
        let line = "12:34:56 ERROR ferro_wg_core::error: Failed to connect";
        let spans = LogsComponent::parse_log_line(line);

        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content, "[ERROR]");
        assert_eq!(spans[2].style.fg, Some(Color::Red));
    }

    #[test]
    fn test_parse_log_line_warn_level() {
        let line = "12:34:56 WARN ferro_wg_core::tunnel: Handshake timeout";
        let spans = LogsComponent::parse_log_line(line);

        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content, "[WARN]");
        assert_eq!(spans[2].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn test_parse_log_line_debug_level() {
        let line = "12:34:56 DEBUG ferro_wg_core::stats: Packet count: 42";
        let spans = LogsComponent::parse_log_line(line);

        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content, "[DEBUG]");
        assert_eq!(spans[2].style.fg, Some(Color::Blue));
    }

    #[test]
    fn test_parse_log_line_legacy_format() {
        let line = "INFO ferro_wg_core::tunnel::mod: Connection abc is up";
        let spans = LogsComponent::parse_log_line(line);

        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, line);
    }

    #[test]
    fn test_parse_log_line_malformed() {
        let line = "some random log message";
        let spans = LogsComponent::parse_log_line(line);

        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, line);
    }
}
