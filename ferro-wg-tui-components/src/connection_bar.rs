//! Connection bar: thin strip showing all connections with status indicators.

use std::borrow::Cow;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use ferro_wg_tui_core::{Action, AppState, Component, ConnectionState};

/// Maximum number of Unicode scalar values shown from a connection name
/// before the display is truncated with a `…` suffix.
const MAX_DISPLAY_NAME_LEN: usize = 20;

/// Number of rows this component occupies in the layout when shown.
///
/// The bar renders a single [`ratatui::widgets::Paragraph`] line with no border.
pub const CONNECTION_BAR_HEIGHT: u16 = 1;

/// Column width of the `" Connections: "` prefix shown when all entries fit.
const PREFIX_WIDTH: usize = 14;

/// Column width of the `" >"` right-overflow indicator.
const RIGHT_INDICATOR_WIDTH: usize = 2;

/// Minimum terminal width at which this bar renders useful content.
///
/// Equal to `PREFIX_WIDTH`(14) + `RIGHT_INDICATOR_WIDTH`(2) + 24 columns of
/// entry budget = 40.  The 24-column budget is enough for the viewport
/// algorithm to show the selected entry and at least one neighbour.
///
/// The compile-time assertion below keeps this literal in sync with its
/// constituent constants so the build breaks if `PREFIX_WIDTH` or
/// `RIGHT_INDICATOR_WIDTH` ever change without a corresponding update here.
/// The host layout should suppress the bar below this value.
pub const MIN_USEFUL_WIDTH: u16 = 40;

const _: () = assert!(
    MIN_USEFUL_WIDTH as usize == PREFIX_WIDTH + RIGHT_INDICATOR_WIDTH + 24,
    "MIN_USEFUL_WIDTH is out of sync with PREFIX_WIDTH / RIGHT_INDICATOR_WIDTH"
);

/// Return a display-ready version of `name`, truncating with `…` when it
/// exceeds [`MAX_DISPLAY_NAME_LEN`] characters.
///
/// Truncation is char-boundary-safe (counts Unicode scalar values, not bytes).
/// Names within the limit are returned as a borrow; only truncated names
/// allocate.
fn truncate_name(name: &str) -> Cow<'_, str> {
    if name.chars().count() > MAX_DISPLAY_NAME_LEN {
        let head: String = name.chars().take(MAX_DISPLAY_NAME_LEN).collect();
        Cow::Owned(format!("{head}…"))
    } else {
        Cow::Borrowed(name)
    }
}

/// Count the number of decimal digits in `n` (minimum 1).
fn digit_count(mut n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    let mut count = 0;
    while n > 0 {
        count += 1;
        n /= 10;
    }
    count
}

/// Compute the column width of connection entry `index` (0-based).
///
/// `display_name` must already be truncated by the caller (via
/// [`truncate_name`]) so that `display_name.chars().count()` is the true
/// display width. Uses `chars().count()` as a display-width proxy — valid
/// for the ASCII and narrow-Unicode characters used in this component.
///
/// * **Selected** `"[N] <name> ●  "` — label + space + name + space + indicator + 2 spaces
/// * **Unselected** `"[N]●  "` — label + indicator + 2 spaces
fn entry_width(index: usize, display_name: &str, selected: bool) -> usize {
    // "[N]" = 2 brackets + number of digits in (index + 1)
    let label_width = 2 + digit_count(index + 1);
    if selected {
        // space + name + space + indicator(1) + 2 trailing spaces
        label_width + 1 + display_name.chars().count() + 1 + 1 + 2
    } else {
        // indicator(1) + 2 trailing spaces
        label_width + 1 + 2
    }
}

