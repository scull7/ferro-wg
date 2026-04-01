//! TUI application state machine.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::TableState;

use ferro_wg_core::config::{PeerConfig, WgConfig};
use ferro_wg_core::error::BackendKind;
use ferro_wg_core::stats::TunnelStats;

/// Active tab in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// Active tunnel overview.
    Status,
    /// All configured peers.
    Peers,
    /// Backend performance comparison.
    Compare,
    /// Interface and peer configuration.
    Config,
    /// Live log viewer.
    Logs,
}

impl Tab {
    /// All tabs in display order.
    pub const ALL: [Self; 5] = [
        Self::Status,
        Self::Peers,
        Self::Compare,
        Self::Config,
        Self::Logs,
    ];

    /// Tab display title.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Status => "Status",
            Self::Peers => "Peers",
            Self::Compare => "Compare",
            Self::Config => "Config",
            Self::Logs => "Logs",
        }
    }

    /// Zero-based tab index.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Self::Status => 0,
            Self::Peers => 1,
            Self::Compare => 2,
            Self::Config => 3,
            Self::Logs => 4,
        }
    }
}

/// Input mode — normal navigation or search filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// Arrow keys navigate, hotkeys active.
    Normal,
    /// Typing into the search bar.
    Search,
}

/// Per-peer runtime state shown in the TUI.
#[derive(Debug, Clone)]
pub struct PeerState {
    /// The peer's static config.
    pub config: PeerConfig,
    /// Whether the tunnel is connected.
    pub connected: bool,
    /// Current tunnel statistics (if connected).
    pub stats: TunnelStats,
    /// Which backend is active for this peer.
    pub backend: BackendKind,
}

/// The full TUI state.
pub struct App {
    /// Whether the app is still running.
    pub running: bool,
    /// Currently selected tab.
    pub active_tab: Tab,
    /// Input mode (normal vs search).
    pub input_mode: InputMode,
    /// Search query string.
    pub search_query: String,
    /// Table selection state (row highlight).
    pub table_state: TableState,

    /// The loaded `WireGuard` configuration.
    pub wg_config: WgConfig,
    /// Per-peer runtime state.
    pub peers: Vec<PeerState>,

    /// Log lines for the Logs tab.
    pub log_lines: Vec<String>,
}

impl App {
    /// Create a new app from a loaded config.
    #[must_use]
    pub fn new(wg_config: WgConfig) -> Self {
        let peers = wg_config
            .peers
            .iter()
            .map(|p| PeerState {
                config: p.clone(),
                connected: false,
                stats: TunnelStats::default(),
                backend: BackendKind::Boringtun,
            })
            .collect();

        Self {
            running: true,
            active_tab: Tab::Status,
            input_mode: InputMode::Normal,
            search_query: String::new(),
            table_state: TableState::default().with_selected(Some(0)),
            wg_config,
            peers,
            log_lines: Vec::new(),
        }
    }

