use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tracing::warn;

use ferro_wg_core::config::LogDisplayConfig;
use ferro_wg_tui_core::{Action, AppState, Component};

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
    /// `(level, message)`, or return `None` if the level token is not a known tracing level.
    fn extract_level_message(after_timestamp: &str) -> Option<(&str, &str)> {
        let space_pos = after_timestamp.find(' ')?;
        let level = &after_timestamp[..space_pos];
        if !matches!(level, "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR") {
            return None;
        }
        Some((level, &after_timestamp[space_pos + 1..]))
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
    /// Falls back to a single plain-text span for legacy or malformed lines.
    #[must_use]
    pub fn parse_log_line(line: &str, config: &LogDisplayConfig) -> Vec<Span<'static>> {
        let line = line.trim();

        let Some(timestamp) = Self::parse_timestamp(line) else {
            return vec![Span::raw(line.to_owned())];
        };
        let Some((level, message)) = Self::extract_level_message(&line[9..]) else {
            return vec![Span::raw(line.to_owned())];
        };

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
        spans
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
                .map(|l| Line::from(Self::parse_log_line(l, config)))
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
    fn level_style_known_with_color() {
        assert_eq!(
            LogsComponent::level_style("INFO", true).fg,
            Some(Color::Green)
        );
    }

    #[test]
    fn level_style_no_color() {
        assert_eq!(LogsComponent::level_style("ERROR", false).fg, None);
    }

    #[test]
    fn level_style_unknown_level() {
        assert_eq!(LogsComponent::level_style("CUSTOM", true), Style::default());
    }

    // --- parse_log_line ---

    #[test]
    fn parse_log_line_with_timestamp_and_level() {
        let spans = LogsComponent::parse_log_line(
            "12:34:56 INFO ferro_wg_core::tunnel::mod: Connection abc is up",
            &cfg(true, true),
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
            &cfg(true, true),
        );
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content, "[ERROR]");
        assert_eq!(spans[2].style.fg, Some(Color::Red));
    }

    #[test]
    fn parse_log_line_warn_level() {
        let spans = LogsComponent::parse_log_line(
            "12:34:56 WARN ferro_wg_core::tunnel: Handshake timeout",
            &cfg(true, true),
        );
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content, "[WARN]");
        assert_eq!(spans[2].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn parse_log_line_debug_level() {
        let spans = LogsComponent::parse_log_line(
            "12:34:56 DEBUG ferro_wg_core::stats: Packet count: 42",
            &cfg(true, true),
        );
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content, "[DEBUG]");
        assert_eq!(spans[2].style.fg, Some(Color::Blue));
    }

    #[test]
    fn parse_log_line_no_timestamp_hides_prefix() {
        let spans = LogsComponent::parse_log_line(
            "12:34:56 INFO ferro_wg_core::tunnel: msg",
            &cfg(false, true),
        );
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
        );
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content, "[INFO]");
        // No color applied.
        assert_eq!(spans[2].style.fg, None);
    }

    #[test]
    fn parse_log_line_legacy_format() {
        let line = "INFO ferro_wg_core::tunnel::mod: Connection abc is up";
        let spans = LogsComponent::parse_log_line(line, &cfg(true, true));
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, line);
    }

    #[test]
    fn parse_log_line_malformed() {
        let line = "some random log message";
        let spans = LogsComponent::parse_log_line(line, &cfg(true, true));
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, line);
    }
}
