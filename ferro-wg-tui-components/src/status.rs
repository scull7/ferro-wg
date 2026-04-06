//! Status tab: active tunnel overview.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

use ferro_wg_tui_core::{
    format_bytes, format_handshake_age, Action, AppState, Component, ConnectionState,
    ConnectionView, Theme,
};

// Status tab connection table column widths (percentages). Must sum to 100.
const COL_NAME_W: u16 = 15; // "Name"
const COL_STATUS_W: u16 = 10; // "Status"
const COL_BACKEND_W: u16 = 10; // "Backend"
const COL_ENDPOINT_W: u16 = 15; // "Endpoint"
const COL_INTERFACE_W: u16 = 10; // "Interface"
const COL_RXTX_W: u16 = 15; // "Rx / Tx"
const COL_HANDSHAKE_W: u16 = 15; // "Handshake"
const COL_WARNINGS_W: u16 = 10; // "Warnings"

const _: () = assert!(
    COL_NAME_W
        + COL_STATUS_W
        + COL_BACKEND_W
        + COL_ENDPOINT_W
        + COL_INTERFACE_W
        + COL_RXTX_W
        + COL_HANDSHAKE_W
        + COL_WARNINGS_W
        == 100,
    "Status percentage columns must sum to 100"
);

/// Active tunnel overview.
///
/// Displays a **connection-level** summary (state, backend, interface,
/// aggregate Rx/Tx bytes, last handshake) above a **per-peer** table
/// (name, endpoint, allowed IPs, keepalive interval).
///
/// Traffic and handshake metrics are whole-tunnel totals reported by the
/// backend — they are **not** per-peer measurements.
pub struct StatusComponent {
    /// Per-component table selection state.
    table_state: TableState,
}

impl StatusComponent {
    /// Create a new status component.
    #[must_use]
    pub fn new() -> Self {
        Self {
            table_state: TableState::default().with_selected(Some(0)),
        }
    }

    /// Number of displayable rows (visible connections).
    fn row_count(state: &AppState) -> usize {
        state
            .connections
            .iter()
            .filter(|c| state.visible_connections.contains(&c.name))
            .count()
    }

    /// Get the visible connections in display order.
    fn visible_connections(state: &AppState) -> Vec<&ConnectionView> {
        state
            .connections
            .iter()
            .filter(|c| state.visible_connections.contains(&c.name))
            .collect()
    }

    /// Get the name of the connection at the selected row, if any.
    fn selected_connection_name(state: &AppState, selected_row: usize) -> Option<String> {
        Self::visible_connections(state)
            .get(selected_row)
            .map(|c| c.name.clone())
    }

    /// Build connection table rows for visible connections.
    fn connection_rows(state: &AppState, theme: &Theme) -> Vec<Row<'static>> {
        Self::visible_connections(state)
            .into_iter()
            .map(|conn| {
                let (state_str, backend_str, endpoint, interface, rx_tx, hs, warning) =
                    conn.status.as_ref().map_or(
                        (
                            "down".to_owned(),
                            "—".to_owned(),
                            "—".to_owned(),
                            "—".to_owned(),
                            "—".to_owned(),
                            "—".to_owned(),
                            "—".to_owned(),
                        ),
                        |s| {
                            let state = if s.state == ConnectionState::Connected {
                                "connected"
                            } else {
                                "down"
                            };
                            let backend = s.backend.to_string();
                            let ep = s.endpoint.clone().unwrap_or_else(|| "—".to_owned());
                            let iface = s.interface.clone().unwrap_or_else(|| "—".to_owned());
                            let rx = format_bytes(s.stats.rx_bytes);
                            let tx = format_bytes(s.stats.tx_bytes);
                            let rxtx = format!("{rx} / {tx}");
                            let handshake = s
                                .stats
                                .last_handshake
                                .map_or_else(|| "—".to_owned(), format_handshake_age);
                            let warn = s.health_warning.clone().unwrap_or_else(|| "—".to_owned());
                            (state.to_owned(), backend, ep, iface, rxtx, handshake, warn)
                        },
                    );

                let state_style = if conn
                    .status
                    .as_ref()
                    .map_or(false, |s| s.state == ConnectionState::Connected)
                {
                    Style::default().fg(theme.success)
                } else {
                    Style::default().fg(theme.muted)
                };

                Row::new(vec![
                    Cell::from(conn.name.clone()),
                    Cell::from(state_str).style(state_style),
                    Cell::from(backend_str),
                    Cell::from(endpoint),
                    Cell::from(interface),
                    Cell::from(rx_tx),
                    Cell::from(hs),
                    Cell::from(warning),
                ])
            })
            .collect()
    }
}

