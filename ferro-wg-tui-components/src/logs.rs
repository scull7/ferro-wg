use std::collections::{HashSet, VecDeque};

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use tracing::warn;

use ferro_wg_core::config::LogDisplayConfig;
use ferro_wg_core::ipc::{LogEntry, LogLevel};
use ferro_wg_core::logs as log_filter;
use ferro_wg_core::logs::ConnectionFilter;
use ferro_wg_tui_core::{Action, AppState, Component, Theme};

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
/// The last span is assumed to be the message body (a `&str` slice from the
/// original entry).  All preceding spans (timestamp, level badge) are returned
/// unchanged.  When `query` is empty the input is returned as-is.
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
    // The message span borrows directly from the LogEntry — it is always Borrowed.
    if let std::borrow::Cow::Borrowed(msg) = last.content {
        spans.extend(highlight_matches(msg, query, base, highlight_style));
    } else {
        spans.push(last);
    }
    spans
}

/// Number of rows consumed by the panel block's top and bottom borders.
const BLOCK_BORDER_HEIGHT: u16 = 2;

/// Logs tab: scrollable log viewer with level filtering, connection filtering,
/// and in-viewer search.
#[derive(Debug, Clone)]
pub struct LogsComponent {
    /// Current scroll state for the log viewer.
    scroll_state: ScrollState,
    /// Minimum severity to display.  Entries below this threshold are hidden
    /// at render time but are never evicted from the shared buffer.
    min_level: LogLevel,
    /// Whether to show all connections or only the selected one.
    connection_filter: ConnectionFilter,
}

/// Scroll state for the logs component.
#[derive(Debug, Clone)]
pub(crate) struct ScrollState {
    /// The index of the first visible log entry in the filtered result set.
    pub(crate) offset: usize,
    /// Whether to automatically scroll to the bottom when new entries arrive.
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
    /// Return the [`Style`] to apply to a level badge.
    ///
    /// Colors are drawn from `theme` so they stay consistent with the active
    /// palette.  When `color_badges` is `false` the returned style is unstyled.
    #[must_use]
    pub fn level_style(level: LogLevel, color_badges: bool, theme: &Theme) -> Style {
        if !color_badges {
            return Style::default();
        }
        let color = match level {
            LogLevel::Trace => theme.muted,
            LogLevel::Debug => theme.accent,
            LogLevel::Info => theme.success,
            LogLevel::Warn => theme.warning,
            LogLevel::Error => theme.error,
        };
        Style::default().fg(color)
    }

