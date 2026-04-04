use std::borrow::Cow;
use std::collections::VecDeque;

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
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

/// Minimum log severity visible in the Logs tab.
///
/// Variants are ordered lowest-to-highest so that `entry_level >= self.min_level`
/// is the only filter predicate needed. TRACE is intentionally absent — TRACE
/// lines always pass the filter (fail-open), consistent with how rare TRACE
/// output is in production.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Show all lines (DEBUG and above). This is the default.
    Debug,
    /// Show INFO, WARN, and ERROR lines only.
    Info,
    /// Show WARN and ERROR lines only.
    Warn,
    /// Show ERROR lines only.
    Error,
}

impl LogLevel {
    /// Advance to the next severity threshold, wrapping from `Error` back to `Debug`.
    ///
    /// Cycle order: `Debug` → `Info` → `Warn` → `Error` → `Debug`.
    #[must_use]
    pub fn cycle(self) -> Self {
        match self {
            Self::Debug => Self::Info,
            Self::Info => Self::Warn,
            Self::Warn => Self::Error,
            Self::Error => Self::Debug,
        }
    }

    /// Short label used in the Logs block title (e.g. `"INFO+"`).
    #[must_use]
    pub fn title_label(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG+",
            Self::Info => "INFO+",
            Self::Warn => "WARN+",
            Self::Error => "ERROR",
        }
    }
}

/// Convert a level token string (as returned by [`LogsComponent::extract_level_message`])
/// into a [`LogLevel`].
///
/// Returns `None` for `"TRACE"` and any unrecognised token — callers must treat
/// `None` as **pass** (fail-open: unrecognised / TRACE lines are always shown).
fn parse_log_level(level: &str) -> Option<LogLevel> {
    match level {
        "DEBUG" => Some(LogLevel::Debug),
        "INFO" => Some(LogLevel::Info),
        "WARN" => Some(LogLevel::Warn),
        "ERROR" => Some(LogLevel::Error),
        _ => None,
    }
}

/// Return `true` when `query` appears anywhere in `line` (case-insensitive).
///
/// `query` **must already be ASCII-lowercased** by the caller (once per render
/// frame) to avoid re-lowercasing on every line.  Returns `true` for an empty
/// query so this predicate composes cleanly with the level filter — an empty
/// query is equivalent to "no search active".
fn line_matches_search(line: &str, query: &str) -> bool {
    query.is_empty() || line.to_ascii_lowercase().contains(query)
}

/// Split `text` into alternating un-highlighted / highlighted [`Span`]s.
///
/// `query` must be ASCII-lowercased by the caller; `text` is lowercased
/// internally for matching so the returned spans preserve the original casing.
/// Returns a single `base_style` span when `query` is empty.
fn highlight_matches<'a>(
    text: &'a str,
    query: &str,
    base_style: Style,
    highlight_style: Style,
) -> Vec<Span<'a>> {
    if query.is_empty() {
        return vec![Span::styled(text, base_style)];
    }
    let lower = text.to_ascii_lowercase();
    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut cursor = 0_usize;
    while let Some(pos) = lower[cursor..].find(query) {
        let abs = cursor + pos;
        if abs > cursor {
            spans.push(Span::styled(&text[cursor..abs], base_style));
        }
        spans.push(Span::styled(&text[abs..abs + query.len()], highlight_style));
        cursor = abs + query.len();
    }
    if cursor < text.len() {
        spans.push(Span::styled(&text[cursor..], base_style));
    }
    spans
}

/// Replace the last span in `spans` with per-match highlighted sub-spans.
///
/// The last span is assumed to be the message body (a `Cow::Borrowed` slice
/// from the original log line). All preceding spans (timestamp, level badge)
/// are returned unchanged. When `query` is empty the input is returned as-is.
fn apply_search_highlights<'a>(
    mut spans: Vec<Span<'a>>,
    query: &str,
    highlight_style: Style,
) -> Vec<Span<'a>> {
    if query.is_empty() {
        return spans;
    }
    let Some(last) = spans.pop() else {
        return spans;
    };
    let base = last.style;
    if let Cow::Borrowed(msg) = last.content {
        spans.extend(highlight_matches(msg, query, base, highlight_style));
    } else {
        spans.push(last);
    }
    spans
}

/// Number of rows consumed by the panel block's top and bottom borders.
const BLOCK_BORDER_HEIGHT: u16 = 2;

/// Logs tab: scrollable log viewer with parsed log lines and level filtering.
#[derive(Debug, Clone)]
pub struct LogsComponent {
    /// Current scroll state for the log viewer.
    scroll_state: ScrollState,
    /// Minimum severity to display. Lines below this threshold are hidden at
    /// render time but are never removed from the shared buffer.
    min_level: LogLevel,
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

