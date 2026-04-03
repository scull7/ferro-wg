//! Connection bar: thin strip showing all connections with status indicators.

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use ferro_wg_tui_core::{Action, AppState, Component, ConnectionState, ConnectionView};

/// Fixed character width consumed by chrome surrounding the entry list.
///
/// `" Connections: "` (14) + `"◀  "` (3) + `"▶"` (1) = 18.
const CHROME_WIDTH: usize = 18;

/// Display width of one connection entry: `"[N] name X  "`.
///
/// N is 1-based (index 0 → `"[1] "`, index 9 → `"[10] "`).
/// The suffix ` X  ` is 4 chars: space + indicator(1) + 2 trailing spaces.
fn entry_width(idx: usize, name: &str) -> usize {
    format!("[{}] ", idx + 1).len() + name.len() + 4
}

/// First visible connection index (scroll offset).
///
/// Returns 0 when all entries fit within `avail` chars.
/// Otherwise returns the smallest `start ≤ selected` such that iterating
/// forward from `start` keeps `selected` within the visible window.
fn scroll_start(connections: &[ConnectionView], selected: usize, avail: usize) -> usize {
    let total: usize = connections
        .iter()
        .enumerate()
        .map(|(i, c)| entry_width(i, &c.name))
        .sum();

    if total <= avail {
        return 0;
    }

    // Anchor to selected; expand left while there is room.
    let mut start = selected;
    let mut used = entry_width(selected, &connections[selected].name);
    while start > 0 {
        let w = entry_width(start - 1, &connections[start - 1].name);
        if used + w > avail {
            break;
        }
        start -= 1;
        used += w;
    }
    start
}

