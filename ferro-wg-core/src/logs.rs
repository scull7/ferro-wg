//! Pure functions for filtering log entries.
//!
//! These functions operate only on data types from `ferro_wg_core::ipc`,
//! making them suitable for reuse across CLI, TUI, and daemon components.

use std::collections::VecDeque;

use crate::ipc::{LogEntry, LogLevel};

/// Whether the logs view is filtered to a specific connection or shows all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionFilter {
    /// Show entries from every connection (and global daemon events).
    #[default]
    All,
    /// Show only entries for the currently selected connection plus global
    /// daemon events (`connection_name: None`).
    Active,
}

impl ConnectionFilter {
    /// Flip between [`All`](Self::All) and [`Active`](Self::Active).
    #[must_use]
    pub fn toggle(self) -> Self {
        match self {
            Self::All => Self::Active,
            Self::Active => Self::All,
        }
    }
}

/// Return `true` when `query` appears anywhere in `line` (case-insensitive).
///
/// `query` **must already be ASCII-lowercased** by the caller (once per render
/// frame) to avoid re-lowercasing on every line.  Returns `true` for an empty
/// query so this predicate composes cleanly with other filters.
#[must_use]
pub fn line_matches_search(line: &str, query: &str) -> bool {
    query.is_empty() || line.to_ascii_lowercase().contains(query)
}

/// Return `true` when `entry` should be visible under both the current
/// level filter and the connection filter.
///
/// - `Trace` entries always pass the level filter (fail-open).
/// - Entries with `connection_name: None` (global daemon events) always
///   pass the connection filter.
#[must_use]
pub fn entry_passes_filter(
    entry: &LogEntry,
    min_level: LogLevel,
    connection_filter: ConnectionFilter,
    active_connection: Option<&str>,
) -> bool {
    let level_ok = entry.level == LogLevel::Trace || entry.level >= min_level;
    let conn_ok = match connection_filter {
        ConnectionFilter::All => true,
        ConnectionFilter::Active => {
            // Global events always pass; connection events must match the selection.
            entry.connection_name.is_none()
                || active_connection
                    .is_some_and(|name| entry.connection_name.as_deref() == Some(name))
        }
    };
    level_ok && conn_ok
}

/// Return references to all entries in `buf` that pass both the current
/// level/connection filter and the search predicate.
///
/// `search` must be ASCII-lowercased by the caller (done once per render
/// frame, not repeated per entry).  When all filters are inactive the fast
/// path returns every entry without per-entry parsing.
#[must_use]
pub fn filtered_lines<'a>(
    buf: &'a VecDeque<LogEntry>,
    search: &str,
    min_level: LogLevel,
    connection_filter: ConnectionFilter,
    active_connection: Option<&str>,
) -> Vec<&'a LogEntry> {
    // Fast path: no filtering at all.
    if min_level == LogLevel::Debug
        && search.is_empty()
        && connection_filter == ConnectionFilter::All
    {
        return buf.iter().collect();
    }

    buf.iter()
        .filter(|e| {
            entry_passes_filter(e, min_level, connection_filter, active_connection)
                && line_matches_search(&e.message, search)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

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

    // ── ConnectionFilter ──────────────────────────────────────────────────────

    #[test]
    fn connection_filter_toggle() {
        assert_eq!(ConnectionFilter::All.toggle(), ConnectionFilter::Active);
        assert_eq!(ConnectionFilter::Active.toggle(), ConnectionFilter::All);
    }

    // ── line_matches_search ───────────────────────────────────────────────────

    #[test]
    fn line_matches_search_empty_query_always_passes() {
        assert!(line_matches_search("anything", ""));
    }

    #[test]
    fn line_matches_search_case_insensitive() {
        assert!(line_matches_search("Connection Lost", "connection"));
        assert!(line_matches_search("Connection Lost", "lost"));
    }

    #[test]
    fn line_matches_search_no_match_returns_false() {
        assert!(!line_matches_search("tunnel up", "error"));
    }

    // ── entry_passes_filter ───────────────────────────────────────────────────

    #[test]
    fn entry_passes_filter_trace_always_passes_level() {
        let trace_entry = make_entry(LogLevel::Trace, None, "trace");
        assert!(entry_passes_filter(
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
        assert!(!entry_passes_filter(
            &debug_ord,
            LogLevel::Warn,
            ConnectionFilter::Active,
            Some("mia")
        ));

        // WARN + correct connection → passes.
        let warn_mia = make_entry(LogLevel::Warn, Some("mia"), "warn mia");
        assert!(entry_passes_filter(
            &warn_mia,
            LogLevel::Warn,
            ConnectionFilter::Active,
            Some("mia")
        ));

        // WARN + wrong connection → level passes but connection rejects.
        let warn_ord = make_entry(LogLevel::Warn, Some("ord"), "warn ord");
        assert!(!entry_passes_filter(
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
            filtered_lines(&buf, "", LogLevel::Debug, ConnectionFilter::All, None).len(),
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
        let result = filtered_lines(&buf, "", LogLevel::Warn, ConnectionFilter::All, None);
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
        let result = filtered_lines(
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
        let result = filtered_lines(&buf, "refused", LogLevel::Warn, ConnectionFilter::All, None);
        assert_eq!(result.len(), 1);
        assert!(result[0].message.contains("refused"));
    }

    #[test]
    fn filtered_lines_fast_path_skipped_when_active_filter() {
        let buf = make_buf(&[
            make_entry(LogLevel::Debug, Some("mia"), "mia"),
            make_entry(LogLevel::Debug, Some("ord"), "ord"),
        ]);
        // Fast path is skipped because connection_filter != All.
        let result = filtered_lines(
            &buf,
            "",
            LogLevel::Debug,
            ConnectionFilter::Active,
            Some("mia"),
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].connection_name.as_deref(), Some("mia"));
    }
}