    /// Cycle the minimum visible level: `Debug` → `Info` → `Warn` → `Error` → `Debug`.
    ///
    /// Resets `auto_scroll` to `true` so the view jumps to the bottom of the
    /// newly filtered result set, and clears the stored offset to avoid stale state.
    pub fn cycle_level(&mut self) {
        self.min_level = self.min_level.cycle();
        self.scroll_state.auto_scroll = true;
        self.scroll_state.offset = 0;
    }

    /// Return `true` when `line` should be visible under the current level filter.
    ///
    /// Fails open: lines that cannot be parsed (malformed timestamp, unknown or
    /// TRACE level) are always shown regardless of `min_level`.
    fn line_passes_filter(&self, line: &str) -> bool {
        // `parse_timestamp` returns the 8-byte "HH:MM:SS" slice; `+ 1` skips
        // the trailing space, matching the `&line[9..]` offset in `parse_log_line`.
        let Some(ts) = Self::parse_timestamp(line) else {
            return true;
        };
        match Self::extract_level_message(&line[ts.len() + 1..]) {
            Ok((level_str, _)) => {
                parse_log_level(level_str).is_none_or(|lvl| lvl >= self.min_level)
            }
            Err(_) => true,
        }
    }

    /// Return references to all lines in `buf` that pass both the current level
    /// filter and the search predicate.
    ///
    /// `search` must be ASCII-lowercased by the caller (done once per render
    /// frame / key event, not repeated per line).  When both `min_level` is
    /// `Debug` and `search` is empty the fast path returns every entry without
    /// per-line parsing.
    fn filtered_lines<'a>(&self, buf: &'a VecDeque<String>, search: &str) -> Vec<&'a String> {
        if self.min_level == LogLevel::Debug && search.is_empty() {
            return buf.iter().collect();
        }
        buf.iter()
            .filter(|l| self.line_passes_filter(l) && line_matches_search(l, search))
            .collect()
    }

    /// Lock `log_lines` and return the number of lines that pass both the
    /// current level filter and the active search query.
    ///
    /// Returns `None` (and emits a warning) if the mutex is poisoned.
    fn filtered_total(&self, state: &AppState) -> Option<usize> {
        let Ok(lines) = state.log_lines.lock() else {
            warn!("log_lines mutex poisoned in handle_key");
            return None;
        };
        let search = state.search_query.to_ascii_lowercase();
        Some(self.filtered_lines(&lines, &search).len())
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
            min_level: LogLevel::Debug,
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
                let total = self.filtered_total(state)?;
                self.scroll_down(total);
                None
            }
            KeyCode::Char('G') => {
                let total = self.filtered_total(state)?;
                self.jump_to_bottom(total);
                None
            }
            KeyCode::Char('l') => {
                self.cycle_level();
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

        // Build the filtered view. Pre-lowercase once so per-line matching is cheap.
        let search = state.search_query.to_ascii_lowercase();
        let filtered = self.filtered_lines(&log_lines, &search);
        let total_filtered = filtered.len();

        // Cache view_height so handle_key can compute valid scroll bounds.
        // This is the only write allowed in render; offset is never touched here.
        self.scroll_state.view_height = area.height.saturating_sub(BLOCK_BORDER_HEIGHT) as usize;
        let offset = self.display_offset(total_filtered);

        let highlight_style = Style::default()
            .bg(theme.warning)
            .add_modifier(Modifier::BOLD);

        let lines: Vec<Line<'_>> = if filtered.is_empty() {
            vec![Line::from(Span::styled(
                "(no log entries yet)",
                Style::default().fg(theme.muted),
            ))]
        } else {
            filtered
                .iter()
                .skip(offset)
                .take(self.scroll_state.view_height)
                .map(|l| {
                    let spans = Self::parse_log_line(l, config, theme)
                        .unwrap_or_else(|_| vec![Span::raw(l.as_str())]);
                    Line::from(apply_search_highlights(spans, &search, highlight_style))
                })
                .collect()
        };

        let title = if search.is_empty() {
            format!("Logs [{}]", self.min_level.title_label())
        } else {
            format!(
                "Logs [{}] [{} matches]",
                self.min_level.title_label(),
                total_filtered
            )
        };
        let paragraph = Paragraph::new(lines).block(theme.panel_block(&title));
        frame.render_widget(paragraph, area);

        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));
        let mut scrollbar_state = ScrollbarState::default()
            .content_length(total_filtered)
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

    // --- LogLevel ---

    #[test]
    fn log_level_ordering_is_debug_lt_error() {
        assert!(LogLevel::Debug < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Error);
    }

    #[test]
    fn log_level_cycle_wraps_through_all_four() {
        let level = LogLevel::Debug;
        let level = level.cycle();
        assert_eq!(level, LogLevel::Info);
        let level = level.cycle();
        assert_eq!(level, LogLevel::Warn);
        let level = level.cycle();
        assert_eq!(level, LogLevel::Error);
        let level = level.cycle();
        assert_eq!(level, LogLevel::Debug);
    }

    #[test]
    fn log_level_title_labels() {
        assert_eq!(LogLevel::Debug.title_label(), "DEBUG+");
        assert_eq!(LogLevel::Info.title_label(), "INFO+");
        assert_eq!(LogLevel::Warn.title_label(), "WARN+");
        assert_eq!(LogLevel::Error.title_label(), "ERROR");
    }

    // --- parse_log_level ---

    #[test]
    fn parse_log_level_known_levels() {
        assert_eq!(parse_log_level("DEBUG"), Some(LogLevel::Debug));
        assert_eq!(parse_log_level("INFO"), Some(LogLevel::Info));
        assert_eq!(parse_log_level("WARN"), Some(LogLevel::Warn));
        assert_eq!(parse_log_level("ERROR"), Some(LogLevel::Error));
    }

    #[test]
    fn parse_log_level_trace_returns_none() {
        assert_eq!(parse_log_level("TRACE"), None);
    }

    #[test]
    fn parse_log_level_unknown_returns_none() {
        assert_eq!(parse_log_level("CUSTOM"), None);
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

    // --- line_passes_filter ---

    #[test]
    fn filter_at_debug_passes_all() {
        let component = LogsComponent::new(); // min_level = Debug
        let lines = [
            "12:34:56 DEBUG t: msg",
            "12:34:56 INFO t: msg",
            "12:34:56 WARN t: msg",
            "12:34:56 ERROR t: msg",
            "12:34:56 TRACE t: msg",
            "some malformed garbage",
        ];
        for line in lines {
            assert!(
                component.line_passes_filter(line),
                "expected pass: {line:?}"
            );
        }
    }

    #[test]
    fn filter_at_info_hides_debug() {
        let mut component = LogsComponent::new();
        component.min_level = LogLevel::Info;

        assert!(!component.line_passes_filter("12:34:56 DEBUG t: msg"));
        assert!(component.line_passes_filter("12:34:56 INFO t: msg"));
        assert!(component.line_passes_filter("12:34:56 WARN t: msg"));
        assert!(component.line_passes_filter("12:34:56 ERROR t: msg"));
    }

    #[test]
    fn filter_at_warn_hides_info() {
        let mut component = LogsComponent::new();
        component.min_level = LogLevel::Warn;

        assert!(!component.line_passes_filter("12:34:56 DEBUG t: msg"));
        assert!(!component.line_passes_filter("12:34:56 INFO t: msg"));
        assert!(component.line_passes_filter("12:34:56 WARN t: msg"));
        assert!(component.line_passes_filter("12:34:56 ERROR t: msg"));
    }

    #[test]
    fn filter_at_error_only_passes_error() {
        let mut component = LogsComponent::new();
        component.min_level = LogLevel::Error;

        assert!(!component.line_passes_filter("12:34:56 DEBUG t: msg"));
        assert!(!component.line_passes_filter("12:34:56 INFO t: msg"));
        assert!(!component.line_passes_filter("12:34:56 WARN t: msg"));
        assert!(component.line_passes_filter("12:34:56 ERROR t: msg"));
    }

    #[test]
    fn filter_trace_always_passes() {
        // TRACE is not in LogLevel; parse_log_level returns None → fail-open.
        let mut component = LogsComponent::new();
        component.min_level = LogLevel::Error;
        assert!(component.line_passes_filter("12:34:56 TRACE t: msg"));
    }

    #[test]
    fn filter_malformed_always_passes() {
        let mut component = LogsComponent::new();
        component.min_level = LogLevel::Error;
        assert!(component.line_passes_filter("some random garbage"));
    }

    // --- filtered_lines ---

    fn make_buf(lines: &[&str]) -> VecDeque<String> {
        lines.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn filtered_lines_debug_fast_path_returns_all() {
        let component = LogsComponent::new(); // min_level = Debug
        let buf = make_buf(&[
            "12:34:56 DEBUG t: a",
            "12:34:56 INFO t: b",
            "12:34:56 WARN t: c",
            "12:34:56 ERROR t: d",
        ]);
        assert_eq!(component.filtered_lines(&buf, "").len(), 4);
    }

    #[test]
    fn filtered_lines_filters_below_threshold() {
        let mut component = LogsComponent::new();
        component.min_level = LogLevel::Warn;
        let buf = make_buf(&[
            "12:34:56 DEBUG t: a",
            "12:34:56 INFO t: b",
            "12:34:56 WARN t: c",
            "12:34:56 ERROR t: d",
        ]);
        let result = component.filtered_lines(&buf, "");
        assert_eq!(result.len(), 2);
        assert!(result[0].contains("WARN"));
        assert!(result[1].contains("ERROR"));
    }

    #[test]
    fn filtered_lines_empty_buffer() {
        let component = LogsComponent::new();
        let buf: VecDeque<String> = VecDeque::new();
        assert!(component.filtered_lines(&buf, "").is_empty());
    }

    // --- line_matches_search ---

    #[test]
    fn line_matches_search_empty_query_always_passes() {
        assert!(line_matches_search(
            "12:34:56 ERROR t: connection refused",
            ""
        ));
        assert!(line_matches_search("anything", ""));
    }

    #[test]
    fn line_matches_search_case_insensitive() {
        // Query must be pre-lowercased by the caller (as render() and
        // filtered_total() do); the line is lowercased internally.
        assert!(line_matches_search(
            "12:34:56 ERROR t: Connection Lost",
            "connection"
        ));
        assert!(line_matches_search(
            "12:34:56 ERROR t: Connection Lost",
            "lost"
        ));
        assert!(line_matches_search("12:34:56 INFO t: Tunnel UP", "tunnel"));
    }

    #[test]
    fn line_matches_search_no_match_returns_false() {
        assert!(!line_matches_search("12:34:56 INFO t: tunnel up", "error"));
    }

    #[test]
    fn filtered_lines_search_hides_non_matching() {
        let component = LogsComponent::new();
        let buf = make_buf(&[
            "12:34:56 INFO t: tunnel connected",
            "12:34:56 WARN t: peer timeout",
            "12:34:56 ERROR t: connection refused",
        ]);
        // Query is pre-lowercased; result lines retain their original casing.
        let result = component.filtered_lines(&buf, "refused");
        assert_eq!(result.len(), 1);
        assert!(result[0].contains("refused"));
    }

    #[test]
    fn filtered_lines_search_and_level_compose() {
        let mut component = LogsComponent::new();
        component.min_level = LogLevel::Warn;
        let buf = make_buf(&[
            "12:34:56 DEBUG t: noise debug",
            "12:34:56 WARN t: peer timeout",
            "12:34:56 ERROR t: connection refused",
        ]);
        // level filter hides DEBUG; search hides WARN → only ERROR passes
        let result = component.filtered_lines(&buf, "refused");
        assert_eq!(result.len(), 1);
        assert!(result[0].contains("refused"));
    }

    // --- highlight_matches ---

    #[test]
    fn highlight_matches_empty_query_returns_single_base_span() {
        let spans = highlight_matches("hello world", "", Style::default(), Style::default());
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "hello world");
    }

    #[test]
    fn highlight_matches_single_occurrence() {
        let base = Style::default();
        let hi = Style::default().bg(ratatui::style::Color::Yellow);
        let spans = highlight_matches("connection refused", "refused", base, hi);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "connection ");
        assert_eq!(spans[0].style, base);
        assert_eq!(spans[1].content, "refused");
        assert_eq!(spans[1].style, hi);
    }

    #[test]
    fn highlight_matches_multiple_occurrences() {
        let base = Style::default();
        let hi = Style::default().bg(ratatui::style::Color::Yellow);
        let spans = highlight_matches("err: no error found", "err", base, hi);
        // "err" appears at pos 0 and inside "error" at pos 8..11
        assert_eq!(spans.len(), 4);
        assert_eq!(spans[0].content, "err");
        assert_eq!(spans[1].content, ": no ");
        assert_eq!(spans[2].content, "err");
        assert_eq!(spans[3].content, "or found");
    }

    #[test]
    fn highlight_matches_preserves_original_case() {
        let base = Style::default();
        let hi = Style::default().bg(ratatui::style::Color::Yellow);
        // query is lowercase, text has mixed case; matched span preserves original
        let spans = highlight_matches("Connection Lost", "connection", base, hi);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "Connection"); // original casing preserved
        assert_eq!(spans[1].content, " Lost");
    }

    #[test]
    fn highlight_matches_full_string_match() {
        let base = Style::default();
        let hi = Style::default().bg(ratatui::style::Color::Yellow);
        let spans = highlight_matches("error", "error", base, hi);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].style, hi);
    }

    // --- cycle_level ---

    #[test]
    fn cycle_level_advances_min_level() {
        let mut component = LogsComponent::new();
        assert_eq!(component.min_level, LogLevel::Debug);
        component.cycle_level();
        assert_eq!(component.min_level, LogLevel::Info);
    }

    #[test]
    fn cycle_level_resets_auto_scroll_and_offset() {
        let mut component = LogsComponent::new();
        component.scroll_state.auto_scroll = false;
        component.scroll_state.offset = 42;
        component.cycle_level();
        assert!(component.scroll_state.auto_scroll);
        assert_eq!(component.scroll_state.offset, 0);
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
