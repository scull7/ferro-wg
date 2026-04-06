//! Overview tab: aggregate health table across all connections.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Cell, Row, Table, TableState};

use ferro_wg_tui_core::{
    Action, AppState, Component, ConfirmAction, ConnectionState, Tab, format_bytes,
    format_handshake_age,
};

// Overview table column widths. Must sum to 100 for the percentage columns
// (the `#` column uses a fixed Length(3) that ratatui accounts for separately).
const COL_INDEX_W: u16 = 3; // "#"          fixed chars
const COL_NAME_W: u16 = 18; // "Name"       %
const COL_STATUS_W: u16 = 17; // "Status"    %
const COL_BACKEND_W: u16 = 12; // "Backend"  %
const COL_IFACE_W: u16 = 12; // "Interface"  %
const COL_TX_W: u16 = 10; // "Tx"           %
const COL_RX_W: u16 = 10; // "Rx"           %
const COL_HANDSHAKE_W: u16 = 21; // "Last Handshake" %

const _: () = assert!(
    COL_NAME_W + COL_STATUS_W + COL_BACKEND_W + COL_IFACE_W + COL_TX_W + COL_RX_W + COL_HANDSHAKE_W
        == 100,
    "Overview percentage columns must sum to 100"
);

/// Aggregate health table showing all configured connections at a glance.
///
/// One row per connection with live stats sourced from the last daemon
/// poll. The highlighted row tracks `state.selected_connection`; row
/// navigation dispatches `SelectConnection(i)`.
pub struct OverviewComponent {
    /// Table selection state, kept in sync with `state.selected_connection`.
    table_state: TableState,
}

impl OverviewComponent {
    /// Create a new overview component.
    #[must_use]
    pub fn new() -> Self {
        Self {
            table_state: TableState::default().with_selected(Some(0)),
        }
    }
}

