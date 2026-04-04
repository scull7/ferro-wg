use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tracing::warn;

use ferro_wg_tui_core::{Action, AppState, Component};

/// Logs tab: scrollable log viewer with parsed log lines.
pub struct LogsComponent;

impl LogsComponent {
    /// Parse a log line into styled spans for display.
    ///
    /// Expects lines in the format emitted by [`LogLayer`](ferro_wg_core::daemon::LogLayer):
    /// `HH:MM:SS LEVEL target: message`.
    ///
    /// - `show_timestamps`: include the `[HH:MM:SS]` prefix span.
    /// - `color_badges`: apply color to the `[LEVEL]` span; plain text otherwise.
    ///
    /// Falls back to a single plain-text span for legacy or malformed lines.
    #[must_use]
    pub fn parse_log_line(
        line: &str,
        show_timestamps: bool,
        color_badges: bool,
    ) -> Vec<Span<'static>> {
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

        // Expected format: `HH:MM:SS LEVEL target: message`
        // Detect timestamp by positions: digits at 0-1, ':' at 2, digits at 3-4,
        // ':' at 5, digits at 6-7, ' ' at 8.
        let has_timestamp = line.len() >= 9
            && line.as_bytes().get(2) == Some(&b':')
            && line.as_bytes().get(5) == Some(&b':')
            && line.as_bytes().get(8) == Some(&b' ')
            && line[0..8].chars().all(|c| c.is_ascii_digit() || c == ':');

        if has_timestamp {
            let timestamp = &line[0..8];
            let after_timestamp = &line[9..];
            if let Some(space_pos) = after_timestamp.find(' ') {
                let level = &after_timestamp[0..space_pos];
                if let Some(color) = level_color(level) {
                    let message = after_timestamp[space_pos + 1..].to_owned();
                    let level_owned = level.to_owned();
                    let timestamp_owned = timestamp.to_owned();

                    let mut spans: Vec<Span<'static>> = Vec::with_capacity(5);
                    if show_timestamps {
                        spans.push(Span::styled(
                            format!("[{timestamp_owned}]"),
                            Style::default().fg(Color::Cyan),
                        ));
                        spans.push(Span::raw(" "));
                    }
                    let level_span = if color_badges {
                        Span::styled(format!("[{level_owned}]"), Style::default().fg(color))
                    } else {
                        Span::raw(format!("[{level_owned}]"))
                    };
                    spans.push(level_span);
                    spans.push(Span::raw(" "));
                    spans.push(Span::raw(message));
                    return spans;
                }
            }
        }

        // Fallback for legacy or malformed lines.
        vec![Span::raw(line.to_owned())]
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
        let show_timestamps = state.log_display.show_timestamps;
        let color_badges = state.log_display.color_badges;

        let Ok(log_lines) = state.log_lines.lock() else {
            warn!("log_lines mutex poisoned, skipping render");
            return;
        };

        let lines: Vec<Line<'_>> = if log_lines.is_empty() {
            vec![Line::from(Span::styled(
                "(no log entries yet)",
                Style::default().fg(theme.muted),
            ))]
        } else {
            log_lines
                .iter()
                .map(|l| Line::from(Self::parse_log_line(l, show_timestamps, color_badges)))
                .collect()
        };

        let paragraph = Paragraph::new(lines).block(theme.panel_block("Logs"));
        frame.render_widget(paragraph, area);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;

    #[test]
    fn parse_log_line_with_timestamp_and_level() {
        let spans = LogsComponent::parse_log_line(
            "12:34:56 INFO ferro_wg_core::tunnel::mod: Connection abc is up",
            true,
            true,
        );

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
    fn parse_log_line_error_level() {
        let spans = LogsComponent::parse_log_line(
            "12:34:56 ERROR ferro_wg_core::error: Failed to connect",
            true,
            true,
        );
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content, "[ERROR]");
        assert_eq!(spans[2].style.fg, Some(Color::Red));
    }

    #[test]
    fn parse_log_line_warn_level() {
        let spans = LogsComponent::parse_log_line(
            "12:34:56 WARN ferro_wg_core::tunnel: Handshake timeout",
            true,
            true,
        );
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content, "[WARN]");
        assert_eq!(spans[2].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn parse_log_line_debug_level() {
        let spans = LogsComponent::parse_log_line(
            "12:34:56 DEBUG ferro_wg_core::stats: Packet count: 42",
            true,
            true,
        );
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content, "[DEBUG]");
        assert_eq!(spans[2].style.fg, Some(Color::Blue));
    }

    #[test]
    fn parse_log_line_no_timestamp_hides_prefix() {
        let spans =
            LogsComponent::parse_log_line("12:34:56 INFO ferro_wg_core::tunnel: msg", false, true);
        // No timestamp + space → 3 spans: [LEVEL], space, message.
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "[INFO]");
        assert_eq!(spans[0].style.fg, Some(Color::Green));
    }

    #[test]
    fn parse_log_line_no_color_plain_level_badge() {
        let spans =
            LogsComponent::parse_log_line("12:34:56 INFO ferro_wg_core::tunnel: msg", true, false);
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content, "[INFO]");
        // No color applied.
        assert_eq!(spans[2].style.fg, None);
    }

    #[test]
    fn parse_log_line_legacy_format() {
        let line = "INFO ferro_wg_core::tunnel::mod: Connection abc is up";
        let spans = LogsComponent::parse_log_line(line, true, true);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, line);
    }

    #[test]
    fn parse_log_line_malformed() {
        let line = "some random log message";
        let spans = LogsComponent::parse_log_line(line, true, true);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, line);
    }
}