/// Compute the inclusive `[start, end]` viewport into `entry_widths` such
/// that `selected` is always visible and as many neighbours as possible fit
/// within `area_width` columns.
///
/// # Budget guarantee
///
/// The entry budget is `area_width − PREFIX_WIDTH − RIGHT_INDICATOR_WIDTH`
/// (`area_width − 16`).  This is conservative enough to guarantee correctness
/// regardless of which overflow indicators are shown:
///
/// * `start == 0` (no left overflow): `PREFIX_WIDTH`(14) + entries +
///   optional `RIGHT_INDICATOR_WIDTH`(2) ≤ `area_width` ✓
/// * `start > 0` (left overflow): `LEFT_INDICATOR_WIDTH`(2) + entries +
///   optional `RIGHT_INDICATOR_WIDTH`(2) ≤ `area_width − 12` ≤ `area_width` ✓
///
/// # Panics
///
/// Panics in debug builds if `entry_widths` is empty or if
/// `selected >= entry_widths.len()`.
fn viewport(entry_widths: &[usize], selected: usize, area_width: usize) -> (usize, usize) {
    debug_assert!(
        !entry_widths.is_empty(),
        "viewport: entry_widths must not be empty"
    );
    debug_assert!(
        selected < entry_widths.len(),
        "viewport: selected={selected} out of bounds (entry_widths.len()={})",
        entry_widths.len()
    );
    let n = entry_widths.len();

    // Fast path: everything fits with the full prefix, no overflow indicators.
    let total: usize = entry_widths.iter().sum();
    if total + PREFIX_WIDTH <= area_width {
        return (0, n - 1);
    }

    // Scrolled mode: conservative budget that is safe for all indicator combos.
    let budget = area_width.saturating_sub(PREFIX_WIDTH + RIGHT_INDICATOR_WIDTH);

    let sel_w = entry_widths[selected];
    if sel_w > budget {
        // Selected entry alone exceeds budget; show it and let ratatui clip.
        return (selected, selected);
    }

    let mut start = selected;
    let mut end = selected;
    let mut used = sel_w;

    loop {
        let mut grew = false;
        if start > 0 {
            let w = entry_widths[start - 1];
            if used + w <= budget {
                start -= 1;
                used += w;
                grew = true;
            }
        }
        if end + 1 < n {
            let w = entry_widths[end + 1];
            if used + w <= budget {
                end += 1;
                used += w;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }

    (start, end)
}

/// Thin horizontal bar rendered between the tab bar and content area
/// when more than one connection is configured.
///
/// Unselected connections are rendered compactly as `[N]●` to conserve
/// horizontal space. The selected connection expands to show its full
/// (possibly truncated) name in bold accent: `[N] name ●`.
///
/// When the full list exceeds the terminal width, a scrolled viewport is
/// computed so the selected entry is always visible.  `< ` and ` >` overflow
/// indicators appear at the edges when connections are hidden on that side.
///
/// The layout allocates a 1-row slot when `connections.len() > 1` and a
/// 0-row slot otherwise — single-connection users see no visual change.
pub struct ConnectionBarComponent;

impl ConnectionBarComponent {
    /// Create a new connection bar component.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ConnectionBarComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for ConnectionBarComponent {
    fn handle_key(&mut self, key: KeyEvent, _state: &AppState) -> Option<Action> {
        match key.code {
            KeyCode::Char('[') => Some(Action::SelectPrevConnection),
            KeyCode::Char(']') => Some(Action::SelectNextConnection),
            _ => None,
        }
    }

    fn update(&mut self, _action: &Action, _state: &AppState) {}

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        if state.connections.len() <= 1 || area.width == 0 {
            return;
        }

        let theme = &state.theme;
        let n = state.connections.len();
        let sel = state.selected_connection;

        // Pre-compute truncated display names once; reused in both the widths
        // pass and the spans loop so truncate_name is not called twice.
        let display_names: Vec<Cow<'_, str>> = state
            .connections
            .iter()
            .map(|conn| truncate_name(conn.name.as_str()))
            .collect();

        // Pre-compute display width for each entry.
        let widths: Vec<usize> = display_names
            .iter()
            .enumerate()
            .map(|(i, name)| entry_width(i, name, i == sel))
            .collect();

        let (view_start, view_end) = viewport(&widths, sel, area.width as usize);

        let show_left = view_start > 0;
        let show_right = view_end < n - 1;

        let mut spans: Vec<Span<'_>> = Vec::new();

        // Prefix: full label when all (or left-aligned) entries are visible;
        // overflow arrow otherwise.
        if show_left {
            spans.push(Span::styled("< ", Style::default().fg(theme.muted)));
        } else {
            spans.push(Span::raw(" Connections: "));
        }

        for (i, (conn, display_name)) in state
            .connections
            .iter()
            .zip(display_names.iter())
            .enumerate()
            .take(view_end + 1)
            .skip(view_start)
        {
            let (indicator, ind_style): (&'static str, Style) = match &conn.status {
                None => ("?", Style::default().fg(theme.muted)),
                Some(s) if s.state == ConnectionState::Connected => {
                    ("●", Style::default().fg(theme.success))
                }
                Some(_) => ("○", Style::default().fg(theme.muted)),
            };

            if i == sel {
                let label_style = Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD);
                spans.push(Span::styled(format!("[{}] ", i + 1), label_style));
                spans.push(Span::styled(display_name.clone(), label_style));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(indicator, ind_style));
                spans.push(Span::raw("  "));
            } else {
                spans.push(Span::raw(format!("[{}]", i + 1)));
                spans.push(Span::styled(indicator, ind_style));
                spans.push(Span::raw("  "));
            }
        }

        if show_right {
            spans.push(Span::styled(" >", Style::default().fg(theme.muted)));
        }

        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use ferro_wg_core::config::{AppConfig, InterfaceConfig, WgConfig};
    use ferro_wg_core::error::BackendKind;
    use ferro_wg_core::key::PrivateKey;
    use ferro_wg_core::stats::TunnelStats;
    use ferro_wg_tui_core::{ConnectionState, ConnectionStatus};

    fn make_interface() -> InterfaceConfig {
        InterfaceConfig {
            private_key: PrivateKey::generate(),
            listen_port: 51820,
            addresses: vec!["10.0.0.2/24".into()],
            dns: Vec::new(),
            dns_search: Vec::new(),
            mtu: 1420,
            fwmark: 0,
            pre_up: Vec::new(),
            post_up: Vec::new(),
            pre_down: Vec::new(),
            post_down: Vec::new(),
        }
    }

    fn make_wg_config() -> WgConfig {
        WgConfig {
            interface: make_interface(),
            peers: vec![],
        }
    }

    fn three_connection_state() -> AppState {
        let mut connections = BTreeMap::new();
        connections.insert("mia".to_string(), make_wg_config());
        connections.insert("ord01".to_string(), make_wg_config());
        connections.insert("tus1".to_string(), make_wg_config());
        AppState::new(AppConfig { connections })
    }

    fn many_connection_state(n: usize) -> AppState {
        let mut connections = BTreeMap::new();
        for i in 0..n {
            connections.insert(format!("c{i:02}"), make_wg_config());
        }
        AppState::new(AppConfig { connections })
    }

    fn render_bar(state: &AppState, width: u16) -> String {
        let backend = TestBackend::new(width, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let mut comp = ConnectionBarComponent::new();
                comp.render(frame, frame.area(), false, state);
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    // ── digit_count unit tests ───────────────────────────────────────────────

    #[test]
    fn digit_count_zero() {
        assert_eq!(digit_count(0), 1);
    }

    #[test]
    fn digit_count_single_digit() {
        assert_eq!(digit_count(1), 1);
        assert_eq!(digit_count(9), 1);
    }

    #[test]
    fn digit_count_two_digits() {
        assert_eq!(digit_count(10), 2);
        assert_eq!(digit_count(99), 2);
    }

    #[test]
    fn digit_count_three_digits() {
        assert_eq!(digit_count(100), 3);
        assert_eq!(digit_count(999), 3);
    }

    // ── entry_width unit tests ───────────────────────────────────────────────

    #[test]
    fn entry_width_unselected_single_digit() {
        // "[1]●  " = 3 + 1 + 2 = 6
        assert_eq!(entry_width(0, "mia", false), 6);
    }

    #[test]
    fn entry_width_unselected_double_digit() {
        // "[10]●  " = 4 + 1 + 2 = 7
        assert_eq!(entry_width(9, "anything", false), 7);
    }

    #[test]
    fn entry_width_selected_single_digit_short_name() {
        // "[1] mia ●  " = 3 + 1 + 3 + 1 + 1 + 2 = 11
        assert_eq!(entry_width(0, "mia", true), 11);
    }

    #[test]
    fn entry_width_selected_double_digit() {
        // "[10] c09 ●  " = 4 + 1 + 3 + 1 + 1 + 2 = 12
        assert_eq!(entry_width(9, "c09", true), 12);
    }

    #[test]
    fn entry_width_selected_name_truncated_to_limit() {
        // Name exactly at MAX_DISPLAY_NAME_LEN — truncate_name returns it as-is.
        let name = "a".repeat(MAX_DISPLAY_NAME_LEN);
        let display = truncate_name(&name);
        // "[1] " (4) + 20 + " ●  " (4) = 28
        assert_eq!(entry_width(0, &display, true), 4 + MAX_DISPLAY_NAME_LEN + 4);
    }

    #[test]
    fn entry_width_selected_name_over_limit_uses_truncated_width() {
        // Name 5 chars over limit; caller truncates before passing to entry_width.
        let name = "a".repeat(MAX_DISPLAY_NAME_LEN + 5);
        let display = truncate_name(&name); // MAX_DISPLAY_NAME_LEN chars + "…" = +1
        let expected = 4 + (MAX_DISPLAY_NAME_LEN + 1) + 4; // 4 = "[1] ", 4 = " ●  "
        assert_eq!(entry_width(0, &display, true), expected);
    }

    // ── viewport unit tests ──────────────────────────────────────────────────

    #[test]
    fn viewport_fast_path_all_fit() {
        // 3 entries: widths [11, 6, 6] = 23 total. 23 + 14 = 37 <= 120.
        let widths = [11usize, 6, 6];
        assert_eq!(viewport(&widths, 0, 120), (0, 2));
    }

    #[test]
    fn viewport_selected_always_in_window() {
        // 10 entries of width 6 each, selected=4, area=40.
        // budget = 40 - 16 = 24. Algorithm expands from sel=4.
        let widths = [6usize; 10];
        let (start, end) = viewport(&widths, 4, 40);
        assert!(start <= 4, "start={start} must be ≤ selected=4");
        assert!(end >= 4, "end={end} must be ≥ selected=4");
    }

    #[test]
    fn viewport_sel_exceeds_budget_returns_sel_alone() {
        // budget = 10 - 16 = 0 (saturating); sel_w=20>0.
        let widths = [20usize, 20, 20];
        let (start, end) = viewport(&widths, 1, 10);
        assert_eq!((start, end), (1, 1));
    }

    #[test]
    fn viewport_selected_at_end_expands_left() {
        // 20 entries, sel=19 (12 wide), area=80. budget=64.
        // Entry widths for c00..c17 (unselected): indices 0-8 are 6, 9-18 are 7.
        // Entry 19 (selected c19, double-digit): 12.
        let mut widths: Vec<usize> = (0..19).map(|i| if i < 9 { 6 } else { 7 }).collect();
        widths.push(12); // selected at index 19
        let (start, end) = viewport(&widths, 19, 80);
        assert!(start < 19, "start={start} should expand left of sel=19");
        assert_eq!(end, 19);
    }

    #[test]
    fn viewport_no_right_overflow_when_end_reaches_last() {
        // When viewport includes the last entry, show_right must be false.
        let widths = [11usize, 6, 6];
        let (_, end) = viewport(&widths, 0, 120);
        assert_eq!(end, 2); // last index
    }

    // ── render correctness tests ─────────────────────────────────────────────

    #[test]
    fn connection_bar_hidden_single() {
        let mut connections = BTreeMap::new();
        connections.insert("mia".to_string(), make_wg_config());
        let state = AppState::new(AppConfig { connections });
        let content = render_bar(&state, 80);
        assert!(content.trim().is_empty());
    }

    #[test]
    fn connection_bar_renders_selected_name() {
        // BTreeMap sorts: mia(0), ord01(1), tus1(2); selected_connection = 0 → "mia"
        let state = three_connection_state();
        let content = render_bar(&state, 120);
        assert!(
            content.contains("mia"),
            "selected name must appear: {content:?}"
        );
    }

    #[test]
    fn connection_bar_unselected_names_hidden() {
        // selected = 0 ("mia"); ord01 and tus1 are compact — no name shown.
        let state = three_connection_state();
        let content = render_bar(&state, 120);
        assert!(
            !content.contains("ord01"),
            "unselected 'ord01' should not appear: {content:?}"
        );
        assert!(
            !content.contains("tus1"),
            "unselected 'tus1' should not appear: {content:?}"
        );
    }

    #[test]
    fn connection_bar_selection_change_shows_new_name() {
        // Move selection to index 1 → "ord01" expands, "mia" collapses.
        let mut state = three_connection_state();
        state.selected_connection = 1;
        let content = render_bar(&state, 120);
        assert!(
            content.contains("ord01"),
            "newly selected 'ord01' must appear: {content:?}"
        );
        assert!(
            !content.contains("mia"),
            "unselected 'mia' should not appear: {content:?}"
        );
    }

    #[test]
    fn connection_bar_not_polled_shows_question_mark() {
        let state = three_connection_state();
        let content = render_bar(&state, 120);
        assert!(
            content.contains('?'),
            "expected '?' indicator in: {content:?}"
        );
    }

    #[test]
    fn connection_bar_connected_shows_filled_circle() {
        let mut state = three_connection_state();
        state.connections[0].status = Some(ConnectionStatus {
            state: ConnectionState::Connected,
            backend: BackendKind::Boringtun,
            stats: TunnelStats::default(),
            endpoint: None,
            interface: None,
        });
        let content = render_bar(&state, 120);
        assert!(
            content.contains('●'),
            "expected '●' indicator in: {content:?}"
        );
    }

    #[test]
    fn connection_bar_disconnected_shows_open_circle() {
        let mut state = three_connection_state();
        state.connections[0].status = Some(ConnectionStatus {
            state: ConnectionState::Disconnected,
            backend: BackendKind::Boringtun,
            stats: TunnelStats::default(),
            endpoint: None,
            interface: None,
        });
        let content = render_bar(&state, 120);
        assert!(
            content.contains('○'),
            "expected '○' indicator in: {content:?}"
        );
    }

    #[test]
    fn connection_bar_no_panic_extremely_narrow() {
        let state = three_connection_state();
        render_bar(&state, 1);
    }

    #[test]
    fn connection_bar_many_connections_no_overflow_panic() {
        // 20 connections with selected at last index — must not panic.
        let mut state = many_connection_state(20);
        state.selected_connection = 19;
        render_bar(&state, 80);
    }

    // ── truncate_name unit tests ─────────────────────────────────────────────

    #[test]
    fn truncate_name_short_name_unchanged() {
        assert_eq!(truncate_name("mia"), Cow::Borrowed("mia"));
    }

    #[test]
    fn truncate_name_at_limit_unchanged() {
        let name = "a".repeat(MAX_DISPLAY_NAME_LEN);
        assert_eq!(truncate_name(&name), Cow::Borrowed(name.as_str()));
    }

    #[test]
    fn truncate_name_one_over_limit_appends_ellipsis() {
        let name = "a".repeat(MAX_DISPLAY_NAME_LEN + 1);
        let result = truncate_name(&name);
        assert!(
            result.ends_with('…'),
            "expected ellipsis suffix: {result:?}"
        );
        // Visible chars = MAX_DISPLAY_NAME_LEN a's + 1 ellipsis.
        assert_eq!(result.chars().count(), MAX_DISPLAY_NAME_LEN + 1);
    }

    #[test]
    fn truncate_name_long_name_prefix_preserved() {
        let name = "verylongconnectionname-datacenter-west";
        let result = truncate_name(name);
        let expected_prefix: String = name.chars().take(MAX_DISPLAY_NAME_LEN).collect();
        assert!(
            result.starts_with(expected_prefix.as_str()),
            "expected first {MAX_DISPLAY_NAME_LEN} chars preserved: {result:?}"
        );
        assert!(result.ends_with('…'));
    }

    // ── overflow / edge-case rendering tests ─────────────────────────────────

    #[test]
    fn long_selected_name_renders_with_ellipsis() {
        // "mlong-..." sorts after "anchor" lexicographically, so it lands at index 1.
        let long_name = "m".repeat(MAX_DISPLAY_NAME_LEN + 10);
        let mut connections = BTreeMap::new();
        connections.insert("anchor".to_string(), make_wg_config()); // index 0
        connections.insert(long_name.clone(), make_wg_config()); // index 1
        let mut state = AppState::new(AppConfig { connections });
        state.selected_connection = 1; // select the long-named connection
        let content = render_bar(&state, 120);
        assert!(
            content.contains('…'),
            "expected '…' for long selected name: {content:?}"
        );
        // The full long name must NOT appear verbatim.
        assert!(
            !content.contains(long_name.as_str()),
            "full long name must be truncated: {content:?}"
        );
    }

    #[test]
    fn short_selected_name_renders_without_ellipsis() {
        let state = three_connection_state(); // selected = 0 → "mia" (3 chars)
        let content = render_bar(&state, 120);
        assert!(
            !content.contains('…'),
            "short name must not be truncated: {content:?}"
        );
    }

    #[test]
    fn name_exactly_at_limit_renders_without_ellipsis() {
        let exact_name = "b".repeat(MAX_DISPLAY_NAME_LEN);
        let mut connections = BTreeMap::new();
        connections.insert("anchor".to_string(), make_wg_config());
        connections.insert(exact_name.clone(), make_wg_config());
        let mut state = AppState::new(AppConfig { connections });
        // BTreeMap sorts: "anchor" < "bbb...", so exact_name is at index 1.
        state.selected_connection = 1;
        let content = render_bar(&state, 120);
        assert!(
            !content.contains('…'),
            "name at limit must not be truncated: {content:?}"
        );
        assert!(content.contains(exact_name.as_str()));
    }

    #[test]
    fn zero_width_terminal_no_panic() {
        let state = three_connection_state();
        // ratatui clamps to minimum; render must not panic.
        render_bar(&state, 0);
    }

    #[test]
    fn single_char_terminal_no_panic() {
        let state = three_connection_state();
        render_bar(&state, 1);
    }

    #[test]
    fn fifty_connections_no_panic() {
        let mut state = many_connection_state(50);
        state.selected_connection = 49;
        render_bar(&state, 80);
    }

    #[test]
    fn fifty_connections_selected_name_visible() {
        // Even with 50 connections, the selected entry must appear somewhere.
        // Select index 0 ("c00") so it renders first, before any clipping.
        let state = many_connection_state(50); // selected_connection = 0 → "c00"
        let content = render_bar(&state, 120);
        assert!(
            content.contains("c00"),
            "selected 'c00' must appear: {content:?}"
        );
    }

    // ── scrolling correctness tests ───────────────────────────────────────────

    /// With 20 connections and selected at the last index, the selected name
    /// must be visible even on an 80-column terminal (far too narrow for all).
    #[test]
    fn scrolled_selected_at_end_is_visible() {
        let mut state = many_connection_state(20); // c00..c19
        state.selected_connection = 19; // "c19"
        let content = render_bar(&state, 80);
        assert!(
            content.contains("c19"),
            "selected 'c19' must be visible when scrolled: {content:?}"
        );
    }

    /// When many connections overflow to the right, the selected entry in
    /// the middle of the list must still be visible.
    #[test]
    fn scrolled_selected_in_middle_is_visible() {
        let mut state = many_connection_state(20); // c00..c19
        state.selected_connection = 9; // "c09"
        let content = render_bar(&state, 80);
        assert!(
            content.contains("c09"),
            "selected 'c09' must be visible when scrolled: {content:?}"
        );
    }

    /// The `< ` indicator must appear when entries are hidden to the left.
    #[test]
    fn scrolled_left_indicator_shown_when_clipped() {
        let mut state = many_connection_state(20);
        state.selected_connection = 19; // forces scroll right
        let content = render_bar(&state, 80);
        assert!(
            content.contains('<'),
            "left-overflow indicator '<' must appear: {content:?}"
        );
    }

    /// The ` >` indicator must appear when entries are hidden to the right.
    #[test]
    fn scrolled_right_indicator_shown_when_clipped() {
        let state = many_connection_state(20); // selected=0, many entries to the right
        let content = render_bar(&state, 80);
        assert!(
            content.contains('>'),
            "right-overflow indicator '>' must appear: {content:?}"
        );
    }

    /// When all entries fit, neither overflow indicator should be shown.
    #[test]
    fn no_overflow_indicators_when_all_fit() {
        // 3 connections on a wide terminal — everything fits, no arrows needed.
        let state = three_connection_state();
        let content = render_bar(&state, 120);
        assert!(
            !content.contains('<'),
            "no left indicator expected when all fit: {content:?}"
        );
        assert!(
            !content.contains('>'),
            "no right indicator expected when all fit: {content:?}"
        );
    }

    /// When the selected connection is at index 0, the left indicator must
    /// NOT appear (there is nothing to the left).
    #[test]
    fn no_left_indicator_when_selected_is_first() {
        let state = many_connection_state(20); // selected_connection = 0
        let content = render_bar(&state, 80);
        assert!(
            !content.contains('<'),
            "no '<' expected when selected is first entry: {content:?}"
        );
    }

    /// When the selected connection is at the last index, the right indicator
    /// must NOT appear (there is nothing to the right).
    #[test]
    fn no_right_indicator_when_selected_is_last() {
        let mut state = many_connection_state(20);
        state.selected_connection = 19;
        let content = render_bar(&state, 80);
        assert!(
            !content.contains('>'),
            "no '>' expected when selected is last entry: {content:?}"
        );
    }

    /// On a very narrow terminal (40 cols), the selected connection at an
    /// arbitrary index must still be visible.
    #[test]
    fn very_narrow_terminal_selected_visible() {
        let mut state = many_connection_state(20);
        state.selected_connection = 10; // "c10" — middle of the list
        let content = render_bar(&state, 40);
        assert!(
            content.contains("c10"),
            "selected 'c10' must be visible at 40-col width: {content:?}"
        );
    }

    // ── handle_key tests ─────────────────────────────────────────────────────

    #[test]
    fn handle_key_bracket_right_emits_select_next() {
        let mut comp = ConnectionBarComponent::new();
        let state = three_connection_state();
        let action = comp.handle_key(KeyEvent::from(KeyCode::Char(']')), &state);
        assert_eq!(action, Some(Action::SelectNextConnection));
    }

    #[test]
    fn handle_key_bracket_left_emits_select_prev() {
        let mut comp = ConnectionBarComponent::new();
        let state = three_connection_state();
        let action = comp.handle_key(KeyEvent::from(KeyCode::Char('[')), &state);
        assert_eq!(action, Some(Action::SelectPrevConnection));
    }

    #[test]
    fn handle_key_other_keys_return_none() {
        let mut comp = ConnectionBarComponent::new();
        let state = three_connection_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Enter), &state),
            None
        );
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('q')), &state),
            None
        );
    }

    // ── Unicode name tests ───────────────────────────────────────────────────

    /// CJK characters have display width 2 per char, but `chars().count()`
    /// returns 1 per char.  The bar must not panic even when the visual width
    /// estimate is off.
    #[test]
    fn cjk_names_no_panic() {
        let mut connections = BTreeMap::new();
        connections.insert("東京01".to_string(), make_wg_config());
        connections.insert("大阪02".to_string(), make_wg_config());
        connections.insert("名古屋".to_string(), make_wg_config());
        let state = AppState::new(AppConfig { connections });
        render_bar(&state, 80); // must not panic
    }

    #[test]
    fn emoji_names_no_panic() {
        let mut connections = BTreeMap::new();
        connections.insert("🌐-us-east".to_string(), make_wg_config());
        connections.insert("🔒-secure".to_string(), make_wg_config());
        let state = AppState::new(AppConfig { connections });
        render_bar(&state, 80); // must not panic
    }

    #[test]
    fn long_cjk_name_truncation_no_panic() {
        // Names exceeding MAX_DISPLAY_NAME_LEN composed of wide chars must not panic.
        let long_cjk = "東".repeat(MAX_DISPLAY_NAME_LEN + 5);
        let mut connections = BTreeMap::new();
        connections.insert("anchor".to_string(), make_wg_config());
        connections.insert(long_cjk, make_wg_config());
        let state = AppState::new(AppConfig { connections });
        render_bar(&state, 120); // must not panic
    }

    // ── Extreme connection count tests ───────────────────────────────────────

    #[test]
    fn one_thousand_connections_no_panic() {
        let mut state = many_connection_state(1000);
        state.selected_connection = 999;
        render_bar(&state, 80); // must not panic
    }

    #[test]
    fn one_thousand_connections_selected_middle_visible() {
        // Use 4-digit zero-padding so lexicographic and numeric order agree
        // (the shared many_connection_state helper uses :02 which interleaves
        // 2- and 3-digit names when n > 99).
        let mut connections = BTreeMap::new();
        for i in 0..1000usize {
            connections.insert(format!("c{i:04}"), make_wg_config());
        }
        let mut state = AppState::new(AppConfig { connections });
        state.selected_connection = 500; // "c0500"
        let content = render_bar(&state, 120);
        assert!(
            content.contains("c0500"),
            "selected 'c0500' must be visible at 1000 connections: {content:?}"
        );
    }

    #[test]
    fn one_thousand_connections_selected_first_no_left_indicator() {
        let state = many_connection_state(1000); // selected = 0
        let content = render_bar(&state, 80);
        assert!(
            !content.contains('<'),
            "no '<' expected when selected is first of 1000: {content:?}"
        );
    }
}