impl Default for OverviewComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for OverviewComponent {
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action> {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') if !state.connections.is_empty() => {
                let next = (state.selected_connection + 1) % state.connections.len();
                Some(Action::SelectConnection(next))
            }
            KeyCode::Up | KeyCode::Char('k') if !state.connections.is_empty() => {
                let prev = state
                    .selected_connection
                    .checked_sub(1)
                    .unwrap_or(state.connections.len() - 1);
                Some(Action::SelectConnection(prev))
            }
            KeyCode::Enter => Some(Action::SelectTab(Tab::Status)),
            // Bulk connection control (daemon must be connected to be useful,
            // but we don't gate here — feedback from the daemon handles errors).
            KeyCode::Char('u') => Some(Action::ConnectAll),
            KeyCode::Char('d') => Some(Action::RequestConfirm {
                message: "Tear down all connections?".to_owned(),
                action: ConfirmAction::DisconnectAll,
            }),
            // Daemon lifecycle: s starts (only when disconnected), S stops (only when connected).
            KeyCode::Char('s') if !state.daemon_connected => Some(Action::StartDaemon),
            KeyCode::Char('S') if state.daemon_connected => Some(Action::RequestConfirm {
                message: "Stop the running daemon?".to_owned(),
                action: ConfirmAction::StopDaemon,
            }),
            _ => None,
        }
    }

    fn update(&mut self, _action: &Action, state: &AppState) {
        // Keep the table cursor authoritative from state, not local tracking.
        self.table_state.select(Some(state.selected_connection));
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        if area.height == 0 || area.width < 20 {
            return;
        }
        let theme = &state.theme;

        // Sync cursor (defensive; update() handles it on every dispatch, but
        // render may run before the first dispatch on a new state).
        self.table_state.select(Some(state.selected_connection));

        let header = Row::new(vec![
            "#",
            "Name",
            "Status",
            "Backend",
            "Interface",
            "Tx",
            "Rx",
            "Last Handshake",
        ])
        .style(theme.header_style());

        let rows: Vec<Row<'_>> = state
            .connections
            .iter()
            .enumerate()
            .map(|(i, conn)| {
                // Bind once; avoids repeated .as_ref() calls for each column.
                let status_opt = conn.status.as_ref();

                let (status_str, status_style): (&'static str, Style) = match status_opt {
                    None => ("—", Style::default().fg(theme.muted)),
                    Some(s)
                        if s.state == ConnectionState::Connected && s.health_warning.is_none() =>
                    {
                        ("● Connected", Style::default().fg(theme.success))
                    }
                    Some(s) if s.state == ConnectionState::Connected => {
                        ("● Connected [!]", Style::default().fg(theme.warning))
                    }
                    Some(_) => ("○ Disconnected", Style::default().fg(theme.muted)),
                };

                let backend = status_opt.map_or_else(|| "—".to_owned(), |s| s.backend.to_string());

                let interface: &str = status_opt
                    .and_then(|s| s.interface.as_deref())
                    .unwrap_or("—");

                let tx =
                    status_opt.map_or_else(|| "—".to_owned(), |s| format_bytes(s.stats.tx_bytes));

                let rx =
                    status_opt.map_or_else(|| "—".to_owned(), |s| format_bytes(s.stats.rx_bytes));

                let hs = status_opt.map_or_else(
                    || "—".to_owned(),
                    |s| {
                        s.stats
                            .last_handshake
                            .map_or_else(|| "—".to_owned(), format_handshake_age)
                    },
                );

                Row::new(vec![
                    Cell::from(format!("{}", i + 1)),
                    Cell::from(conn.name.as_str()),
                    Cell::from(status_str).style(status_style),
                    Cell::from(backend),
                    Cell::from(interface),
                    Cell::from(tx),
                    Cell::from(rx),
                    Cell::from(hs),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(COL_INDEX_W),
                Constraint::Percentage(COL_NAME_W),
                Constraint::Percentage(COL_STATUS_W),
                Constraint::Percentage(COL_BACKEND_W),
                Constraint::Percentage(COL_IFACE_W),
                Constraint::Percentage(COL_TX_W),
                Constraint::Percentage(COL_RX_W),
                Constraint::Percentage(COL_HANDSHAKE_W),
            ],
        )
        .header(header)
        .block(theme.panel_block("Overview"))
        .row_highlight_style(theme.highlight_style());

        frame.render_stateful_widget(table, area, &mut self.table_state);
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
            peers: vec![PeerConfig {
                name: "dc".into(),
                public_key: PrivateKey::generate().public_key(),
                preshared_key: None,
                endpoint: Some("1.2.3.4:51820".into()),
                allowed_ips: vec!["10.0.0.0/8".into()],
                persistent_keepalive: 25,
            }],
        }
    }

    fn three_connection_state() -> AppState {
        let mut connections = BTreeMap::new();
        connections.insert("mia".to_string(), make_wg_config());
        connections.insert("ord01".to_string(), make_wg_config());
        connections.insert("tus1".to_string(), make_wg_config());
        AppState::new(AppConfig {
            connections,
            ..AppConfig::default()
        })
    }

    fn connected_status() -> ConnectionStatus {
        ConnectionStatus {
            state: ConnectionState::Connected,
            backend: BackendKind::Boringtun,
            stats: TunnelStats::default(),
            endpoint: Some("1.2.3.4:51820".into()),
            interface: Some("utun4".into()),
            health_warning: None,
        }
    }

    fn render_overview(state: &AppState) -> String {
        render_overview_sized(state, 120, 20)
    }

    fn render_overview_sized(state: &AppState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let mut comp = OverviewComponent::new();
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

    // ── Column percentage validation ───────────────────────────────────────────

    #[test]
    fn overview_column_widths_sum_to_100() {
        assert_eq!(
            COL_NAME_W
                + COL_STATUS_W
                + COL_BACKEND_W
                + COL_IFACE_W
                + COL_TX_W
                + COL_RX_W
                + COL_HANDSHAKE_W,
            100,
            "overview column percentages must sum to 100"
        );
    }

    #[test]
    fn overview_renders_all_connections() {
        let state = three_connection_state();
        let content = render_overview(&state);
        assert!(content.contains("mia"), "expected 'mia' in: {content:?}");
        assert!(
            content.contains("ord01"),
            "expected 'ord01' in: {content:?}"
        );
        assert!(content.contains("tus1"), "expected 'tus1' in: {content:?}");
    }

    #[test]
    fn overview_empty_config() {
        let state = AppState::new(AppConfig::default());
        // Must not panic with an empty connection list.
        let content = render_overview(&state);
        assert!(content.contains("Overview"));
    }

    #[test]
    fn overview_shows_not_polled_placeholder() {
        let state = three_connection_state();
        // All connections have status: None — every data column shows "—".
        let content = render_overview(&state);
        assert!(
            content.contains('—'),
            "expected '—' placeholder in: {content:?}"
        );
    }

    #[test]
    fn overview_connected_shows_filled_circle() {
        let mut state = three_connection_state();
        state.connections[0].status = Some(connected_status());
        let content = render_overview(&state);
        assert!(
            content.contains('●'),
            "expected '●' for connected in: {content:?}"
        );
    }

    #[test]
    fn overview_disconnected_shows_open_circle() {
        let mut state = three_connection_state();
        state.connections[0].status = Some(ConnectionStatus {
            state: ConnectionState::Disconnected,
            backend: BackendKind::Boringtun,
            stats: TunnelStats::default(),
            endpoint: None,
            interface: None,
            health_warning: None,
        });
        let content = render_overview(&state);
        assert!(
            content.contains('○'),
            "expected '○' for disconnected in: {content:?}"
        );
    }

    #[test]
    fn overview_key_down_dispatches_select_next() {
        let mut comp = OverviewComponent::new();
        let state = three_connection_state(); // selected_connection = 0
        let action = comp.handle_key(KeyEvent::from(KeyCode::Down), &state);
        assert_eq!(action, Some(Action::SelectConnection(1)));
    }

    #[test]
    fn overview_key_down_wraps() {
        let mut comp = OverviewComponent::new();
        let mut state = three_connection_state();
        state.selected_connection = 2; // last index (3 connections)
        let action = comp.handle_key(KeyEvent::from(KeyCode::Down), &state);
        assert_eq!(action, Some(Action::SelectConnection(0)));
    }

    #[test]
    fn overview_key_up_wraps() {
        let mut comp = OverviewComponent::new();
        let mut state = three_connection_state();
        state.selected_connection = 0;
        let action = comp.handle_key(KeyEvent::from(KeyCode::Up), &state);
        assert_eq!(action, Some(Action::SelectConnection(2)));
    }

    #[test]
    fn overview_enter_switches_tab() {
        let mut comp = OverviewComponent::new();
        let state = three_connection_state();
        let action = comp.handle_key(KeyEvent::from(KeyCode::Enter), &state);
        assert_eq!(action, Some(Action::SelectTab(Tab::Status)));
    }

    #[test]
    fn overview_empty_key_down_no_panic() {
        let mut comp = OverviewComponent::new();
        let state = AppState::new(AppConfig::default());
        let action = comp.handle_key(KeyEvent::from(KeyCode::Down), &state);
        assert_eq!(action, None);
    }

    #[test]
    fn overview_update_syncs_table_cursor() {
        let mut comp = OverviewComponent::new();
        let mut state = three_connection_state();
        state.selected_connection = 2;
        comp.update(&Action::SelectConnection(2), &state);
        assert_eq!(comp.table_state.selected(), Some(2));
    }

    // ── Narrow-terminal edge cases ────────────────────────────────────────────

    #[test]
    fn overview_narrow_terminal_no_panic() {
        let state = three_connection_state();
        render_overview_sized(&state, 10, 5);
    }

    #[test]
    fn overview_minimal_terminal_no_panic() {
        let state = three_connection_state();
        render_overview_sized(&state, 1, 1);
    }

    #[test]
    fn overview_zero_height_no_panic() {
        // ratatui clamps to a minimum; must not panic.
        let state = three_connection_state();
        render_overview_sized(&state, 80, 0);
    }

    // ── Unicode connection name tests ─────────────────────────────────────────

    #[test]
    fn overview_cjk_connection_names_no_panic() {
        let mut connections = BTreeMap::new();
        connections.insert("東京-vps".to_string(), make_wg_config());
        connections.insert("大阪-cdn".to_string(), make_wg_config());
        let state = AppState::new(AppConfig {
            connections,
            ..AppConfig::default()
        });
        let content = render_overview(&state);
        assert!(content.contains("Overview"), "panel title must be present");
    }

    #[test]
    fn overview_emoji_connection_names_no_panic() {
        let mut connections = BTreeMap::new();
        connections.insert("🌐-global".to_string(), make_wg_config());
        connections.insert("🔒-vpn".to_string(), make_wg_config());
        let state = AppState::new(AppConfig {
            connections,
            ..AppConfig::default()
        });
        render_overview(&state); // must not panic
    }

    // ── Extreme connection count tests ────────────────────────────────────────

    #[test]
    fn overview_hundred_connections_no_panic() {
        let mut connections = BTreeMap::new();
        for i in 0..100 {
            connections.insert(format!("conn{i:03}"), make_wg_config());
        }
        let state = AppState::new(AppConfig {
            connections,
            ..AppConfig::default()
        });
        render_overview(&state); // must not panic
    }

    #[test]
    fn overview_hundred_connections_selected_last_no_panic() {
        let mut connections = BTreeMap::new();
        for i in 0..100 {
            connections.insert(format!("conn{i:03}"), make_wg_config());
        }
        let mut state = AppState::new(AppConfig {
            connections,
            ..AppConfig::default()
        });
        state.selected_connection = 99;
        render_overview(&state); // must not panic
    }

    // ── Commit 5: health indicators ──────────────────────────────────────────

    #[test]
    fn overview_health_warning_shows_exclamation() {
        let mut state = three_connection_state();
        state.connections[0].status = Some(ConnectionStatus {
            state: ConnectionState::Connected,
            backend: BackendKind::Boringtun,
            stats: TunnelStats::default(),
            endpoint: None,
            interface: None,
            health_warning: Some("stale handshake".into()),
        });
        let content = render_overview(&state);
        assert!(
            content.contains("[!]"),
            "expected '[!]' health indicator in: {content:?}"
        );
    }

    #[test]
    fn overview_healthy_connected_shows_no_exclamation() {
        let mut state = three_connection_state();
        state.connections[0].status = Some(connected_status()); // health_warning: None
        let content = render_overview(&state);
        assert!(
            !content.contains("[!]"),
            "expected no '[!]' for healthy connection in: {content:?}"
        );
    }

    // ── Commit 2: bulk connection keybindings ─────────────────────────────────

    #[test]
    fn overview_u_emits_connect_all() {
        let mut comp = OverviewComponent::new();
        let state = three_connection_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('u')), &state),
            Some(Action::ConnectAll),
        );
    }

    #[test]
    fn overview_d_emits_request_confirm_disconnect_all() {
        use ferro_wg_tui_core::ConfirmAction;
        let mut comp = OverviewComponent::new();
        let state = three_connection_state();
        let action = comp.handle_key(KeyEvent::from(KeyCode::Char('d')), &state);
        assert!(
            matches!(
                action,
                Some(Action::RequestConfirm {
                    action: ConfirmAction::DisconnectAll,
                    ..
                })
            ),
            "expected RequestConfirm(DisconnectAll), got {action:?}"
        );
    }

    #[test]
    fn overview_u_works_on_empty_connections() {
        let mut comp = OverviewComponent::new();
        let state = AppState::new(AppConfig::default());
        // ConnectAll should still be emitted even with no connections (daemon handles it).
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('u')), &state),
            Some(Action::ConnectAll),
        );
    }

    // ── Commit 3: daemon lifecycle keybindings ────────────────────────────────

    #[test]
    fn overview_s_emits_start_daemon_when_disconnected() {
        let mut comp = OverviewComponent::new();
        let mut state = three_connection_state();
        state.daemon_connected = false;
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('s')), &state),
            Some(Action::StartDaemon),
        );
    }

    #[test]
    fn overview_s_ignored_when_connected() {
        let mut comp = OverviewComponent::new();
        let mut state = three_connection_state();
        state.daemon_connected = true;
        // lowercase 's' should not produce StartDaemon when already connected
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('s')), &state),
            None,
        );
    }

    #[test]
    fn overview_shift_s_emits_request_confirm_stop_daemon_when_connected() {
        use ferro_wg_tui_core::ConfirmAction;
        let mut comp = OverviewComponent::new();
        let mut state = three_connection_state();
        state.daemon_connected = true;
        let action = comp.handle_key(KeyEvent::from(KeyCode::Char('S')), &state);
        assert!(
            matches!(
                action,
                Some(Action::RequestConfirm {
                    action: ConfirmAction::StopDaemon,
                    ..
                })
            ),
            "expected RequestConfirm(StopDaemon), got {action:?}"
        );
    }

    #[test]
    fn overview_shift_s_ignored_when_disconnected() {
        let mut comp = OverviewComponent::new();
        let mut state = three_connection_state();
        state.daemon_connected = false;
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('S')), &state),
            None,
        );
    }
}