/// Thin horizontal bar rendered between the tab bar and content area
/// when more than one connection is configured.
///
/// Renders: ` Connections: ◀  [1] mia ●  [2] tus1 ○  [3] ord01 ?  ▶`
///
/// When connections overflow the terminal width a viewport is applied:
/// the selected connection is always visible and the `◀`/`▶` arrows are
/// highlighted in the accent colour when there are hidden entries in that
/// direction (dim when all entries in that direction are visible).
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
    fn handle_key(&mut self, _key: KeyEvent, _state: &AppState) -> Option<Action> {
        None
    }

    fn update(&mut self, _action: &Action, _state: &AppState) {}

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        if state.connections.len() <= 1 {
            return;
        }

        let theme = &state.theme;
        let avail = (area.width as usize).saturating_sub(CHROME_WIDTH);
        let start = scroll_start(&state.connections, state.selected_connection, avail);

        // Determine the last visible entry index.
        let mut end = start;
        let mut used = 0usize;
        for (i, conn) in state.connections.iter().enumerate().skip(start) {
            let w = entry_width(i, &conn.name);
            // Always include at least the first entry (selected may be wider than avail).
            if used + w > avail && i > start {
                break;
            }
            used += w;
            end = i;
        }

        let can_scroll_left = start > 0;
        let can_scroll_right = end + 1 < state.connections.len();

        let arrow_style = |active: bool| -> Style {
            if active {
                Style::default().fg(theme.accent)
            } else {
                Style::default().fg(theme.muted)
            }
        };

        let mut spans: Vec<Span<'static>> = vec![
            Span::raw(" Connections: "),
            Span::styled("◀  ", arrow_style(can_scroll_left)),
        ];

        for (i, conn) in state
            .connections
            .iter()
            .enumerate()
            .skip(start)
            .take(end - start + 1)
        {
            let (indicator, ind_style): (&'static str, Style) = match &conn.status {
                None => ("?", Style::default().fg(theme.warning)),
                Some(s) if s.state == ConnectionState::Connected => {
                    ("●", Style::default().fg(theme.success))
                }
                Some(_) => ("○", Style::default().fg(theme.muted)),
            };

            let is_selected = i == state.selected_connection;
            let name_style = if is_selected {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            spans.push(Span::raw(format!("[{}] ", i + 1)));
            spans.push(Span::styled(conn.name.clone(), name_style));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(indicator, ind_style));
            spans.push(Span::raw("  "));
        }

        spans.push(Span::styled("▶", arrow_style(can_scroll_right)));

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

    // ── entry_width / scroll_start unit tests ────────────────────────────────

    #[test]
    fn entry_width_single_digit_index() {
        // "[1] abc ?  " = 4 + 3 + 4 = 11
        assert_eq!(entry_width(0, "abc"), 11);
    }

    #[test]
    fn entry_width_double_digit_index() {
        // "[10] abc ?  " = 5 + 3 + 4 = 12
        assert_eq!(entry_width(9, "abc"), 12);
    }

    #[test]
    fn scroll_start_returns_zero_when_all_fit() {
        let state = three_connection_state();
        // Total width for 3 entries with names "mia"(11), "ord01"(13), "tus1"(12) = 36
        let start = scroll_start(&state.connections, 2, 100);
        assert_eq!(start, 0);
    }

    #[test]
    fn scroll_start_anchors_to_selected_when_overflow() {
        // 10 connections named "c00".."c09", each entry_width ≈ 11 chars.
        // With avail = 15, only 1 entry fits.  selected = 9 → start = 9.
        let state = many_connection_state(10);
        let start = scroll_start(&state.connections, 9, 15);
        assert_eq!(start, 9);
    }

    #[test]
    fn scroll_start_expands_left_when_possible() {
        // 10 connections. avail = 30 → fits ≈ 2 entries.
        // selected = 9; entries 8 & 9 together ≈ 11 + 11 = 22 ≤ 30 → start = 8.
        let state = many_connection_state(10);
        let start = scroll_start(&state.connections, 9, 30);
        assert!(start <= 9, "start ({start}) should be ≤ selected (9)");
        assert!(
            start >= 8,
            "start ({start}) should pull left to fit more entries"
        );
    }

    // ── rendering tests ───────────────────────────────────────────────────────

    #[test]
    fn connection_bar_hidden_single() {
        let mut connections = BTreeMap::new();
        connections.insert("mia".to_string(), make_wg_config());
        let state = AppState::new(AppConfig { connections });
        // With a single connection the bar renders nothing.
        let content = render_bar(&state, 80);
        assert!(content.trim().is_empty());
    }

    #[test]
    fn connection_bar_renders_all_names() {
        let state = three_connection_state();
        let content = render_bar(&state, 120);
        assert!(content.contains("mia"), "expected 'mia' in: {content:?}");
        assert!(content.contains("tus1"), "expected 'tus1' in: {content:?}");
        assert!(
            content.contains("ord01"),
            "expected 'ord01' in: {content:?}"
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
    fn connection_bar_selected_always_visible_when_narrow() {
        // 10 connections, terminal only 40 wide — selected (last) must appear.
        let mut state = many_connection_state(10);
        state.selected_connection = 9; // "c09"
        let content = render_bar(&state, 40);
        assert!(
            content.contains("c09"),
            "selected 'c09' must be visible in narrow bar: {content:?}"
        );
    }

    #[test]
    fn connection_bar_hides_overflow_entries() {
        // 10 connections, very narrow terminal — first entry must not appear when
        // the selected connection is at the end of the list.
        let mut state = many_connection_state(10);
        state.selected_connection = 9; // last entry
        // Each entry ≈ 11 chars, chrome = 18. At width 30 avail = 12 → 1 entry fits.
        let content = render_bar(&state, 30);
        assert!(
            !content.contains("c00"),
            "c00 should be scrolled out of view: {content:?}"
        );
    }

    #[test]
    fn connection_bar_no_panic_extremely_narrow() {
        // Even with a 1-char-wide terminal, rendering must not panic.
        let state = three_connection_state();
        render_bar(&state, 1);
    }
}
