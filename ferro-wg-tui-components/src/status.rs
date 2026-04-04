//! Status tab: active tunnel overview.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use ferro_wg_tui_core::{
    Action, AppState, Component, ConnectionState, format_bytes, format_handshake_age,
};

// Status tab peer table column widths (percentages). Must sum to 100.
const COL_PEER_W: u16 = 25; // "Peer"
const COL_ENDPOINT_W: u16 = 30; // "Endpoint"
const COL_ALLOWED_W: u16 = 30; // "Allowed IPs"
const COL_KEEPALIVE_W: u16 = 15; // "Keepalive"

const _: () = assert!(
    COL_PEER_W + COL_ENDPOINT_W + COL_ALLOWED_W + COL_KEEPALIVE_W == 100,
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

    /// Number of displayable rows (filtered peers of the active connection).
    fn row_count(state: &AppState) -> usize {
        state.filtered_peers().count()
    }

    /// Get the name of the active connection, if any.
    fn active_connection_name(state: &AppState) -> Option<String> {
        state.active_connection().map(|c| c.name.clone())
    }
}

impl Default for StatusComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for StatusComponent {
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action> {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => Some(Action::NextRow),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::PrevRow),
            KeyCode::Enter | KeyCode::Char('u') => {
                Self::active_connection_name(state).map(Action::ConnectPeer)
            }
            KeyCode::Char('d') => Self::active_connection_name(state).map(Action::DisconnectPeer),
            KeyCode::Char('b') => Self::active_connection_name(state).map(Action::CyclePeerBackend),
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
        let theme = &state.theme;

        let Some(conn) = state.active_connection() else {
            let para = Paragraph::new("No connections configured.")
                .block(theme.panel_block("Status"))
                .style(Style::default().fg(theme.muted));
            frame.render_widget(para, area);
            return;
        };

        // Render the outer border once; work inside the inner area.
        let block = theme.panel_block("Status");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Split inner area: 2-line connection summary, then the peer table.
        let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(inner);

        // ── Connection-level summary ──────────────────────────────────────────
        let (is_connected, state_str, hs, rx, tx, backend_str, iface_str) =
            conn.status.as_ref().map_or(
                (
                    false,
                    "down",
                    "-".to_owned(),
                    "-".to_owned(),
                    "-".to_owned(),
                    "-".to_owned(),
                    "-".to_owned(),
                ),
                |s| {
                    let connected = s.state == ConnectionState::Connected;
                    let hs = s
                        .stats
                        .last_handshake
                        .map_or_else(|| "-".to_owned(), format_handshake_age);
                    let rx = format_bytes(s.stats.rx_bytes);
                    let tx = format_bytes(s.stats.tx_bytes);
                    let backend = s.backend.to_string();
                    let iface = s.interface.clone().unwrap_or_else(|| "-".to_owned());
                    (connected, if connected { "connected" } else { "down" }, hs, rx, tx, backend, iface)
                },
            );

        let state_style = if is_connected {
            Style::default().fg(theme.success)
        } else {
            Style::default().fg(theme.muted)
        };
        let label = Style::default().fg(theme.muted);

        let summary = Paragraph::new(vec![
            Line::from(vec![
                Span::styled("State: ", label),
                Span::styled(state_str, state_style),
                Span::raw("  "),
                Span::styled("Backend: ", label),
                Span::raw(backend_str),
                Span::raw("  "),
                Span::styled("Interface: ", label),
                Span::raw(iface_str),
            ]),
            Line::from(vec![
                Span::styled("Rx: ", label),
                Span::raw(rx),
                Span::raw("  "),
                Span::styled("Tx: ", label),
                Span::raw(tx),
                Span::raw("  "),
                Span::styled("Handshake: ", label),
                Span::raw(hs),
                Span::styled("  (connection totals, not per-peer)", label),
            ]),
        ]);
        frame.render_widget(summary, chunks[0]);

        // ── Per-peer table ────────────────────────────────────────────────────
        let header = Row::new(vec!["Peer", "Endpoint", "Allowed IPs", "Keepalive"])
            .style(theme.header_style());

        let rows: Vec<Row<'_>> = state
            .filtered_peers()
            .map(|p| {
                let allowed = p.allowed_ips.join(", ");
                let keepalive = if p.persistent_keepalive == 0 {
                    "off".to_owned()
                } else {
                    format!("{}s", p.persistent_keepalive)
                };
                Row::new(vec![
                    Cell::from(p.name.clone()),
                    Cell::from(p.endpoint.clone().unwrap_or_else(|| "-".to_owned())),
                    Cell::from(allowed),
                    Cell::from(keepalive),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(COL_PEER_W),
                Constraint::Percentage(COL_ENDPOINT_W),
                Constraint::Percentage(COL_ALLOWED_W),
                Constraint::Percentage(COL_KEEPALIVE_W),
            ],
        )
        .header(header)
        .row_highlight_style(theme.highlight_style());

        frame.render_stateful_widget(table, chunks[1], &mut self.table_state);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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
        AppConfig { connections }
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
        assert!(content.contains("peer-a"), "expected 'peer-a' in: {content:?}");
        assert!(content.contains("peer-b"), "expected 'peer-b' in: {content:?}");
    }

    #[test]
    fn status_renders_distinct_peer_endpoints() {
        let state = test_state();
        let content = render_status(&state, 120, 20);
        assert!(content.contains("1.2.3.4"), "expected endpoint '1.2.3.4' in: {content:?}");
        assert!(content.contains("5.6.7.8"), "expected endpoint '5.6.7.8' in: {content:?}");
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
        assert!(content.contains("25s"), "expected '25s' keepalive in: {content:?}");
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
        assert!(content.contains("off"), "expected 'off' for zero keepalive in: {content:?}");
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
        assert!(content.contains("down"), "expected 'down' in summary: {content:?}");
    }

    #[test]
    fn status_empty_config_shows_no_connections_message() {
        let state = AppState::new(AppConfig::default());
        let content = render_status(&state, 120, 20);
        assert!(
            content.contains("No connections configured"),
            "expected 'No connections configured' in: {content:?}"
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
