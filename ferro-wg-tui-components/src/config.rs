//! Config tab: interface and peer configuration display.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use ferro_wg_tui_core::config_edit::{fields_for_section, ConfigSection, EditableField};
use ferro_wg_tui_core::{Action, AppState, Component};

/// Interactive display and editor of the active `WireGuard` interface and peer configuration.
///
/// Shows public key, listen port, addresses, DNS servers, MTU, and peer details.
/// Supports navigation with j/k, editing with e, adding peers with +, deleting with x.
pub struct ConfigComponent {
    /// Which section of the form is focused (component-local state).
    focused_section: ConfigSection,
    /// Which field within the section is focused.
    focused_field_idx: usize,
}

impl ConfigComponent {
    /// Create a new config component.
    #[must_use]
    pub fn new() -> Self {
        Self {
            focused_section: ConfigSection::Interface,
            focused_field_idx: 0,
        }
    }

    /// Move focus to the next field, wrapping within the current section.
    fn focus_next(&mut self, _config: &ferro_wg_core::config::WgConfig) {
        let fields = fields_for_section(self.focused_section, false); // is_new_peer=false for navigation
        if fields.is_empty() {
            return;
        }
        self.focused_field_idx = (self.focused_field_idx + 1) % fields.len();
    }

    /// Move focus to the previous field, wrapping within the current section.
    fn focus_prev(&mut self, _config: &ferro_wg_core::config::WgConfig) {
        let fields = fields_for_section(self.focused_section, false);
        if fields.is_empty() {
            return;
        }
        self.focused_field_idx = if self.focused_field_idx == 0 {
            fields.len() - 1
        } else {
            self.focused_field_idx - 1
        };
    }

    /// Check if the currently focused field is editable.
    fn can_edit_current_field(&self, config: &ferro_wg_core::config::WgConfig) -> bool {
        let fields = fields_for_section(self.focused_section, false);
        if self.focused_field_idx >= fields.len() {
            return false;
        }
        let field = fields[self.focused_field_idx];
        match field {
            EditableField::PeerPublicKey => {
                // PublicKey is read-only for existing peers
                if let ConfigSection::Peer(peer_idx) = self.focused_section {
                    peer_idx >= config.peers.len() // true for new peers
                } else {
                    false
                }
            }
            // All other fields are editable
            _ => true,
        }
    }
}