impl Default for StatusComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for StatusComponent {
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action> {
        let selected_row = self.table_state.selected().unwrap_or(0);
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => Some(Action::NextRow),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::PrevRow),
            KeyCode::Enter | KeyCode::Char('u') => {
                Self::selected_connection_name(state, selected_row).map(Action::ConnectPeer)
            }
            KeyCode::Char('d') => {
                Self::selected_connection_name(state, selected_row).map(Action::DisconnectPeer)
            }
            KeyCode::Char('b') => {
                Self::selected_connection_name(state, selected_row).map(Action::CyclePeerBackend)
            }
            _ => None,
        }
    }

    fn update(&mut self, action: &Action, state: &AppState) {
        let max = Self::row_count(state).saturating_sub(1);
        let current = self.table_state.selected().unwrap_or(0);

        match action {
            Action::NextRow => {
                self.table_state
                    .select(Some(current.saturating_add(1).min(max)));
            }
            Action::PrevRow => {
                self.table_state.select(Some(current.saturating_sub(1)));
            }
            Action::SelectTab(_) | Action::NextTab | Action::PrevTab => {
                self.table_state.select(Some(0));
            }
            _ => {}
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        if area.height == 0 || area.width < 20 {
            return;
        }
        let theme = &state.theme;

        if Self::row_count(state) == 0 {
            let para = Paragraph::new("No visible connections.")
                .block(theme.panel_block("Status"))
                .style(Style::default().fg(theme.muted));
            frame.render_widget(para, area);
            return;
        }

        // Render the outer border once; work inside the inner area.
        let block = theme.panel_block("Status");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // ── Connection table ──────────────────────────────────────────────────
        let header = Row::new(vec![
            "Name",
            "Status",
            "Backend",
            "Endpoint",
            "Interface",
            "Rx / Tx",
            "Handshake",
            "Warnings",
        ])
        .style(theme.header_style());

        let rows = Self::connection_rows(state, theme);

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(COL_NAME_W),
                Constraint::Percentage(COL_STATUS_W),
                Constraint::Percentage(COL_BACKEND_W),
                Constraint::Percentage(COL_ENDPOINT_W),
                Constraint::Percentage(COL_INTERFACE_W),
                Constraint::Percentage(COL_RXTX_W),
                Constraint::Percentage(COL_HANDSHAKE_W),
                Constraint::Percentage(COL_WARNINGS_W),
            ],
        )
        .header(header)
        .row_highlight_style(theme.highlight_style());

        frame.render_stateful_widget(table, inner, &mut self.table_state);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;
    use ferro_wg_core::config::{AppConfig, InterfaceConfig, PeerConfig, WgConfig};
    use ferro_wg_core::error::BackendKind;
    use ferro_wg_core::key::PrivateKey;
    use ferro_wg_core::stats::TunnelStats;
    use ferro_wg_tui_core::{ConnectionState, ConnectionStatus, Tab};

    fn render_status(state: &AppState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let mut comp = StatusComponent::new();
                comp.render(frame, frame.area(), true, state);
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

    fn make_app_config_with_peers(peers: Vec<PeerConfig>) -> AppConfig {
        let mut connections = BTreeMap::new();
        connections.insert(
            "test".to_string(),
            WgConfig {
                interface: InterfaceConfig {
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
                },
                peers,
            },
        );
        AppConfig {
            connections,
            ..AppConfig::default()
        }
    }

    fn test_state() -> AppState {
        AppState::new(make_app_config_with_peers(vec![
            PeerConfig {
                name: "peer-a".into(),
                public_key: PrivateKey::generate().public_key(),
                preshared_key: None,
                endpoint: Some("1.2.3.4:51820".into()),
                allowed_ips: vec!["10.0.0.0/24".into()],
                persistent_keepalive: 25,
            },
            PeerConfig {
                name: "peer-b".into(),
                public_key: PrivateKey::generate().public_key(),
                preshared_key: None,
                endpoint: Some("5.6.7.8:51820".into()),
                allowed_ips: vec!["10.0.1.0/24".into()],
                persistent_keepalive: 25,
            },
        ]))
    }

    // ── Column percentage validation ───────────────────────────────────────────

    #[test]
    fn status_column_widths_sum_to_100() {
        assert_eq!(
            COL_NAME_W
                + COL_STATUS_W
                + COL_BACKEND_W
                + COL_ENDPOINT_W
                + COL_INTERFACE_W
                + COL_RXTX_W
                + COL_HANDSHAKE_W
                + COL_WARNINGS_W,
            100,
            "status column percentages must sum to 100"
        );
    }

    #[test]
    fn handle_key_row_navigation() {
        let mut comp = StatusComponent::new();
        let state = test_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Down), &state),
            Some(Action::NextRow)
        );
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('k')), &state),
            Some(Action::PrevRow)
        );
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('x')), &state),
            None
        );
    }

    #[test]
    fn update_clamps_rows() {
        let mut comp = StatusComponent::new();
        let state = test_state(); // 2 peers
        comp.update(&Action::NextRow, &state);
        assert_eq!(comp.table_state.selected(), Some(1));
        comp.update(&Action::NextRow, &state);
        assert_eq!(comp.table_state.selected(), Some(1)); // clamped
    }

    #[test]
    fn update_resets_on_tab_change() {
        let mut comp = StatusComponent::new();
        let state = test_state();
        comp.update(&Action::NextRow, &state);
        assert_eq!(comp.table_state.selected(), Some(1));
        comp.update(&Action::SelectTab(Tab::Peers), &state);
        assert_eq!(comp.table_state.selected(), Some(0));
    }

    // ── Multi-peer render tests ───────────────────────────────────────────────

    #[test]
    fn status_renders_both_peer_names() {
        let state = test_state(); // peer-a and peer-b
        let content = render_status(&state, 120, 20);
        assert!(
            content.contains("peer-a"),
            "expected 'peer-a' in: {content:?}"
        );
        assert!(
            content.contains("peer-b"),
            "expected 'peer-b' in: {content:?}"
        );
    }

    #[test]
    fn status_renders_distinct_peer_endpoints() {
        let state = test_state();
        let content = render_status(&state, 120, 20);
        assert!(
            content.contains("1.2.3.4"),
            "expected endpoint '1.2.3.4' in: {content:?}"
        );
        assert!(
            content.contains("5.6.7.8"),
            "expected endpoint '5.6.7.8' in: {content:?}"
        );
    }

    #[test]
    fn status_renders_distinct_allowed_ips() {
        let state = test_state();
        let content = render_status(&state, 120, 20);
        assert!(
            content.contains("10.0.0.0/24"),
            "expected '10.0.0.0/24' in: {content:?}"
        );
        assert!(
            content.contains("10.0.1.0/24"),
            "expected '10.0.1.0/24' in: {content:?}"
        );
    }

    #[test]
    fn status_renders_nonzero_keepalive_as_seconds() {
        // test_state has persistent_keepalive=25 for both peers
        let state = test_state();
        let content = render_status(&state, 120, 20);
        assert!(
            content.contains("25s"),
            "expected '25s' keepalive in: {content:?}"
        );
    }

    #[test]
    fn status_renders_zero_keepalive_as_off() {
        let state = AppState::new(make_app_config_with_peers(vec![PeerConfig {
            name: "peer-zero".into(),
            public_key: PrivateKey::generate().public_key(),
            preshared_key: None,
            endpoint: Some("9.9.9.9:51820".into()),
            allowed_ips: vec!["192.168.0.0/24".into()],
            persistent_keepalive: 0,
        }]));
        let content = render_status(&state, 120, 20);
        assert!(
            content.contains("off"),
            "expected 'off' for zero keepalive in: {content:?}"
        );
    }

    #[test]
    fn status_connected_summary_shows_connected() {
        let mut state = test_state();
        state.connections[0].status = Some(ConnectionStatus {
            state: ConnectionState::Connected,
            backend: BackendKind::Boringtun,
            stats: TunnelStats::default(),
            endpoint: None,
            interface: Some("utun4".into()),
            health_warning: None,
        });
        let content = render_status(&state, 120, 20);
        assert!(
            content.contains("connected"),
            "expected 'connected' in summary: {content:?}"
        );
    }

    #[test]
    fn status_no_status_shows_down() {
        // All connections have status: None — summary must read "down".
        let state = test_state();
        let content = render_status(&state, 120, 20);
        assert!(
            content.contains("down"),
            "expected 'down' in summary: {content:?}"
        );
    }

    #[test]
    fn status_empty_config_shows_no_visible_connections_message() {
        let state = AppState::new(AppConfig::default());
        let content = render_status(&state, 120, 20);
        assert!(
            content.contains("No visible connections"),
            "expected 'No visible connections' in: {content:?}"
        );
    }

    // ── Commit 5: health indicators ──────────────────────────────────────────

    #[test]
    fn status_health_warning_renders_in_summary() {
        let mut state = test_state();
        state.connections[0].status = Some(ConnectionStatus {
            state: ConnectionState::Connected,
            backend: BackendKind::Boringtun,
            stats: TunnelStats::default(),
            endpoint: None,
            interface: Some("utun4".into()),
            health_warning: Some("stale handshake".into()),
        });
        let content = render_status(&state, 120, 20);
        assert!(
            content.contains("[!]"),
            "expected '[!]' warning indicator in: {content:?}"
        );
        assert!(
            content.contains("stale handshake"),
            "expected warning text in: {content:?}"
        );
    }

    #[test]
    fn status_no_warning_when_healthy() {
        let mut state = test_state();
        state.connections[0].status = Some(ConnectionStatus {
            state: ConnectionState::Connected,
            backend: BackendKind::Boringtun,
            stats: TunnelStats::default(),
            endpoint: None,
            interface: Some("utun4".into()),
            health_warning: None,
        });
        let content = render_status(&state, 120, 20);
        assert!(
            !content.contains("[!]"),
            "expected no '[!]' for healthy connection in: {content:?}"
        );
    }

    // ── Narrow-terminal edge cases ────────────────────────────────────────────

    #[test]
    fn status_narrow_terminal_no_panic() {
        let state = test_state();
        render_status(&state, 10, 5);
    }

    #[test]
    fn status_minimal_terminal_no_panic() {
        let state = test_state();
        render_status(&state, 1, 1);
    }

    #[test]
    fn status_zero_height_no_panic() {
        // ratatui clamps to a minimum; must not panic.
        let state = test_state();
        render_status(&state, 80, 0);
    }
}