    /// Render a [`LogEntry`] into styled spans for display.
    ///
    /// Display behaviour is controlled by `config`:
    /// - `show_timestamps`: include the `[HH:MM:SS]` prefix span.
    /// - `color_badges`: apply colour to the `[LEVEL]` span; plain text otherwise.
    ///
    /// The message portion borrows directly from `entry.message` via
    /// `&str`, avoiding an allocation for the (typically largest) part of
    /// each log entry.  Only the bracketed prefix labels use heap-allocated
    /// owned strings.
    #[must_use]
    pub fn render_entry<'a>(
        entry: &'a LogEntry,
        config: &LogDisplayConfig,
        theme: &Theme,
    ) -> Vec<Span<'a>> {
        let mut spans: Vec<Span<'a>> =
            Vec::with_capacity(if config.show_timestamps { 5 } else { 3 });

        if config.show_timestamps {
            spans.push(Span::styled(
                format!("[{}]", entry.time_label()),
                Style::default().fg(theme.accent),
            ));
            spans.push(Span::raw(" "));
        }

        spans.push(Span::styled(
            format!("[{}]", entry.level.badge()),
            Self::level_style(entry.level, config.color_badges, theme),
        ));
        spans.push(Span::raw(" "));
        // Borrow directly from entry.message — no heap allocation.
        spans.push(Span::raw(entry.message.as_str()));
        spans
    }

    /// Return references to all entries in `buf` that pass both the current
    /// level/connection filter and the search predicate.
    ///
    /// `search` must be ASCII-lowercased by the caller (done once per render
    /// frame, not repeated per entry).  When all filters are inactive the fast
    /// path returns every entry without per-entry parsing.
    fn get_filtered_entries<'a>(
        &self,
        buf: &'a VecDeque<LogEntry>,
        search: &str,
        visible_connections: &HashSet<String>,
    ) -> Vec<&'a LogEntry> {
        let mut filtered = log_filter::filtered_lines(
            buf,
            search,
            self.min_level,
            ConnectionFilter::All, // always All to get all, then filter
            None,
        );
        if self.connection_filter == ConnectionFilter::Active {
            filtered.retain(|entry| {
                entry.connection_name.is_none()
                    || entry
                        .connection_name
                        .as_ref()
                        .is_some_and(|name| visible_connections.contains(name))
            });
        }
        filtered
    }

    /// Lock `log_entries` and return the number of entries that pass all active
    /// filters.
    ///
    /// Returns `None` (and emits a warning) if the mutex is poisoned.
    fn filtered_total(&self, state: &AppState) -> Option<usize> {
        let Ok(entries) = state.log_entries.lock() else {
            warn!("log_entries mutex poisoned in handle_key");
            return None;
        };
        let search = state.search_query.to_ascii_lowercase();
        Some(
            self.get_filtered_entries(&entries, &search, &state.visible_connections)
                .len(),
        )
    }

    /// Compute the first visible entry index for the current render pass.
    ///
    /// When `auto_scroll` is enabled the offset tracks the bottom of the log;
    /// otherwise the manually-scrolled `offset` is used unchanged.
    /// This is a pure calculation — it never modifies `self`.
    fn display_offset(&self, total_entries: usize) -> usize {
        if self.scroll_state.auto_scroll {
            total_entries.saturating_sub(self.scroll_state.view_height)
        } else {
            self.scroll_state.offset
        }
    }

    /// Scroll up by one entry toward older entries, disabling auto-scroll.
    pub fn scroll_up(&mut self) {
        if self.scroll_state.offset > 0 {
            self.scroll_state.offset -= 1;
            self.scroll_state.auto_scroll = false;
        }
    }

    /// Scroll down by one entry toward newer entries.
    pub fn scroll_down(&mut self, total_entries: usize) {
        let max_offset = total_entries.saturating_sub(self.scroll_state.view_height);
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
    pub fn jump_to_bottom(&mut self, total_entries: usize) {
        self.scroll_state.offset = total_entries.saturating_sub(self.scroll_state.view_height);
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

    /// Toggle the connection filter between [`All`](ConnectionFilter::All) and
    /// [`Active`](ConnectionFilter::Active), then snap back to the bottom.
    pub fn toggle_connection_filter(&mut self) {
        self.connection_filter = self.connection_filter.toggle();
        self.scroll_state.auto_scroll = true;
        self.scroll_state.offset = 0;
    }

    /// Create a new logs component.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scroll_state: ScrollState::default(),
            min_level: LogLevel::Debug,
            connection_filter: ConnectionFilter::All,
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
            KeyCode::Char('c') => {
                self.toggle_connection_filter();
                None
            }
            _ => None,
        }
    }

    fn update(&mut self, _action: &Action, _state: &AppState) {
        // No local state to update from actions.
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        if area.height == 0 || area.width < 20 {
            return;
        }
        let theme = &state.theme;
        let config = &state.log_display;

        let Ok(log_entries) = state.log_entries.lock() else {
            warn!("log_entries mutex poisoned, skipping render");
            return;
        };

        // Build the filtered view. Pre-lowercase once so per-entry matching is cheap.
        let search = state.search_query.to_ascii_lowercase();
        let filtered = self.get_filtered_entries(&log_entries, &search, &state.visible_connections);
        let total_filtered = filtered.len();

        // Cache view_height so handle_key can compute valid scroll bounds.
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
                .map(|entry| {
                    let spans = Self::render_entry(entry, config, theme);
                    Line::from(apply_search_highlights(spans, &search, highlight_style))
                })
                .collect()
        };

        // Build block title with level and connection filter labels.
        let conn_label: String = match self.connection_filter {
            ConnectionFilter::All => "all".to_owned(),
            ConnectionFilter::Active => "visible".to_owned(),
        };

        let title = if search.is_empty() {
            format!("Logs [{}] [{}]", self.min_level.title_label(), conn_label)
        } else {
            format!(
                "Logs [{}] [{}] [{} matches]",
                self.min_level.title_label(),
                conn_label,
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

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn make_entry(level: LogLevel, connection: Option<&str>, msg: &str) -> LogEntry {
        LogEntry {
            timestamp_ms: 0,
            level,
            connection_name: connection.map(ToOwned::to_owned),
            message: msg.to_owned(),
        }
    }

    fn debug_entry(msg: &str) -> LogEntry {
        make_entry(LogLevel::Debug, None, msg)
    }

    fn info_entry(msg: &str) -> LogEntry {
        make_entry(LogLevel::Info, None, msg)
    }

    fn make_buf(entries: &[LogEntry]) -> VecDeque<LogEntry> {
        entries.iter().cloned().collect()
    }

    fn cfg(show_timestamps: bool, color_badges: bool) -> LogDisplayConfig {
        LogDisplayConfig {
            show_timestamps,
            color_badges,
        }
    }

    fn mocha() -> Theme {
        Theme::mocha()
    }

    // ── ConnectionFilter ──────────────────────────────────────────────────────

    #[test]
    fn connection_filter_toggle() {
        assert_eq!(ConnectionFilter::All.toggle(), ConnectionFilter::Active);
        assert_eq!(ConnectionFilter::Active.toggle(), ConnectionFilter::All);
    }

    #[test]
    fn connection_filter_all_shows_all() {
        let component = LogsComponent::new(); // ConnectionFilter::All by default
        let buf = make_buf(&[
            make_entry(LogLevel::Info, Some("mia"), "msg a"),
            make_entry(LogLevel::Info, Some("ord"), "msg b"),
            make_entry(LogLevel::Info, None, "global msg"),
        ]);
        let visible = HashSet::new();
        let result = component.get_filtered_entries(&buf, "", &visible);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn connection_filter_active_hides_other_connection() {
        let mut component = LogsComponent::new();
        component.connection_filter = ConnectionFilter::Active;
        let buf = make_buf(&[
            make_entry(LogLevel::Info, Some("mia"), "mia entry"),
            make_entry(LogLevel::Info, Some("ord"), "ord entry"),
        ]);
        // visible connections = "mia"; "ord" entry should be hidden.
        let visible = HashSet::from(["mia".to_string()]);
        let result = component.get_filtered_entries(&buf, "", &visible);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].connection_name.as_deref(), Some("mia"));
    }

    #[test]
    fn connection_filter_active_global_events_always_pass() {
        let mut component = LogsComponent::new();
        component.connection_filter = ConnectionFilter::Active;
        let buf = make_buf(&[
            make_entry(LogLevel::Info, None, "global startup"),
            make_entry(LogLevel::Info, Some("ord"), "ord entry"),
        ]);
        let visible = HashSet::from(["mia".to_string()]);
        let result = component.get_filtered_entries(&buf, "", &visible);
        assert_eq!(result.len(), 1);
        assert!(
            result[0].connection_name.is_none(),
            "global event must pass"
        );
    }

    #[test]
    fn connection_filter_active_no_active_connection_shows_only_globals() {
        let mut component = LogsComponent::new();
        component.connection_filter = ConnectionFilter::Active;
        let buf = make_buf(&[
            make_entry(LogLevel::Info, None, "global"),
            make_entry(LogLevel::Info, Some("mia"), "mia entry"),
        ]);
        // No visible connections → only global events pass.
        let visible = HashSet::new();
        let result = component.get_filtered_entries(&buf, "", &visible);
        assert_eq!(result.len(), 1);
        assert!(result[0].connection_name.is_none());
    }

    #[test]
    fn c_key_toggles_filter_and_resets_scroll() {
        let mut component = LogsComponent::new();
        assert_eq!(component.connection_filter, ConnectionFilter::All);
        component.scroll_state.offset = 10;
        component.scroll_state.auto_scroll = false;
        component.toggle_connection_filter();
        assert_eq!(component.connection_filter, ConnectionFilter::Active);
        assert_eq!(component.scroll_state.offset, 0);
        assert!(component.scroll_state.auto_scroll);
    }

    // ── entry_passes_filter ───────────────────────────────────────────────────

    #[test]
    fn entry_passes_filter_trace_always_passes_level() {
        let trace_entry = make_entry(LogLevel::Trace, None, "trace");
        assert!(log_filter::entry_passes_filter(
            &trace_entry,
            LogLevel::Error,
            ConnectionFilter::All,
            None
        ));
    }

    #[test]
    fn entry_passes_filter_level_and_connection_compose() {
        // DEBUG + wrong connection → both filters reject.
        let debug_ord = make_entry(LogLevel::Debug, Some("ord"), "debug ord");
        assert!(!log_filter::entry_passes_filter(
            &debug_ord,
            LogLevel::Warn,
            ConnectionFilter::Active,
            Some("mia")
        ));

        // WARN + correct connection → passes.
        let warn_mia = make_entry(LogLevel::Warn, Some("mia"), "warn mia");
        assert!(log_filter::entry_passes_filter(
            &warn_mia,
            LogLevel::Warn,
            ConnectionFilter::Active,
            Some("mia")
        ));

        // WARN + wrong connection → level passes but connection rejects.
        let warn_ord = make_entry(LogLevel::Warn, Some("ord"), "warn ord");
        assert!(!log_filter::entry_passes_filter(
            &warn_ord,
            LogLevel::Warn,
            ConnectionFilter::Active,
            Some("mia")
        ));
    }

    // ── filtered_lines ────────────────────────────────────────────────────────

    #[test]
    fn filtered_lines_debug_fast_path_returns_all() {
        let buf = make_buf(&[
            debug_entry("a"),
            info_entry("b"),
            make_entry(LogLevel::Warn, None, "c"),
            make_entry(LogLevel::Error, None, "d"),
        ]);
        assert_eq!(
            log_filter::filtered_lines(&buf, "", LogLevel::Debug, ConnectionFilter::All, None)
                .len(),
            4
        );
    }

    #[test]
    fn filtered_lines_filters_below_threshold() {
        let buf = make_buf(&[
            debug_entry("debug"),
            info_entry("info"),
            make_entry(LogLevel::Warn, None, "warn"),
            make_entry(LogLevel::Error, None, "error"),
        ]);
        let result =
            log_filter::filtered_lines(&buf, "", LogLevel::Warn, ConnectionFilter::All, None);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].level, LogLevel::Warn);
        assert_eq!(result[1].level, LogLevel::Error);
    }

    #[test]
    fn filtered_lines_search_hides_non_matching() {
        let buf = make_buf(&[
            info_entry("tunnel connected"),
            info_entry("peer timeout"),
            make_entry(LogLevel::Error, None, "connection refused"),
        ]);
        let result = log_filter::filtered_lines(
            &buf,
            "refused",
            LogLevel::Debug,
            ConnectionFilter::All,
            None,
        );
        assert_eq!(result.len(), 1);
        assert!(result[0].message.contains("refused"));
    }

    #[test]
    fn filtered_lines_search_and_level_compose() {
        let buf = make_buf(&[
            debug_entry("noise debug"),
            make_entry(LogLevel::Warn, None, "peer timeout"),
            make_entry(LogLevel::Error, None, "connection refused"),
        ]);
        // Level filter hides DEBUG; search hides WARN → only ERROR passes.
        let result = log_filter::filtered_lines(
            &buf,
            "refused",
            LogLevel::Warn,
            ConnectionFilter::All,
            None,
        );
        assert_eq!(result.len(), 1);
        assert!(result[0].message.contains("refused"));
    }

    #[test]
    fn filtered_lines_fast_path_skipped_when_active_filter() {
        let mut component = LogsComponent::new(); // min_level = Debug (fast path candidate)
        component.connection_filter = ConnectionFilter::Active;
        let buf = make_buf(&[
            make_entry(LogLevel::Debug, Some("mia"), "mia"),
            make_entry(LogLevel::Debug, Some("ord"), "ord"),
        ]);
        // Fast path is skipped because connection_filter != All.
        let visible = HashSet::from(["mia".to_string()]);
        let result = component.get_filtered_entries(&buf, "", &visible);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].connection_name.as_deref(), Some("mia"));
    }

    // ── render_entry ──────────────────────────────────────────────────────────

    #[test]
    fn render_entry_with_timestamps_produces_five_spans() {
        let theme = mocha();
        let entry = info_entry("ferro_wg_core::tunnel: connected");
        let spans = LogsComponent::render_entry(&entry, &cfg(true, true), &theme);
        assert_eq!(
            spans.len(),
            5,
            "timestamp + space + badge + space + message"
        );
        // Timestamp span
        assert!(
            spans[0].content.starts_with('['),
            "timestamp should start with ["
        );
        // Level badge
        assert_eq!(spans[2].content, "[INFO]");
        assert_eq!(spans[2].style.fg, Some(theme.success));
        // Message borrows from entry
        assert_eq!(spans[4].content, "ferro_wg_core::tunnel: connected");
    }

    #[test]
    fn render_entry_without_timestamps_produces_three_spans() {
        let theme = mocha();
        let entry = info_entry("target: msg");
        let spans = LogsComponent::render_entry(&entry, &cfg(false, true), &theme);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "[INFO]");
        assert_eq!(spans[2].content, "target: msg");
    }

    #[test]
    fn render_entry_level_colors_all_variants() {
        let theme = mocha();
        let cases = [
            (LogLevel::Trace, theme.muted),
            (LogLevel::Debug, theme.accent),
            (LogLevel::Info, theme.success),
            (LogLevel::Warn, theme.warning),
            (LogLevel::Error, theme.error),
        ];
        for (level, expected_color) in cases {
            let entry = make_entry(level, None, "msg");
            let spans = LogsComponent::render_entry(&entry, &cfg(false, true), &theme);
            assert_eq!(spans[0].style.fg, Some(expected_color), "level={level:?}");
        }
    }

    #[test]
    fn render_entry_no_color_plain_level_badge() {
        let theme = mocha();
        let entry = info_entry("msg");
        let spans = LogsComponent::render_entry(&entry, &cfg(false, false), &theme);
        assert_eq!(spans[0].style.fg, None);
    }

    #[test]
    fn render_entry_invalid_timestamp_renders_fallback() {
        let theme = mocha();
        let entry = LogEntry {
            timestamp_ms: i64::MAX,
            level: LogLevel::Info,
            connection_name: None,
            message: "msg".to_owned(),
        };
        let spans = LogsComponent::render_entry(&entry, &cfg(true, false), &theme);
        assert_eq!(spans[0].content, "[??:??:??]");
    }

    // ── level_style ───────────────────────────────────────────────────────────

    #[test]
    fn level_style_all_known_levels() {
        let theme = mocha();
        let cases = [
            (LogLevel::Trace, theme.muted),
            (LogLevel::Debug, theme.accent),
            (LogLevel::Info, theme.success),
            (LogLevel::Warn, theme.warning),
            (LogLevel::Error, theme.error),
        ];
        for (level, expected) in cases {
            assert_eq!(
                LogsComponent::level_style(level, true, &theme).fg,
                Some(expected),
                "level={level:?} with color"
            );
            assert_eq!(
                LogsComponent::level_style(level, false, &theme).fg,
                None,
                "level={level:?} without color"
            );
        }
    }

    // ── line_matches_search ───────────────────────────────────────────────────

    #[test]
    fn line_matches_search_empty_query_always_passes() {
        assert!(log_filter::line_matches_search("anything", ""));
    }

    #[test]
    fn line_matches_search_case_insensitive() {
        assert!(log_filter::line_matches_search(
            "Connection Lost",
            "connection"
        ));
        assert!(log_filter::line_matches_search("Connection Lost", "lost"));
    }

    #[test]
    fn line_matches_search_no_match_returns_false() {
        assert!(!log_filter::line_matches_search("tunnel up", "error"));
    }

    // ── highlight_matches ─────────────────────────────────────────────────────

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
        assert_eq!(spans[1].content, "refused");
        assert_eq!(spans[1].style, hi);
    }

    #[test]
    fn highlight_matches_multiple_occurrences() {
        let base = Style::default();
        let hi = Style::default().bg(ratatui::style::Color::Yellow);
        let spans = highlight_matches("err: no error found", "err", base, hi);
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
        let spans = highlight_matches("Connection Lost", "connection", base, hi);
        assert_eq!(spans[0].content, "Connection");
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

    // ── cycle_level ───────────────────────────────────────────────────────────

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

    // ── scroll / jump tests ───────────────────────────────────────────────────

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
        component.scroll_down(20);
        assert_eq!(component.scroll_state.offset, 1);
        assert!(!component.scroll_state.auto_scroll);
    }

    #[test]
    fn scroll_down_does_nothing_at_max_offset() {
        let mut component = LogsComponent::new();
        component.scroll_state.view_height = 10;
        component.scroll_state.offset = 10;
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
        assert_eq!(component.scroll_state.offset, 20);
        assert!(component.scroll_state.auto_scroll);
    }

    #[test]
    fn display_offset_auto_scroll_clamps_to_bottom() {
        let mut component = LogsComponent::new();
        component.scroll_state.view_height = 5;
        assert_eq!(component.display_offset(20), 15);
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
        assert_eq!(component.display_offset(10), 0);
    }
}