impl Default for ConfigComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for ConfigComponent {
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action> {
        let Some(conn) = state.active_connection() else {
            return None;
        };

        // Only handle keys when Config tab is active and not in edit mode
        if state.active_tab != ferro_wg_tui_core::app::Tab::Config
            || state.input_mode != ferro_wg_tui_core::app::InputMode::Normal
        {
            return None;
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.focus_next(&conn.config);
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.focus_prev(&conn.config);
                None
            }
            KeyCode::Char('e') => {
                // Check if the focused field is editable
                if self.can_edit_current_field(&conn.config) {
                    Some(Action::EnterConfigEdit)
                } else {
                    None
                }
            }
            KeyCode::Char('+') => Some(Action::AddConfigPeer),
            KeyCode::Char('x') => {
                if let ConfigSection::Peer(peer_idx) = self.focused_section {
                    if peer_idx < conn.config.peers.len() {
                        Some(Action::RequestConfirm {
                            message: format!("Delete peer {}?", peer_idx),
                            action: ferro_wg_tui_core::action::ConfirmAction::DeletePeer(peer_idx),
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            KeyCode::Char('p') => Some(Action::PreviewConfig),
            KeyCode::Esc => Some(Action::DiscardConfigEdits),
            _ => None,
        }
    }

    fn update(&mut self, action: &Action, state: &AppState) {
        match action {
            Action::ConfigFocusInterface => {
                self.focused_section = ConfigSection::Interface;
                self.focused_field_idx = 0;
            }
            Action::ConfigFocusPeer(peer_idx) => {
                self.focused_section = ConfigSection::Peer(*peer_idx);
                self.focused_field_idx = 0;
            }
            Action::ConfigFocusNext => {
                if let Some(conn) = state.active_connection() {
                    self.focus_next(&conn.config);
                }
            }
            Action::ConfigFocusPrev => {
                if let Some(conn) = state.active_connection() {
                    self.focus_prev(&conn.config);
                }
            }
            Action::AddConfigPeer => {
                // Focus will be moved by state.rs when entering edit mode
            }
            Action::DeleteConfigPeer(peer_idx) => {
                // Adjust focus if the deleted peer was focused
                if let ConfigSection::Peer(focused_idx) = self.focused_section {
                    if focused_idx == *peer_idx {
                        // Move focus to interface if this was the last peer
                        if let Some(conn) = state.active_connection() {
                            if conn.config.peers.is_empty() {
                                self.focused_section = ConfigSection::Interface;
                                self.focused_field_idx = 0;
                            }
                        }
                    } else if focused_idx > *peer_idx {
                        // Shift focus up if we deleted a peer before the focused one
                        self.focused_section = ConfigSection::Peer(focused_idx.saturating_sub(1));
                    }
                }
            }
            _ => {}
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        let theme = &state.theme;

        let Some(conn) = state.active_connection() else {
            let para = Paragraph::new("No connections configured.")
                .block(theme.panel_block("Config"))
                .style(Style::default().fg(theme.muted));
            frame.render_widget(para, area);
            return;
        };

        let mut lines = Vec::new();

        // Render interface section
        lines.push(Line::from(Span::styled(
            "[Interface]",
            Style::default().fg(theme.accent),
        )));
        self.render_section_fields(&mut lines, &conn.config, ConfigSection::Interface, state);

        // Render peer sections
        for (peer_idx, _peer) in conn.config.peers.iter().enumerate() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("[Peer {}]", peer_idx),
                Style::default().fg(theme.accent),
            )));
            self.render_section_fields(
                &mut lines,
                &conn.config,
                ConfigSection::Peer(peer_idx),
                state,
            );
        }

        let paragraph = Paragraph::new(lines).block(theme.panel_block("Config"));
        frame.render_widget(paragraph, area);
    }
}

impl ConfigComponent {
    /// Render the fields for a specific section.
    fn render_section_fields(
        &self,
        lines: &mut Vec<Line>,
        config: &ferro_wg_core::config::WgConfig,
        section: ConfigSection,
        state: &AppState,
    ) {
        let theme = &state.theme;
        let fields = fields_for_section(section, false);
        let is_editing_this_section = state
            .config_edit
            .as_ref()
            .map(|edit| edit.focused_section == section)
            .unwrap_or(false);

        for (field_idx, &field) in fields.iter().enumerate() {
            let is_focused = section == self.focused_section && field_idx == self.focused_field_idx;
            let is_editing = is_editing_this_section
                && is_focused
                && state.config_edit.as_ref().unwrap().edit_buffer.is_some();

            let (label, value, is_read_only) = self.get_field_display(field, section, config);

            let mut line_spans = Vec::new();

            // Focus indicator
            if is_focused {
                if is_editing {
                    line_spans.push(Span::styled(
                        "[editing] ",
                        Style::default().fg(theme.accent),
                    ));
                } else {
                    line_spans.push(Span::styled(
                        "[focused] ",
                        Style::default().fg(theme.warning),
                    ));
                }
            } else {
                line_spans.push(Span::raw("          "));
            }

            // Label
            line_spans.push(Span::styled(
                format!("{}: ", label),
                Style::default().fg(theme.accent),
            ));

            // Value or buffer
            if is_editing {
                let buffer = state
                    .config_edit
                    .as_ref()
                    .unwrap()
                    .edit_buffer
                    .as_ref()
                    .unwrap();
                line_spans.push(Span::raw(buffer.clone()));
                line_spans.push(Span::styled("█", Style::default().fg(theme.muted)));
            } else {
                let value_style = if is_read_only {
                    Style::default().fg(theme.muted)
                } else {
                    Style::default()
                };
                line_spans.push(Span::styled(value, value_style));
                if is_read_only {
                    line_spans.push(Span::styled(
                        " (read-only)",
                        Style::default().fg(theme.muted),
                    ));
                }
            }

            lines.push(Line::from(line_spans));

            // Show field error if this is the focused field being edited
            if is_editing {
                if let Some(error) = &state.config_edit.as_ref().unwrap().field_error {
                    lines.push(Line::from(Span::styled(
                        format!("  Error: {}", error),
                        Style::default().fg(theme.error),
                    )));
                }
            }
        }
    }

