//! Overview tab: aggregate health table across all connections.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Cell, Row, Table, TableState};

use ferro_wg_tui_core::{
    Action, AppState, Component, ConnectionState, Tab, format_bytes, format_handshake_age,
};

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
        if state.connections.is_empty() {
            return None;
        }
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                let next = (state.selected_connection + 1) % state.connections.len();
                Some(Action::SelectConnection(next))
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let prev = state
                    .selected_connection
                    .checked_sub(1)
                    .unwrap_or(state.connections.len() - 1);
                Some(Action::SelectConnection(prev))
            }
            KeyCode::Enter => Some(Action::SelectTab(Tab::Status)),
            _ => None,
        }
    }

    fn update(&mut self, _action: &Action, state: &AppState) {
        // Keep the table cursor authoritative from state, not local tracking.
        self.table_state.select(Some(state.selected_connection));
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
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
                let (status_str, status_style): (&'static str, Style) = match &conn.status {
                    None => ("—", Style::default().fg(theme.muted)),
                    Some(s) if s.state == ConnectionState::Connected => {
                        ("● Connected", Style::default().fg(theme.success))
                    }
                    Some(_) => ("○ Disconnected", Style::default().fg(theme.muted)),
                };

                let backend = conn
                    .status
                    .as_ref()
                    .map_or_else(|| "—".to_owned(), |s| s.backend.to_string());

                let interface = conn
                    .status
                    .as_ref()
                    .and_then(|s| s.interface.clone())
                    .unwrap_or_else(|| "—".to_owned());

                let tx = conn
                    .status
                    .as_ref()
                    .map_or_else(|| "—".to_owned(), |s| format_bytes(s.stats.tx_bytes));

                let rx = conn
                    .status
                    .as_ref()
                    .map_or_else(|| "—".to_owned(), |s| format_bytes(s.stats.rx_bytes));

                let hs = conn.status.as_ref().map_or_else(
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
                Constraint::Length(3),
                Constraint::Percentage(18),
                Constraint::Percentage(17),
                Constraint::Percentage(12),
                Constraint::Percentage(12),
                Constraint::Percentage(10),
                Constraint::Percentage(10),
                Constraint::Percentage(21),
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
        AppState::new(AppConfig { connections })
    }

    fn connected_status() -> ConnectionStatus {
        ConnectionStatus {
            state: ConnectionState::Connected,
            backend: BackendKind::Boringtun,
            stats: TunnelStats::default(),
            endpoint: Some("1.2.3.4:51820".into()),
            interface: Some("utun4".into()),
        }
    }

    fn render_overview(state: &AppState) -> String {
        let backend = TestBackend::new(120, 20);
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
}
