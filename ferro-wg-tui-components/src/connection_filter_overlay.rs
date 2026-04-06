//! Connection filter overlay component.
//!
//! [`ConnectionFilterOverlayComponent`] renders a centered modal overlay
//! for filtering connection visibility. Displays a searchable list of connections
//! with checkboxes for toggling visibility.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState};
use ratatui::Frame;

use ferro_wg_tui_core::{Action, AppState, Component};

/// A modal overlay for filtering connection visibility.
///
/// Activated when `state.show_connection_filter` is `true`. Renders a centered
/// overlay with a search input and a scrollable list of connections with checkboxes.
pub struct ConnectionFilterOverlayComponent {
    /// Selection state for the connection list.
    table_state: TableState,
}

impl ConnectionFilterOverlayComponent {
    /// Create a new connection filter overlay component.
    #[must_use]
    pub fn new() -> Self {
        Self {
            table_state: TableState::default().with_selected(Some(0)),
        }
    }

    /// Get connections filtered by the current search query.
    fn filtered_connections(state: &AppState) -> Vec<&ferro_wg_tui_core::ConnectionView> {
        let query = state.connection_filter_search.to_lowercase();
        state
            .connections
            .iter()
            .filter(|conn| query.is_empty() || conn.name.to_lowercase().contains(&query))
            .collect()
    }
}

impl Default for ConnectionFilterOverlayComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for ConnectionFilterOverlayComponent {
    /// Route keys when the connection filter overlay is active.
    ///
    /// Returns `None` when the overlay is not shown (acts as a no-op).
    /// `Esc` → [`Action::HideConnectionFilter`]; `Char(c)` appends to search;
    /// `Backspace` removes last char; `Up`/`Down` navigate; `Enter` toggles visibility.
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action> {
        if !state.show_connection_filter {
            return None;
        }
        match key.code {
            KeyCode::Esc => Some(Action::HideConnectionFilter),
            KeyCode::Char(c) => {
                let mut search = state.connection_filter_search.clone();
                search.push(c);
                Some(Action::SetConnectionFilterSearch(search))
            }
            KeyCode::Backspace => {
                let mut search = state.connection_filter_search.clone();
                search.pop();
                Some(Action::SetConnectionFilterSearch(search))
            }
            KeyCode::Up => Some(Action::PrevRow),
            KeyCode::Down => Some(Action::NextRow),
            KeyCode::Enter => {
                let selected = self.table_state.selected()?;
                let filtered = Self::filtered_connections(state);
                filtered
                    .get(selected)
                    .map(|conn| Action::ToggleConnectionVisibility(conn.name.clone()))
            }
            _ => None,
        }
    }

    fn update(&mut self, action: &Action, state: &AppState) {
        if !state.show_connection_filter {
            return;
        }
        match action {
            Action::NextRow => {
                let filtered = Self::filtered_connections(state);
                let current = self.table_state.selected().unwrap_or(0);
                let next = (current + 1).min(filtered.len().saturating_sub(1));
                self.table_state.select(Some(next));
            }
            Action::PrevRow => {
                let current = self.table_state.selected().unwrap_or(0);
                let prev = current.saturating_sub(1);
                self.table_state.select(Some(prev));
            }
            Action::SetConnectionFilterSearch(_) => {
                // Reset selection when search changes
                self.table_state.select(Some(0));
            }
            Action::ShowConnectionFilter => {
                self.table_state.select(Some(0));
            }
            _ => {}
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        if !state.show_connection_filter {
            return;
        }
        let overlay_area = crate::util::centered_rect(80, 20, area);
        frame.render_widget(Clear, overlay_area);
        let block = state.theme.overlay_block("Connection Filter");
        let inner = block.inner(overlay_area);
        frame.render_widget(block, overlay_area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(inner);

        // Search input
        let search_para = Paragraph::new(format!("Search: {}", state.connection_filter_search))
            .block(Block::default().borders(Borders::ALL).title("Search"));
        frame.render_widget(search_para, chunks[0]);

        // Connection list
        let filtered = Self::filtered_connections(state);
        if filtered.is_empty() {
            let para = Paragraph::new("No visible connections.")
                .block(Block::default().borders(Borders::ALL).title("Connections"));
            frame.render_widget(para, chunks[1]);
        } else {
            let rows: Vec<Row> = filtered
                .into_iter()
                .map(|conn| {
                    let checked = state.visible_connections.contains(&conn.name);
                    let checkbox = if checked { "[x]" } else { "[ ]" };
                    Row::new(vec![Cell::from(format!("{} {}", checkbox, conn.name))])
                })
                .collect();
            let table = Table::new(rows, [Constraint::Percentage(100)])
                .block(Block::default().borders(Borders::ALL).title("Connections"))
                .row_highlight_style(state.theme.highlight_style());
            frame.render_stateful_widget(table, chunks[1], &mut self.table_state);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use ferro_wg_core::config::{AppConfig, InterfaceConfig, PeerConfig, WgConfig};
    use ferro_wg_core::key::PrivateKey;
    use ferro_wg_tui_core::{AppState, Component};

    fn render_overlay(state: &AppState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let mut comp = super::ConnectionFilterOverlayComponent::new();
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

    fn make_app_config_with_connections(connections: &[(&str, Vec<PeerConfig>)]) -> AppConfig {
        let mut conns = std::collections::BTreeMap::new();
        for (name, peers) in connections {
            let config = WgConfig {
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
                peers: peers.clone(),
            };
            conns.insert((*name).to_string(), config);
        }
        AppConfig {
            connections: conns,
            ..AppConfig::default()
        }
    }

    fn make_peer(name: &str) -> PeerConfig {
        PeerConfig {
            name: name.into(),
            public_key: PrivateKey::generate().public_key(),
            preshared_key: None,
            endpoint: Some("198.51.100.1:51820".to_string()),
            allowed_ips: vec!["10.100.0.0/16".into()],
            persistent_keepalive: 25,
        }
    }

    fn test_state() -> AppState {
        let mut state = AppState::new(make_app_config_with_connections(&[
            ("test", vec![make_peer("peer-a")]),
            ("other", vec![make_peer("peer-b")]),
        ]));
        state.show_connection_filter = true;
        state
    }

    #[test]
    fn render_overlay_snapshot() {
        let state = test_state();
        let content = render_overlay(&state, 80, 20);
        assert!(
            content.contains("test") && content.contains("other"),
            "expected connections in overlay: {content:?}"
        );
    }

    #[test]
    fn render_overlay_empty_connections() {
        let mut state = AppState::new(make_app_config_with_connections(&[]));
        state.show_connection_filter = true;
        let content = render_overlay(&state, 80, 20);
        assert!(
            content.contains("No visible connections"),
            "expected 'No visible connections' in: {content:?}"
        );
    }
}
