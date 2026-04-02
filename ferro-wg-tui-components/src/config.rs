//! Config tab: interface and peer configuration display.

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use ferro_wg_tui_core::{Action, AppState, Component};

/// Read-only display of the active `WireGuard` interface configuration.
///
/// Shows public key, listen port, addresses, DNS servers, MTU, and
/// the number of configured peers.
pub struct ConfigComponent;

impl ConfigComponent {
    /// Create a new config component.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ConfigComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for ConfigComponent {
    fn handle_key(&mut self, _key: KeyEvent, _state: &AppState) -> Option<Action> {
        // Config view has no interactive elements yet.
        None
    }

    fn update(&mut self, _action: &Action, _state: &AppState) {
        // No local state to update.
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        let theme = &state.theme;
        let iface = &state.wg_config.interface;
        let label_style = Style::default().fg(theme.accent);

        let public_key = iface.private_key.public_key().to_base64();
        let addrs = iface.addresses.join(", ");
        let dns: Vec<String> = iface.dns.iter().map(ToString::to_string).collect();

        let text = vec![
            Line::from(vec![
                Span::styled("Public Key: ", label_style),
                Span::raw(public_key),
            ]),
            Line::from(vec![
                Span::styled("Listen Port: ", label_style),
                Span::raw(iface.listen_port.to_string()),
            ]),
            Line::from(vec![
                Span::styled("Address: ", label_style),
                Span::raw(if addrs.is_empty() {
                    "(none)".to_owned()
                } else {
                    addrs
                }),
            ]),
            Line::from(vec![
                Span::styled("DNS: ", label_style),
                Span::raw(if dns.is_empty() {
                    "(none)".to_owned()
                } else {
                    dns.join(", ")
                }),
            ]),
            Line::from(vec![
                Span::styled("MTU: ", label_style),
                Span::raw(iface.mtu.to_string()),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                format!("{} peer(s) configured", state.wg_config.peers.len()),
                Style::default().fg(theme.warning),
            )),
        ];

        let paragraph = Paragraph::new(text).block(theme.panel_block("Config"));
        frame.render_widget(paragraph, area);
    }
}
