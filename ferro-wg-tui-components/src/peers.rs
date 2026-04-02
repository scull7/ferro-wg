//! Peers tab: all configured peers with routing and backend details.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::{Cell, Row, Table, TableState};

use ferro_wg_tui_core::{Action, AppState, Component};

/// Peer configuration table showing public keys, endpoints, allowed
/// IPs, keepalive intervals, and active backends.
pub struct PeersComponent {
    /// Per-component table selection state.
    table_state: TableState,
}

impl PeersComponent {
    /// Create a new peers component.
    #[must_use]
    pub fn new() -> Self {
        Self {
            table_state: TableState::default().with_selected(Some(0)),
        }
    }

    /// Number of displayable rows (filtered peers).
    fn row_count(state: &AppState) -> usize {
        state.filtered_peers().count()
    }
}

impl Default for PeersComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for PeersComponent {
    fn handle_key(&mut self, key: KeyEvent, _state: &AppState) -> Option<Action> {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => Some(Action::NextRow),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::PrevRow),
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

        let header = Row::new(vec![
            "Peer",
            "Public Key",
            "Endpoint",
            "Allowed IPs",
            "Keepalive",
            "Backend",
        ])
        .style(theme.header_style());

        let rows: Vec<Row<'static>> = state
            .filtered_peers()
            .map(|p| {
                let pk = p.config.public_key.to_base64();
                let short_pk = format!("{}...", &pk[..10]);
                let name = p.config.name.clone();
                let endpoint = p.config.endpoint.clone().unwrap_or_else(|| "-".into());
                let allowed = p.config.allowed_ips.join(", ");
                let keepalive = format!("{}s", p.config.persistent_keepalive);
                let backend = p.backend.to_string();

                Row::new(vec![
                    Cell::from(name),
                    Cell::from(short_pk),
                    Cell::from(endpoint),
                    Cell::from(allowed),
                    Cell::from(keepalive),
                    Cell::from(backend),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(15),
                Constraint::Percentage(15),
                Constraint::Percentage(20),
                Constraint::Percentage(20),
                Constraint::Percentage(10),
                Constraint::Percentage(20),
            ],
        )
        .header(header)
        .block(theme.panel_block("Peers"))
        .row_highlight_style(theme.highlight_style());

        frame.render_stateful_widget(table, area, &mut self.table_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_wg_core::config::{InterfaceConfig, PeerConfig, WgConfig};
    use ferro_wg_core::key::PrivateKey;

    fn test_state() -> AppState {
        AppState::new(WgConfig {
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
            peers: vec![PeerConfig {
                name: "peer-a".into(),
                public_key: PrivateKey::generate().public_key(),
                preshared_key: None,
                endpoint: Some("1.2.3.4:51820".into()),
                allowed_ips: vec!["10.0.0.0/24".into()],
                persistent_keepalive: 25,
            }],
        })
    }

    #[test]
    fn handle_key_returns_actions() {
        let mut comp = PeersComponent::new();
        let state = test_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('j')), &state),
            Some(Action::NextRow)
        );
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Up), &state),
            Some(Action::PrevRow)
        );
    }

    #[test]
    fn update_prev_row_clamps_at_zero() {
        let mut comp = PeersComponent::new();
        let state = test_state();
        comp.update(&Action::PrevRow, &state);
        assert_eq!(comp.table_state.selected(), Some(0));
    }
}
