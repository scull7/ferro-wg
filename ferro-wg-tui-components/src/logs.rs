use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tracing::warn;

use ferro_wg_core::config::LogDisplayConfig;
use ferro_wg_tui_core::{Action, AppState, Component};

/// Errors that can occur when parsing a structured log line.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum LogParseError {
    /// The line does not begin with a valid `HH:MM:SS ` timestamp prefix.
    #[error("line does not begin with a valid HH:MM:SS timestamp")]
    MalformedTimestamp,
    /// The token following the timestamp is not a recognised tracing level.
    #[error("unknown log level: {0:?}")]
    UnknownLevel(String),
}

/// Logs tab: scrollable log viewer with parsed log lines.
pub struct LogsComponent;

impl LogsComponent {
    /// Extract the `HH:MM:SS` timestamp from the start of a log line.
    ///
    /// Returns `Some(timestamp)` when the line begins with the pattern
    /// `HH:MM:SS ` (digits, colons, trailing space), `None` otherwise.
    #[must_use]
    pub fn parse_timestamp(line: &str) -> Option<&str> {
        // Byte indexing is safe and correct here: the timestamp is always ASCII,
        // so each byte corresponds to exactly one character, and matching ASCII
        // literals rules out any multi-byte UTF-8 continuation bytes.
        let bytes = line.as_bytes();
        let valid = bytes.len() >= 9
            && bytes[2] == b':'
            && bytes[5] == b':'
            && bytes[8] == b' '
            && bytes[0..8].iter().all(|&b| b.is_ascii_digit() || b == b':');
        if valid { Some(&line[0..8]) } else { None }
    }

    /// Return the [`Style`] to apply to a level badge.
    ///
    /// When `color_badges` is `false` the returned style is unstyled (default).
    #[must_use]
    pub fn level_style(level: &str, color_badges: bool) -> Style {
        if !color_badges {
            return Style::default();
        }
        let color = match level {
            "TRACE" => Color::Gray,
            "DEBUG" => Color::Blue,
            "INFO" => Color::Green,
            "WARN" => Color::Yellow,
            "ERROR" => Color::Red,
            _ => return Style::default(),
        };
        Style::default().fg(color)
    }

    /// Split `after_timestamp` (the portion of a log line after `HH:MM:SS `) into
    /// `(level, message)`.
    ///
    /// # Errors
    ///
    /// Returns [`LogParseError::UnknownLevel`] when the token before the first
    /// space is not a recognised tracing level, or when there is no space at all.
    fn extract_level_message(after_timestamp: &str) -> Result<(&str, &str), LogParseError> {
        let space_pos = after_timestamp
            .find(' ')
            .ok_or_else(|| LogParseError::UnknownLevel(after_timestamp.to_owned()))?;
        let level = &after_timestamp[..space_pos];
        if !matches!(level, "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR") {
            return Err(LogParseError::UnknownLevel(level.to_owned()));
        }
        Ok((level, &after_timestamp[space_pos + 1..]))
    }

    /// Parse a log line into styled spans for display.
    ///
    /// Expects lines in the format emitted by [`LogLayer`](ferro_wg_core::daemon::LogLayer):
    /// `HH:MM:SS LEVEL target: message`.
    ///
    /// Display behaviour is controlled by `config`:
    /// - `show_timestamps`: include the `[HH:MM:SS]` prefix span.
    /// - `color_badges`: apply color to the `[LEVEL]` span; plain text otherwise.
    ///
    /// # Errors
    ///
    /// Returns [`LogParseError::MalformedTimestamp`] when the line does not begin
    /// with `HH:MM:SS `, or [`LogParseError::UnknownLevel`] when the level token
    /// is not a recognised tracing level. Callers should fall back to a plain-text
    /// span on error.
    pub fn parse_log_line(
        line: &str,
        config: &LogDisplayConfig,
    ) -> Result<Vec<Span<'static>>, LogParseError> {
        let line = line.trim();

        let timestamp = Self::parse_timestamp(line).ok_or(LogParseError::MalformedTimestamp)?;
        let (level, message) = Self::extract_level_message(&line[9..])?;

        let mut spans: Vec<Span<'static>> = Vec::with_capacity(5);
        if config.show_timestamps {
            spans.push(Span::styled(
                format!("[{timestamp}]"),
                Style::default().fg(Color::Cyan),
            ));
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            format!("[{level}]"),
            Self::level_style(level, config.color_badges),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::raw(message.to_owned()));
        Ok(spans)
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
        let config = &state.log_display;

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
                .map(|l| {
                    let spans = Self::parse_log_line(l, config)
                        .unwrap_or_else(|_| vec![Span::raw(l.clone())]);
                    Line::from(spans)
                })
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

