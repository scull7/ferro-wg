//! Status tab: active tunnel overview.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Cell, Row, Table, TableState};

use ferro_wg_tui_core::{
    Action, AppState, Component, ConnectionState, format_bytes, format_handshake_age,
};

/// Active tunnels overview showing connection state, traffic, and
/// handshake age for each peer.
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
            let para = ratatui::widgets::Paragraph::new("No connections configured.")
                .block(theme.panel_block("Status"))
                .style(Style::default().fg(theme.muted));
            frame.render_widget(para, area);
            return;
        };

        // Connection-level status and stats.
        let (is_connected, hs, rx, tx) = conn.status.as_ref().map_or(
            (false, "-".to_owned(), "-".to_owned(), "-".to_owned()),
            |s| {
                let hs = s
                    .stats
                    .last_handshake
                    .map_or_else(|| "-".to_owned(), format_handshake_age);
                let rx = format_bytes(s.stats.rx_bytes);
                let tx = format_bytes(s.stats.tx_bytes);
                (s.state == ConnectionState::Connected, hs, rx, tx)
            },
        );

        let header = Row::new(vec!["Peer", "Endpoint", "Status", "Rx", "Tx", "Handshake"])
            .style(theme.header_style());

        let rows: Vec<Row<'static>> = state
            .filtered_peers()
            .map(|p| {
                let (status_str, status_style): (String, Style) = if is_connected {
                    ("connected".into(), Style::default().fg(theme.success))
                } else {
                    ("down".into(), Style::default().fg(theme.muted))
                };

                Row::new(vec![
                    Cell::from(p.name.clone()),
                    Cell::from(p.endpoint.clone().unwrap_or_else(|| "-".into())),
                    Cell::from(status_str).style(status_style),
                    Cell::from(rx.clone()),
                    Cell::from(tx.clone()),
                    Cell::from(hs.clone()),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(20),
                Constraint::Percentage(25),
                Constraint::Percentage(12),
                Constraint::Percentage(12),
                Constraint::Percentage(12),
                Constraint::Percentage(19),
            ],
        )
        .header(header)
        .block(theme.panel_block("Status"))
        .row_highlight_style(theme.highlight_style());

        frame.render_stateful_widget(table, area, &mut self.table_state);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use ferro_wg_core::config::{AppConfig, InterfaceConfig, PeerConfig, WgConfig};
    use ferro_wg_core::key::PrivateKey;
    use ferro_wg_tui_core::Tab;

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
}
