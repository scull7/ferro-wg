use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use tracing::warn;

use ferro_wg_core::config::LogDisplayConfig;
use ferro_wg_tui_core::{Action, AppState, Component, Theme};

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

/// Number of rows consumed by the panel block's top and bottom borders.
const BLOCK_BORDER_HEIGHT: u16 = 2;

/// Logs tab: scrollable log viewer with parsed log lines.
#[derive(Debug, Clone)]
pub struct LogsComponent {
    /// Current scroll state for the log viewer.
    scroll_state: ScrollState,
}

/// Scroll state for the logs component.
#[derive(Debug, Clone)]
pub(crate) struct ScrollState {
    /// The index of the first visible log line in the `log_lines` `VecDeque`.
    pub(crate) offset: usize,
    /// Whether to automatically scroll to the bottom when new logs are added.
    pub(crate) auto_scroll: bool,
    /// The number of lines that fit in the current view area.
    pub(crate) view_height: usize,
}

impl Default for ScrollState {
    /// Creates a `ScrollState` with auto-scroll enabled so the component
    /// tracks the bottom of the log until the user manually scrolls away.
    fn default() -> Self {
        Self {
            offset: 0,
            auto_scroll: true,
            view_height: 0,
        }
    }
}

impl LogsComponent {
    /// Extract the `HH:MM:SS` timestamp from the start of a log line.
    ///
    /// Returns `Some(timestamp)` when the line begins with the pattern
    /// `HH:MM:SS ` (digits, colons, trailing space), `None` otherwise.
    #[must_use]
    pub fn parse_timestamp(line: &str) -> Option<&str> {
        let mut chars = line.chars();
        let valid = chars.next()?.is_ascii_digit()
            && chars.next()?.is_ascii_digit()
            && chars.next()? == ':'
            && chars.next()?.is_ascii_digit()
            && chars.next()?.is_ascii_digit()
            && chars.next()? == ':'
            && chars.next()?.is_ascii_digit()
            && chars.next()?.is_ascii_digit()
            && chars.next()? == ' ';
        // All 8 preceding chars are ASCII (digits/colons), so byte offset 8 is
        // a valid char boundary and `line[..8]` is always a well-formed str.
        if valid { Some(&line[..8]) } else { None }
    }

    /// Return the [`Style`] to apply to a level badge.
    ///
    /// Colors are drawn from `theme` so they stay consistent with the active
    /// palette. When `color_badges` is `false` the returned style is unstyled.
    #[must_use]
    pub fn level_style(level: &str, color_badges: bool, theme: &Theme) -> Style {
        if !color_badges {
            return Style::default();
        }
        let color = match level {
            "TRACE" => theme.muted,
            "DEBUG" => theme.accent,
            "INFO" => theme.success,
            "WARN" => theme.warning,
            "ERROR" => theme.error,
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
    /// Expects lines in the format emitted by `LogLayer`:
    /// `HH:MM:SS LEVEL target: message`.
    ///
    /// Display behaviour is controlled by `config`:
    /// - `show_timestamps`: include the `[HH:MM:SS]` prefix span.
    /// - `color_badges`: apply color to the `[LEVEL]` span; plain text otherwise.
    ///
    /// The message portion of the span borrows directly from `line` via
    /// [`Cow::Borrowed`](std::borrow::Cow), avoiding an allocation for the
    /// (typically largest) part of each log entry. Only the bracketed
    /// `[HH:MM:SS]` and `[LEVEL]` labels require a heap allocation.
    ///
    /// # Errors
    ///
    /// Returns [`LogParseError::MalformedTimestamp`] when the line does not begin
    /// with `HH:MM:SS `, or [`LogParseError::UnknownLevel`] when the level token
    /// is not a recognised tracing level. Callers should fall back to a plain-text
    /// span on error.
    pub fn parse_log_line<'a>(
        line: &'a str,
        config: &LogDisplayConfig,
        theme: &Theme,
    ) -> Result<Vec<Span<'a>>, LogParseError> {
        let line = line.trim();

        let timestamp = Self::parse_timestamp(line).ok_or(LogParseError::MalformedTimestamp)?;
        let (level, message) = Self::extract_level_message(&line[9..])?;

        let mut spans: Vec<Span<'a>> = Vec::with_capacity(5);
        if config.show_timestamps {
            spans.push(Span::styled(
                format!("[{timestamp}]"),
                Style::default().fg(theme.accent),
            ));
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            format!("[{level}]"),
            Self::level_style(level, config.color_badges, theme),
        ));
        spans.push(Span::raw(" "));
        // `message` borrows directly from `line` — no allocation needed.
        spans.push(Span::raw(message));
        Ok(spans)
    }

    /// Scroll up by one line toward older entries, disabling auto-scroll.
    pub fn scroll_up(&mut self) {
        if self.scroll_state.offset > 0 {
            self.scroll_state.offset -= 1;
            self.scroll_state.auto_scroll = false;
        }
    }

    /// Scroll down by one line toward newer entries, disabling auto-scroll.
    pub fn scroll_down(&mut self, total_lines: usize) {
        let max_offset = total_lines.saturating_sub(self.scroll_state.view_height);
        if self.scroll_state.offset < max_offset {
            self.scroll_state.offset += 1;
            self.scroll_state.auto_scroll = false;
        }
    }

    /// Jump to the oldest entry (top), disabling auto-scroll.
    pub fn jump_to_top(&mut self) {
        self.scroll_state.offset = 0;
        self.scroll_state.auto_scroll = false;
    }

    /// Jump to the newest entry (bottom), enabling auto-scroll.
    pub fn jump_to_bottom(&mut self, total_lines: usize) {
        self.scroll_state.offset = total_lines.saturating_sub(self.scroll_state.view_height);
        self.scroll_state.auto_scroll = true;
    }

    /// Lock `log_lines` and return the current line count.
    ///
    /// Returns `None` (and emits a warning) if the mutex is poisoned.
    fn total_lines(state: &AppState) -> Option<usize> {
        if let Ok(lines) = state.log_lines.lock() {
            Some(lines.len())
        } else {
            warn!("log_lines mutex poisoned in handle_key");
            None
        }
    }

    /// Compute the first visible line index for the current render pass.
    ///
    /// When `auto_scroll` is enabled the offset tracks the bottom of the log;
    /// otherwise the manually-scrolled `offset` is used unchanged.
    /// This is a pure calculation — it never modifies `self`.
    fn display_offset(&self, total_lines: usize) -> usize {
        if self.scroll_state.auto_scroll {
            total_lines.saturating_sub(self.scroll_state.view_height)
        } else {
            self.scroll_state.offset
        }
    }

    /// Create a new logs component.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scroll_state: ScrollState::default(),
        }
    }
}