    /// Handle a key press event.
    pub fn handle_key(&mut self, key: KeyEvent) {
        match self.input_mode {
            InputMode::Search => self.handle_search_key(key),
            InputMode::Normal => self.handle_normal_key(key),
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.running = false,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.running = false;
            }
            // Tab navigation.
            KeyCode::Tab | KeyCode::Right => self.next_tab(),
            KeyCode::BackTab | KeyCode::Left => self.prev_tab(),
            // Numeric tab selection.
            KeyCode::Char('1') => self.set_tab(Tab::Status),
            KeyCode::Char('2') => self.set_tab(Tab::Peers),
            KeyCode::Char('3') => self.set_tab(Tab::Compare),
            KeyCode::Char('4') => self.set_tab(Tab::Config),
            KeyCode::Char('5') => self.set_tab(Tab::Logs),
            // Row navigation.
            KeyCode::Down | KeyCode::Char('j') => self.next_row(),
            KeyCode::Up | KeyCode::Char('k') => self.prev_row(),
            // Search.
            KeyCode::Char('/') => {
                self.input_mode = InputMode::Search;
                self.search_query.clear();
            }
            _ => {}
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.search_query.clear();
            }
            KeyCode::Enter => self.input_mode = InputMode::Normal,
            KeyCode::Backspace => {
                self.search_query.pop();
            }
            KeyCode::Char(c) => self.search_query.push(c),
            _ => {}
        }
    }

    fn set_tab(&mut self, tab: Tab) {
        self.active_tab = tab;
        self.table_state.select(Some(0));
    }

    fn next_tab(&mut self) {
        let idx = (self.active_tab.index() + 1) % Tab::ALL.len();
        self.active_tab = Tab::ALL[idx];
        self.table_state.select(Some(0));
    }

    fn prev_tab(&mut self) {
        let idx = (self.active_tab.index() + Tab::ALL.len() - 1) % Tab::ALL.len();
        self.active_tab = Tab::ALL[idx];
        self.table_state.select(Some(0));
    }

    fn next_row(&mut self) {
        let max = self.current_row_count().saturating_sub(1);
        let current = self.table_state.selected().unwrap_or(0);
        self.table_state
            .select(Some(current.saturating_add(1).min(max)));
    }

    fn prev_row(&mut self) {
        let current = self.table_state.selected().unwrap_or(0);
        self.table_state.select(Some(current.saturating_sub(1)));
    }

    /// Number of rows in the currently active tab.
    fn current_row_count(&self) -> usize {
        match self.active_tab {
            Tab::Status | Tab::Peers | Tab::Compare | Tab::Config => self.peers.len(),
            Tab::Logs => self.log_lines.len(),
        }
    }

    /// Filtered peers matching the current search query.
    pub fn filtered_peers(&self) -> impl Iterator<Item = &PeerState> {
        let query = self.search_query.to_lowercase();
        self.peers.iter().filter(move |p| {
            query.is_empty()
                || p.config.name.to_lowercase().contains(&query)
                || p.config
                    .endpoint
                    .is_some_and(|ep| ep.to_string().contains(&query))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_wg_core::config::{InterfaceConfig, PeerConfig, WgConfig};
    use ferro_wg_core::key::PrivateKey;

    fn test_config() -> WgConfig {
        WgConfig {
            interface: InterfaceConfig {
                private_key: PrivateKey::generate(),
                listen_port: 51820,
                addresses: vec!["10.0.0.2/24".into()],
                dns: Vec::new(),
                mtu: 1420,
                fwmark: 0,
                pre_up: Vec::new(),
                post_up: Vec::new(),
                pre_down: Vec::new(),
                post_down: Vec::new(),
            },
            peers: vec![
                PeerConfig {
                    name: "dc-sjc01".into(),
                    public_key: PrivateKey::generate().public_key(),
                    preshared_key: None,
                    endpoint: Some("198.51.100.1:51820".parse().expect("endpoint")),
                    allowed_ips: vec!["10.100.0.0/16".into()],
                    persistent_keepalive: 25,
                },
                PeerConfig {
                    name: "dc-ord01".into(),
                    public_key: PrivateKey::generate().public_key(),
                    preshared_key: None,
                    endpoint: Some("198.51.100.2:51820".parse().expect("endpoint")),
                    allowed_ips: vec!["10.200.0.0/16".into()],
                    persistent_keepalive: 25,
                },
            ],
        }
    }

    fn test_app() -> App {
        App::new(test_config())
    }

    #[test]
    fn initial_state() {
        let app = test_app();
        assert!(app.running);
        assert_eq!(app.active_tab, Tab::Status);
        assert_eq!(app.input_mode, InputMode::Normal);
        assert_eq!(app.peers.len(), 2);
    }

    #[test]
    fn tab_cycling() {
        let mut app = test_app();
        assert_eq!(app.active_tab, Tab::Status);
        app.next_tab();
        assert_eq!(app.active_tab, Tab::Peers);
        app.next_tab();
        assert_eq!(app.active_tab, Tab::Compare);
        app.next_tab();
        assert_eq!(app.active_tab, Tab::Config);
        app.next_tab();
        assert_eq!(app.active_tab, Tab::Logs);
        app.next_tab();
        assert_eq!(app.active_tab, Tab::Status); // wraps
    }

    #[test]
    fn prev_tab_wraps() {
        let mut app = test_app();
        app.prev_tab();
        assert_eq!(app.active_tab, Tab::Logs);
    }

    #[test]
    fn numeric_tab_selection() {
        let mut app = test_app();
        app.handle_key(KeyEvent::from(KeyCode::Char('3')));
        assert_eq!(app.active_tab, Tab::Compare);
        app.handle_key(KeyEvent::from(KeyCode::Char('1')));
        assert_eq!(app.active_tab, Tab::Status);
    }

    #[test]
    fn quit_on_q() {
        let mut app = test_app();
        app.handle_key(KeyEvent::from(KeyCode::Char('q')));
        assert!(!app.running);
    }

    #[test]
    fn search_mode() {
        let mut app = test_app();
        app.handle_key(KeyEvent::from(KeyCode::Char('/')));
        assert_eq!(app.input_mode, InputMode::Search);

        app.handle_key(KeyEvent::from(KeyCode::Char('s')));
        app.handle_key(KeyEvent::from(KeyCode::Char('j')));
        assert_eq!(app.search_query, "sj");

        app.handle_key(KeyEvent::from(KeyCode::Backspace));
        assert_eq!(app.search_query, "s");

        app.handle_key(KeyEvent::from(KeyCode::Esc));
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.search_query.is_empty());
    }

    #[test]
    fn filtered_peers_matches() {
        let mut app = test_app();
        app.search_query = "sjc".into();
        let matched: Vec<_> = app.filtered_peers().collect();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].config.name, "dc-sjc01");
    }

    #[test]
    fn filtered_peers_empty_query_returns_all() {
        let app = test_app();
        let matched: Vec<_> = app.filtered_peers().collect();
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn tab_titles() {
        assert_eq!(Tab::Status.title(), "Status");
        assert_eq!(Tab::Peers.title(), "Peers");
        assert_eq!(Tab::Compare.title(), "Compare");
        assert_eq!(Tab::Config.title(), "Config");
        assert_eq!(Tab::Logs.title(), "Logs");
    }

    #[test]
    fn row_navigation() {
        let mut app = test_app();
        assert_eq!(app.table_state.selected(), Some(0));
        app.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(app.table_state.selected(), Some(1));
        app.handle_key(KeyEvent::from(KeyCode::Down));
        // Should clamp at max (2 peers, max index = 1).
        assert_eq!(app.table_state.selected(), Some(1));
        app.handle_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(app.table_state.selected(), Some(0));
    }

    #[test]
    fn peers_initialized_disconnected() {
        let app = test_app();
        for peer in &app.peers {
            assert!(!peer.connected);
            assert_eq!(peer.stats.tx_bytes, 0);
        }
    }
}