    fn cfg(show_timestamps: bool, color_badges: bool) -> LogDisplayConfig {
        LogDisplayConfig {
            show_timestamps,
            color_badges,
        }
    }

    // --- parse_timestamp ---

    #[test]
    fn parse_timestamp_valid() {
        assert_eq!(
            LogsComponent::parse_timestamp("12:34:56 INFO target: msg"),
            Some("12:34:56")
        );
    }

    #[test]
    fn parse_timestamp_no_match() {
        assert_eq!(LogsComponent::parse_timestamp("INFO target: msg"), None);
    }

    // --- level_style ---

    #[test]
    fn level_style_all_known_levels() {
        let cases = [
            ("TRACE", Color::Gray),
            ("DEBUG", Color::Blue),
            ("INFO", Color::Green),
            ("WARN", Color::Yellow),
            ("ERROR", Color::Red),
        ];
        for (level, expected) in cases {
            assert_eq!(
                LogsComponent::level_style(level, true).fg,
                Some(expected),
                "level={level} with color"
            );
            assert_eq!(
                LogsComponent::level_style(level, false).fg,
                None,
                "level={level} without color"
            );
        }
    }

    #[test]
    fn level_style_unknown_level() {
        assert_eq!(LogsComponent::level_style("CUSTOM", true), Style::default());
    }

    // --- parse_log_line: success paths ---

    #[test]
    fn parse_log_line_full_structure() {
        // Verifies all five spans and their content/style for a well-formed line.
        let spans = LogsComponent::parse_log_line(
            "12:34:56 INFO ferro_wg_core::tunnel::mod: Connection abc is up",
            &cfg(true, true),
        )
        .unwrap();

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
    fn parse_log_line_all_known_levels() {
        // Covers every tracing level: badge content and color are correct for each.
        let cases = [
            ("TRACE", Color::Gray),
            ("DEBUG", Color::Blue),
            ("INFO", Color::Green),
            ("WARN", Color::Yellow),
            ("ERROR", Color::Red),
        ];
        for (level, expected_color) in cases {
            let spans = LogsComponent::parse_log_line(
                &format!("12:34:56 {level} target: message"),
                &cfg(true, true),
            )
            .unwrap_or_else(|e| panic!("level={level}: {e}"));
            assert_eq!(spans[2].content, format!("[{level}]"), "level={level}");
            assert_eq!(spans[2].style.fg, Some(expected_color), "level={level}");
        }
    }

    #[test]
    fn parse_log_line_no_timestamp_hides_prefix() {
        let spans = LogsComponent::parse_log_line(
            "12:34:56 INFO ferro_wg_core::tunnel: msg",
            &cfg(false, true),
        )
        .unwrap();
        // No timestamp + space → 3 spans: [LEVEL], space, message.
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "[INFO]");
        assert_eq!(spans[0].style.fg, Some(Color::Green));
    }

    #[test]
    fn parse_log_line_no_color_plain_level_badge() {
        let spans = LogsComponent::parse_log_line(
            "12:34:56 INFO ferro_wg_core::tunnel: msg",
            &cfg(true, false),
        )
        .unwrap();
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content, "[INFO]");
        // No color applied.
        assert_eq!(spans[2].style.fg, None);
    }

    // --- parse_log_line: error paths ---

    #[test]
    fn parse_log_line_malformed_timestamp() {
        // Both legacy format (no timestamp) and fully unstructured lines
        // produce MalformedTimestamp.
        let cases = [
            "INFO ferro_wg_core::tunnel::mod: Connection abc is up",
            "some random log message",
        ];
        for line in cases {
            let err = LogsComponent::parse_log_line(line, &cfg(true, true)).unwrap_err();
            assert_eq!(err, LogParseError::MalformedTimestamp, "line={line:?}");
        }
    }

    #[test]
    fn parse_log_line_unknown_level_errs() {
        let err = LogsComponent::parse_log_line(
            "12:34:56 CUSTOM ferro_wg_core::tunnel: msg",
            &cfg(true, true),
        )
        .unwrap_err();
        assert_eq!(err, LogParseError::UnknownLevel("CUSTOM".to_owned()));
    }
}
