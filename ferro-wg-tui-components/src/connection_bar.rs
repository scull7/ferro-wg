//! Connection bar: thin strip showing all connections with status indicators.

use std::borrow::Cow;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use ferro_wg_tui_core::{Action, AppState, Component, ConnectionState};

/// Maximum number of Unicode scalar values shown from a connection name
/// before the display is truncated with a `…` suffix.
const MAX_DISPLAY_NAME_LEN: usize = 20;

/// Return a display-ready version of `name`, truncating with `…` when it
/// exceeds [`MAX_DISPLAY_NAME_LEN`] characters.
///
/// Truncation is char-boundary-safe (counts Unicode scalar values, not bytes).
/// Names within the limit are returned as a borrow; only truncated names
/// allocate.
fn truncate_name(name: &str) -> Cow<'_, str> {
    if name.chars().count() > MAX_DISPLAY_NAME_LEN {
        let head: String = name.chars().take(MAX_DISPLAY_NAME_LEN).collect();
        Cow::Owned(format!("{head}…"))
    } else {
        Cow::Borrowed(name)
    }
}

/// Thin horizontal bar rendered between the tab bar and content area
/// when more than one connection is configured.
///
/// Unselected connections are rendered compactly as `[N]●` to conserve
/// horizontal space. The selected connection expands to show its full
/// name in bold accent: `[N] name ●`.
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
    fn handle_key(&mut self, key: KeyEvent, _state: &AppState) -> Option<Action> {
        match key.code {
            KeyCode::Char('[') => Some(Action::SelectPrevConnection),
            KeyCode::Char(']') => Some(Action::SelectNextConnection),
            _ => None,
        }
    }

    fn update(&mut self, _action: &Action, _state: &AppState) {}

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        if state.connections.len() <= 1 {
            return;
        }

        let theme = &state.theme;

        let mut spans: Vec<Span<'_>> = vec![Span::raw(" Connections: ")];

        for (i, conn) in state.connections.iter().enumerate() {
            let (indicator, ind_style): (&'static str, Style) = match &conn.status {
                None => ("?", Style::default().fg(theme.warning)),
                Some(s) if s.state == ConnectionState::Connected => {
                    ("●", Style::default().fg(theme.success))
                }
                Some(_) => ("○", Style::default().fg(theme.muted)),
            };

            if i == state.selected_connection {
                let label_style = Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD);
                spans.push(Span::styled(format!("[{}] ", i + 1), label_style));
                spans.push(Span::styled(truncate_name(conn.name.as_str()), label_style));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(indicator, ind_style));
                spans.push(Span::raw("  "));
            } else {
                spans.push(Span::raw(format!("[{}]", i + 1)));
                spans.push(Span::styled(indicator, ind_style));
                spans.push(Span::raw("  "));
            }
        }

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

    fn many_connection_state(n: usize) -> AppState {
        let mut connections = BTreeMap::new();
        for i in 0..n {
            connections.insert(format!("c{i:02}"), make_wg_config());
        }
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
        let content = render_bar(&state, 80);
        assert!(content.trim().is_empty());
    }

    #[test]
    fn connection_bar_renders_selected_name() {
        // BTreeMap sorts: mia(0), ord01(1), tus1(2); selected_connection = 0 → "mia"
        let state = three_connection_state();
        let content = render_bar(&state, 120);
        assert!(
            content.contains("mia"),
            "selected name must appear: {content:?}"
        );
    }

    #[test]
    fn connection_bar_unselected_names_hidden() {
        // selected = 0 ("mia"); ord01 and tus1 are compact — no name shown.
        let state = three_connection_state();
        let content = render_bar(&state, 120);
        assert!(
            !content.contains("ord01"),
            "unselected 'ord01' should not appear: {content:?}"
        );
        assert!(
            !content.contains("tus1"),
            "unselected 'tus1' should not appear: {content:?}"
        );
    }

    #[test]
    fn connection_bar_selection_change_shows_new_name() {
        // Move selection to index 1 → "ord01" expands, "mia" collapses.
        let mut state = three_connection_state();
        state.selected_connection = 1;
        let content = render_bar(&state, 120);
        assert!(
            content.contains("ord01"),
            "newly selected 'ord01' must appear: {content:?}"
        );
        assert!(
            !content.contains("mia"),
            "unselected 'mia' should not appear: {content:?}"
        );
    }

    #[test]
    fn connection_bar_not_polled_shows_question_mark() {
        let state = three_connection_state();
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

    #[test]
    fn connection_bar_no_panic_extremely_narrow() {
        let state = three_connection_state();
        render_bar(&state, 1);
    }

    #[test]
    fn connection_bar_many_connections_no_overflow_panic() {
        // 20 connections with selected at last index — must not panic.
        let mut state = many_connection_state(20);
        state.selected_connection = 19;
        render_bar(&state, 80);
    }

    // ── truncate_name unit tests ─────────────────────────────────────────────

    #[test]
    fn truncate_name_short_name_unchanged() {
        assert_eq!(truncate_name("mia"), Cow::Borrowed("mia"));
    }

    #[test]
    fn truncate_name_at_limit_unchanged() {
        let name = "a".repeat(MAX_DISPLAY_NAME_LEN);
        assert_eq!(truncate_name(&name), Cow::Borrowed(name.as_str()));
    }

    #[test]
    fn truncate_name_one_over_limit_appends_ellipsis() {
        let name = "a".repeat(MAX_DISPLAY_NAME_LEN + 1);
        let result = truncate_name(&name);
        assert!(
            result.ends_with('…'),
            "expected ellipsis suffix: {result:?}"
        );
        // Visible chars = MAX_DISPLAY_NAME_LEN a's + 1 ellipsis.
        assert_eq!(result.chars().count(), MAX_DISPLAY_NAME_LEN + 1);
    }

    #[test]
    fn truncate_name_long_name_prefix_preserved() {
        let name = "verylongconnectionname-datacenter-west";
        let result = truncate_name(name);
        let expected_prefix: String = name.chars().take(MAX_DISPLAY_NAME_LEN).collect();
        assert!(
            result.starts_with(expected_prefix.as_str()),
            "expected first {MAX_DISPLAY_NAME_LEN} chars preserved: {result:?}"
        );
        assert!(result.ends_with('…'));
    }

    // ── overflow / edge-case rendering tests ─────────────────────────────────

    #[test]
    fn long_selected_name_renders_with_ellipsis() {
        // "mlong-..." sorts after "anchor" lexicographically, so it lands at index 1.
        let long_name = "m".repeat(MAX_DISPLAY_NAME_LEN + 10);
        let mut connections = BTreeMap::new();
        connections.insert("anchor".to_string(), make_wg_config()); // index 0
        connections.insert(long_name.clone(), make_wg_config()); // index 1
        let mut state = AppState::new(AppConfig { connections });
        state.selected_connection = 1; // select the long-named connection
        let content = render_bar(&state, 120);
        assert!(
            content.contains('…'),
            "expected '…' for long selected name: {content:?}"
        );
        // The full long name must NOT appear verbatim.
        assert!(
            !content.contains(long_name.as_str()),
            "full long name must be truncated: {content:?}"
        );
    }

    #[test]
    fn short_selected_name_renders_without_ellipsis() {
        let state = three_connection_state(); // selected = 0 → "mia" (3 chars)
        let content = render_bar(&state, 120);
        assert!(
            !content.contains('…'),
            "short name must not be truncated: {content:?}"
        );
    }

    #[test]
    fn name_exactly_at_limit_renders_without_ellipsis() {
        let exact_name = "b".repeat(MAX_DISPLAY_NAME_LEN);
        let mut connections = BTreeMap::new();
        connections.insert("anchor".to_string(), make_wg_config());
        connections.insert(exact_name.clone(), make_wg_config());
        let mut state = AppState::new(AppConfig { connections });
        // BTreeMap sorts: "anchor" < "bbb...", so exact_name is at index 1.
        state.selected_connection = 1;
        let content = render_bar(&state, 120);
        assert!(
            !content.contains('…'),
            "name at limit must not be truncated: {content:?}"
        );
        assert!(content.contains(exact_name.as_str()));
    }

    #[test]
    fn zero_width_terminal_no_panic() {
        let state = three_connection_state();
        // ratatui clamps to minimum; render must not panic.
        render_bar(&state, 0);
    }

    #[test]
    fn single_char_terminal_no_panic() {
        let state = three_connection_state();
        render_bar(&state, 1);
    }

    #[test]
    fn fifty_connections_no_panic() {
        let mut state = many_connection_state(50);
        state.selected_connection = 49;
        render_bar(&state, 80);
    }

    #[test]
    fn fifty_connections_selected_name_visible() {
        // Even with 50 connections, the selected entry must appear somewhere.
        // Select index 0 ("c00") so it renders first, before any clipping.
        let state = many_connection_state(50); // selected_connection = 0 → "c00"
        let content = render_bar(&state, 120);
        assert!(
            content.contains("c00"),
            "selected 'c00' must appear: {content:?}"
        );
    }

    // ── handle_key tests ─────────────────────────────────────────────────────

    #[test]
    fn handle_key_bracket_right_emits_select_next() {
        let mut comp = ConnectionBarComponent::new();
        let state = three_connection_state();
        let action = comp.handle_key(KeyEvent::from(KeyCode::Char(']')), &state);
        assert_eq!(action, Some(Action::SelectNextConnection));
    }

    #[test]
    fn handle_key_bracket_left_emits_select_prev() {
        let mut comp = ConnectionBarComponent::new();
        let state = three_connection_state();
        let action = comp.handle_key(KeyEvent::from(KeyCode::Char('[')), &state);
        assert_eq!(action, Some(Action::SelectPrevConnection));
    }

    #[test]
    fn handle_key_other_keys_return_none() {
        let mut comp = ConnectionBarComponent::new();
        let state = three_connection_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Enter), &state),
            None
        );
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('q')), &state),
            None
        );
    }
}