    /// Get the display label and value for a field.
    fn get_field_display(
        &self,
        field: EditableField,
        section: ConfigSection,
        config: &ferro_wg_core::config::WgConfig,
    ) -> (&'static str, String, bool) {
        let iface = &config.interface;
        match field {
            EditableField::ListenPort => ("Listen Port", iface.listen_port.to_string(), false),
            EditableField::Addresses => ("Addresses", iface.addresses.join(", "), false),
            EditableField::Dns => (
                "DNS",
                iface
                    .dns
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
                false,
            ),
            EditableField::DnsSearch => ("DNS Search", iface.dns_search.join(", "), false),
            EditableField::Mtu => ("MTU", iface.mtu.to_string(), false),
            EditableField::Fwmark => ("Fwmark", iface.fwmark.to_string(), false),
            EditableField::PreUp => ("PreUp", iface.pre_up.join(", "), false),
            EditableField::PostUp => ("PostUp", iface.post_up.join(", "), false),
            EditableField::PreDown => ("PreDown", iface.pre_down.join(", "), false),
            EditableField::PostDown => ("PostDown", iface.post_down.join(", "), false),
            EditableField::PeerName => {
                if let ConfigSection::Peer(peer_idx) = section {
                    if let Some(peer) = config.peers.get(peer_idx) {
                        ("Name", peer.name.clone(), false)
                    } else {
                        ("Name", String::new(), false)
                    }
                } else {
                    ("Name", String::new(), false)
                }
            }
            EditableField::PeerPublicKey => {
                if let ConfigSection::Peer(peer_idx) = section {
                    if let Some(peer) = config.peers.get(peer_idx) {
                        ("Public Key", peer.public_key.to_base64(), true)
                    } else {
                        ("Public Key", String::new(), false)
                    }
                } else {
                    ("Public Key", String::new(), false)
                }
            }
            EditableField::PeerEndpoint => {
                if let ConfigSection::Peer(peer_idx) = section {
                    if let Some(peer) = config.peers.get(peer_idx) {
                        ("Endpoint", peer.endpoint.clone().unwrap_or_default(), false)
                    } else {
                        ("Endpoint", String::new(), false)
                    }
                } else {
                    ("Endpoint", String::new(), false)
                }
            }
            EditableField::PeerAllowedIps => {
                if let ConfigSection::Peer(peer_idx) = section {
                    if let Some(peer) = config.peers.get(peer_idx) {
                        ("Allowed IPs", peer.allowed_ips.join(", "), false)
                    } else {
                        ("Allowed IPs", String::new(), false)
                    }
                } else {
                    ("Allowed IPs", String::new(), false)
                }
            }
            EditableField::PeerPersistentKeepalive => {
                if let ConfigSection::Peer(peer_idx) = section {
                    if let Some(peer) = config.peers.get(peer_idx) {
                        (
                            "Persistent Keepalive",
                            if peer.persistent_keepalive == 0 {
                                String::new()
                            } else {
                                peer.persistent_keepalive.to_string()
                            },
                            false,
                        )
                    } else {
                        ("Persistent Keepalive", String::new(), false)
                    }
                } else {
                    ("Persistent Keepalive", String::new(), false)
                }
            }
        }
    }
}