impl Default for LogsComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for LogsComponent {
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action> {
        use crossterm::event::KeyCode;

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_up();
                None
            }
            KeyCode::Char('g') => {
                self.jump_to_top();
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let total = Self::total_lines(state)?;
                self.scroll_down(total);
                None
            }
            KeyCode::Char('G') => {
                let total = Self::total_lines(state)?;
                self.jump_to_bottom(total);
                None
            }
            _ => None,
        }
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

        let total_lines = log_lines.len();
        // Cache view_height so handle_key can compute valid scroll bounds.
        // This is the only write allowed in render; offset is never touched here.
        self.scroll_state.view_height = area.height.saturating_sub(BLOCK_BORDER_HEIGHT) as usize;
        let offset = self.display_offset(total_lines);

        let lines: Vec<Line<'_>> = if log_lines.is_empty() {
            vec![Line::from(Span::styled(
                "(no log entries yet)",
                Style::default().fg(theme.muted),
            ))]
        } else {
            log_lines
                .iter()
                .skip(offset)
                .take(self.scroll_state.view_height)
                .map(|l| {
                    let spans = Self::parse_log_line(l, config, theme)
                        .unwrap_or_else(|_| vec![Span::raw(l.as_str())]);
                    Line::from(spans)
                })
                .collect()
        };

        let paragraph = Paragraph::new(lines).block(theme.panel_block("Logs"));
        frame.render_widget(paragraph, area);

        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));
        let mut scrollbar_state = ScrollbarState::default()
            .content_length(total_lines)
            .position(offset);
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(show_timestamps: bool, color_badges: bool) -> LogDisplayConfig {
        LogDisplayConfig {
            show_timestamps,
            color_badges,
        }
    }

    fn mocha() -> Theme {
        Theme::mocha()
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
        let theme = mocha();
        let cases = [
            ("TRACE", theme.muted),
            ("DEBUG", theme.accent),
            ("INFO", theme.success),
            ("WARN", theme.warning),
            ("ERROR", theme.error),
        ];
        for (level, expected) in cases {
            assert_eq!(
                LogsComponent::level_style(level, true, &theme).fg,
                Some(expected),
                "level={level} with color"
            );
            assert_eq!(
                LogsComponent::level_style(level, false, &theme).fg,
                None,
                "level={level} without color"
            );
        }
    }

    #[test]
    fn level_style_unknown_level() {
        assert_eq!(
            LogsComponent::level_style("CUSTOM", true, &mocha()),
            Style::default()
        );
    }

    // --- parse_log_line: success paths ---

    #[test]
    fn parse_log_line_full_structure() {
        let theme = mocha();
        let spans = LogsComponent::parse_log_line(
            "12:34:56 INFO ferro_wg_core::tunnel::mod: Connection abc is up",
            &cfg(true, true),
            &theme,
        )
        .unwrap();

        assert_eq!(spans.len(), 5);
        assert_eq!(spans[0].content, "[12:34:56]");
        assert_eq!(spans[0].style.fg, Some(theme.accent));
        assert_eq!(spans[2].content, "[INFO]");
        assert_eq!(spans[2].style.fg, Some(theme.success));
        assert_eq!(
            spans[4].content,
            "ferro_wg_core::tunnel::mod: Connection abc is up"
        );
    }

    #[test]
    fn parse_log_line_all_known_levels() {
        let theme = mocha();
        let cases = [
            ("TRACE", theme.muted),
            ("DEBUG", theme.accent),
            ("INFO", theme.success),
            ("WARN", theme.warning),
            ("ERROR", theme.error),
        ];
        for (level, expected_color) in cases {
            let line = format!("12:34:56 {level} target: message");
            let spans = LogsComponent::parse_log_line(&line, &cfg(true, true), &theme)
                .unwrap_or_else(|e| panic!("level={level}: {e}"));
            assert_eq!(spans[2].content, format!("[{level}]"), "level={level}");
            assert_eq!(spans[2].style.fg, Some(expected_color), "level={level}");
        }
    }

    #[test]
    fn parse_log_line_no_timestamp_hides_prefix() {
        let theme = mocha();
        let spans = LogsComponent::parse_log_line(
            "12:34:56 INFO ferro_wg_core::tunnel: msg",
            &cfg(false, true),
            &theme,
        )
        .unwrap();
        // No timestamp + space → 3 spans: [LEVEL], space, message.
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "[INFO]");
        assert_eq!(spans[0].style.fg, Some(theme.success));
    }

    #[test]
    fn parse_log_line_no_color_plain_level_badge() {
        let theme = mocha();
        let spans = LogsComponent::parse_log_line(
            "12:34:56 INFO ferro_wg_core::tunnel: msg",
            &cfg(true, false),
            &theme,
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
        let theme = mocha();
        let cases = [
            "INFO ferro_wg_core::tunnel::mod: Connection abc is up",
            "some random log message",
        ];
        for line in cases {
            let err = LogsComponent::parse_log_line(line, &cfg(true, true), &theme).unwrap_err();
            assert_eq!(err, LogParseError::MalformedTimestamp, "line={line:?}");
        }
    }

    #[test]
    fn parse_log_line_unknown_level_errs() {
        let theme = mocha();
        let err = LogsComponent::parse_log_line(
            "12:34:56 CUSTOM ferro_wg_core::tunnel: msg",
            &cfg(true, true),
            &theme,
        )
        .unwrap_err();
        assert_eq!(err, LogParseError::UnknownLevel("CUSTOM".to_owned()));
    }

    // --- ScrollState tests ---

    #[test]
    fn scroll_up_decreases_offset_when_possible() {
        let mut component = LogsComponent::new();
        component.scroll_state.offset = 5;
        component.scroll_up();
        assert_eq!(component.scroll_state.offset, 4);
        assert!(!component.scroll_state.auto_scroll);
    }

    #[test]
    fn scroll_up_does_nothing_at_zero_offset() {
        let mut component = LogsComponent::new();
        component.scroll_state.offset = 0;
        component.scroll_up();
        assert_eq!(component.scroll_state.offset, 0);
        assert!(component.scroll_state.auto_scroll);
    }

    #[test]
    fn scroll_down_increases_offset_when_possible() {
        let mut component = LogsComponent::new();
        component.scroll_state.view_height = 10;
        component.scroll_state.offset = 0;
        component.scroll_down(20); // total_lines = 20
        assert_eq!(component.scroll_state.offset, 1);
        assert!(!component.scroll_state.auto_scroll);
    }

    #[test]
    fn scroll_down_does_nothing_at_max_offset() {
        let mut component = LogsComponent::new();
        component.scroll_state.view_height = 10;
        component.scroll_state.offset = 10; // max_offset = 20 - 10 = 10
        component.scroll_down(20);
        assert_eq!(component.scroll_state.offset, 10);
        assert!(component.scroll_state.auto_scroll);
    }

    #[test]
    fn jump_to_top_sets_offset_to_zero_and_disables_auto_scroll() {
        let mut component = LogsComponent::new();
        component.scroll_state.offset = 15;
        component.scroll_state.auto_scroll = true;
        component.jump_to_top();
        assert_eq!(component.scroll_state.offset, 0);
        assert!(!component.scroll_state.auto_scroll);
    }

    #[test]
    fn jump_to_bottom_sets_offset_to_max_and_enables_auto_scroll() {
        let mut component = LogsComponent::new();
        component.scroll_state.view_height = 5;
        component.scroll_state.offset = 0;
        component.scroll_state.auto_scroll = false;
        component.jump_to_bottom(25);
        assert_eq!(component.scroll_state.offset, 20); // 25 - 5 = 20
        assert!(component.scroll_state.auto_scroll);
    }

    #[test]
    fn display_offset_auto_scroll_clamps_to_bottom() {
        let mut component = LogsComponent::new();
        component.scroll_state.view_height = 5;
        // auto_scroll is true by default; offset must track the bottom.
        assert_eq!(component.display_offset(20), 15); // 20 - 5
    }

    #[test]
    fn display_offset_manual_scroll_uses_stored_offset() {
        let mut component = LogsComponent::new();
        component.scroll_state.auto_scroll = false;
        component.scroll_state.offset = 7;
        component.scroll_state.view_height = 5;
        assert_eq!(component.display_offset(20), 7);
    }

    #[test]
    fn display_offset_auto_scroll_saturates_at_zero_when_content_fits() {
        let mut component = LogsComponent::new();
        component.scroll_state.view_height = 30;
        assert_eq!(component.display_offset(10), 0); // 10 < 30 → saturates
    }
}
