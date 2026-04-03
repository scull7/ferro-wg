//! Connection bar: thin strip showing all connections with status indicators.

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use ferro_wg_tui_core::{Action, AppState, Component, ConnectionState};

/// Thin horizontal bar rendered between the tab bar and content area
/// when more than one connection is configured.
///
/// Renders: ` Connections: ◀  [1] mia ●  [2] tus1 ○  [3] ord01 ?  ▶`
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

        let mut spans: Vec<Span<'static>> = vec![
            Span::raw(" Connections: "),
            Span::styled("◀  ", Style::default().fg(theme.muted)),
        ];

        for (i, conn) in state.connections.iter().enumerate() {
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

        spans.push(Span::styled("▶", Style::default().fg(theme.muted)));

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

    #[test]
    fn connection_bar_hidden_single() {
        let mut connections = BTreeMap::new();
        connections.insert("mia".to_string(), make_wg_config());
        let state = AppState::new(AppConfig { connections });

        // With a single connection the bar renders nothing.
        let content = render_bar(&state, 80);
        // All cells should be the default space.
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
        // All connections start with status: None.
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
}
