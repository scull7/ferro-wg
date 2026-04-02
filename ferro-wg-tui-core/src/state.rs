//! Centralized TUI application state.
//!
//! [`AppState`] owns all shared data (config, peers, logs, theme) and
//! processes [`Action`]s via [`dispatch()`](AppState::dispatch). Components
//! receive `&AppState` for read-only access during rendering and key
//! handling.

use std::time::{Duration, Instant};

use ferro_wg_core::config::{PeerConfig, WgConfig};
use ferro_wg_core::error::BackendKind;
use ferro_wg_core::stats::TunnelStats;

use crate::action::Action;
use crate::app::{InputMode, Tab};
use crate::theme::Theme;

/// How long feedback messages are displayed before expiring.
const FEEDBACK_DURATION: Duration = Duration::from_secs(3);

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

/// A transient feedback message shown in the status bar.
#[derive(Debug, Clone)]
pub struct Feedback {
    /// The message text.
    pub message: String,
    /// Whether this is an error (`true`) or success (`false`).
    pub is_error: bool,
    /// When this feedback expires and should be hidden.
    pub expires_at: Instant,
}

impl Feedback {
    /// Create a success feedback message.
    #[must_use]
    pub fn success(message: String) -> Self {
        Self {
            message,
            is_error: false,
            expires_at: Instant::now() + FEEDBACK_DURATION,
        }
    }

    /// Create an error feedback message.
    #[must_use]
    pub fn error(message: String) -> Self {
        Self {
            message,
            is_error: true,
            expires_at: Instant::now() + FEEDBACK_DURATION,
        }
    }

    /// Whether this feedback has expired.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

/// Centralized application state.
///
/// All shared data lives here. Components never own or duplicate this
/// data — they receive `&AppState` for read-only access.
pub struct AppState {
    /// Whether the app is still running.
    pub running: bool,
    /// Currently selected tab.
    pub active_tab: Tab,
    /// Input mode (normal vs search).
    pub input_mode: InputMode,
    /// Search query string.
    pub search_query: String,

    /// The loaded `WireGuard` configuration.
    pub wg_config: WgConfig,
    /// Per-peer runtime state.
    pub peers: Vec<PeerState>,

    /// Log lines for the Logs tab.
    pub log_lines: Vec<String>,

    /// Active color theme.
    pub theme: Theme,

    /// Whether the daemon is currently reachable.
    pub daemon_connected: bool,
    /// Transient feedback message (success or error) with expiry.
    pub feedback: Option<Feedback>,
}

impl AppState {
    /// Create a new state from a loaded config.
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
            wg_config,
            peers,
            log_lines: Vec::new(),
            theme: Theme::mocha(),
            daemon_connected: false,
            feedback: None,
        }
    }

    /// Dispatch an action, mutating shared state.
    ///
    /// This is Phase 1 of the two-phase dispatch cycle. After this
    /// returns, the caller should forward the action to all components
    /// via [`Component::update()`](crate::component::Component::update).
    pub fn dispatch(&mut self, action: &Action) {
        match action {
            Action::Quit => self.running = false,
            Action::NextTab => self.active_tab = self.active_tab.next(),
            Action::PrevTab => self.active_tab = self.active_tab.prev(),
            Action::SelectTab(tab) => self.active_tab = *tab,
            Action::EnterSearch => {
                self.input_mode = InputMode::Search;
                self.search_query.clear();
            }
            Action::ExitSearch => self.input_mode = InputMode::Normal,
            Action::ClearSearch => {
                self.input_mode = InputMode::Normal;
                self.search_query.clear();
            }
            Action::SearchInput(c) => self.search_query.push(*c),
            Action::SearchBackspace => {
                self.search_query.pop();
            }
            // -- Daemon integration --
            Action::UpdatePeers(statuses) => {
                self.daemon_connected = true;
                for status in statuses {
                    if let Some(peer) = self.peers.iter_mut().find(|p| p.config.name == status.name)
                    {
                        peer.connected = status.connected;
                        peer.stats = status.stats.clone();
                        peer.backend = status.backend;
                    }
                }
            }
            Action::DaemonConnectivityChanged(connected) => {
                self.daemon_connected = *connected;
            }
            Action::DaemonOk(msg) => {
                self.feedback = Some(Feedback::success(msg.clone()));
            }
            Action::DaemonError(msg) => {
                self.feedback = Some(Feedback::error(msg.clone()));
            }
            // Row navigation, tick, and peer commands are handled by
            // components or the event loop, not by shared state.
            Action::NextRow
            | Action::PrevRow
            | Action::Tick
            | Action::ConnectPeer(_)
            | Action::DisconnectPeer(_)
            | Action::CyclePeerBackend(_) => {}
        }
    }

    /// Clear expired feedback messages. Called on each tick.
    pub fn clear_expired_feedback(&mut self) {
        if self.feedback.as_ref().is_some_and(Feedback::is_expired) {
            self.feedback = None;
        }
    }

    /// Peers matching the current search query.
    ///
    /// Returns all peers when the query is empty. Matches against
    /// the peer name and endpoint (case-insensitive substring).
    pub fn filtered_peers(&self) -> impl Iterator<Item = &PeerState> {
        let query = self.search_query.to_lowercase();
        self.peers.iter().filter(move |p| {
            query.is_empty()
                || p.config.name.to_lowercase().contains(&query)
                || p.config
                    .endpoint
                    .as_ref()
                    .is_some_and(|ep| ep.to_lowercase().contains(&query))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_wg_core::config::InterfaceConfig;
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
                    endpoint: Some("198.51.100.1:51820".into()),
                    allowed_ips: vec!["10.100.0.0/16".into()],
                    persistent_keepalive: 25,
                },
                PeerConfig {
                    name: "dc-ord01".into(),
                    public_key: PrivateKey::generate().public_key(),
                    preshared_key: None,
                    endpoint: Some("198.51.100.2:51820".into()),
                    allowed_ips: vec!["10.200.0.0/16".into()],
                    persistent_keepalive: 25,
                },
            ],
        }
    }

    fn test_state() -> AppState {
        AppState::new(test_config())
    }

    #[test]
    fn initial_state() {
        let state = test_state();
        assert!(state.running);
        assert_eq!(state.active_tab, Tab::Status);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert_eq!(state.peers.len(), 2);
    }

    #[test]
    fn dispatch_quit() {
        let mut state = test_state();
        state.dispatch(&Action::Quit);
        assert!(!state.running);
    }

    #[test]
    fn dispatch_tab_navigation() {
        let mut state = test_state();
        state.dispatch(&Action::NextTab);
        assert_eq!(state.active_tab, Tab::Peers);
        state.dispatch(&Action::PrevTab);
        assert_eq!(state.active_tab, Tab::Status);
        state.dispatch(&Action::SelectTab(Tab::Compare));
        assert_eq!(state.active_tab, Tab::Compare);
    }

    #[test]
    fn dispatch_search_lifecycle() {
        let mut state = test_state();
        state.dispatch(&Action::EnterSearch);
        assert_eq!(state.input_mode, InputMode::Search);
        assert!(state.search_query.is_empty());

        state.dispatch(&Action::SearchInput('s'));
        state.dispatch(&Action::SearchInput('j'));
        assert_eq!(state.search_query, "sj");

        state.dispatch(&Action::SearchBackspace);
        assert_eq!(state.search_query, "s");

        state.dispatch(&Action::ExitSearch);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert_eq!(state.search_query, "s"); // kept
    }

    #[test]
    fn dispatch_clear_search() {
        let mut state = test_state();
        state.dispatch(&Action::EnterSearch);
        state.dispatch(&Action::SearchInput('x'));
        state.dispatch(&Action::ClearSearch);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn filtered_peers_matches() {
        let mut state = test_state();
        state.search_query = "sjc".into();
        let matched: Vec<_> = state.filtered_peers().collect();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].config.name, "dc-sjc01");
    }

    #[test]
    fn filtered_peers_empty_query_returns_all() {
        let state = test_state();
        let matched: Vec<_> = state.filtered_peers().collect();
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn peers_initialized_disconnected() {
        let state = test_state();
        for peer in &state.peers {
            assert!(!peer.connected);
            assert_eq!(peer.stats.tx_bytes, 0);
        }
    }

    #[test]
    fn dispatch_update_peers() {
        use ferro_wg_core::ipc::PeerStatus;

        let mut state = test_state();
        assert!(!state.daemon_connected);

        let statuses = vec![PeerStatus {
            name: "dc-sjc01".into(),
            connected: true,
            backend: BackendKind::Neptun,
            stats: TunnelStats {
                tx_bytes: 1000,
                rx_bytes: 2000,
                ..TunnelStats::default()
            },
            endpoint: Some("198.51.100.1:51820".into()),
            interface: Some("utun4".into()),
        }];

        state.dispatch(&Action::UpdatePeers(statuses));
        assert!(state.daemon_connected);

        let sjc = &state.peers[0];
        assert!(sjc.connected);
        assert_eq!(sjc.stats.tx_bytes, 1000);
        assert_eq!(sjc.stats.rx_bytes, 2000);
        assert_eq!(sjc.backend, BackendKind::Neptun);

        // Unmatched peer stays disconnected.
        let ord = &state.peers[1];
        assert!(!ord.connected);
    }

    #[test]
    fn dispatch_daemon_connectivity() {
        let mut state = test_state();
        state.dispatch(&Action::DaemonConnectivityChanged(true));
        assert!(state.daemon_connected);
        state.dispatch(&Action::DaemonConnectivityChanged(false));
        assert!(!state.daemon_connected);
    }

    #[test]
    fn dispatch_daemon_feedback() {
        let mut state = test_state();

        state.dispatch(&Action::DaemonOk("tunnel up".into()));
        assert!(state.feedback.is_some());
        let fb = state.feedback.as_ref().unwrap();
        assert!(!fb.is_error);
        assert_eq!(fb.message, "tunnel up");
        assert!(!fb.is_expired());

        state.dispatch(&Action::DaemonError("not found".into()));
        let fb = state.feedback.as_ref().unwrap();
        assert!(fb.is_error);
        assert_eq!(fb.message, "not found");
    }

    #[test]
    fn clear_expired_feedback_removes_old() {
        let mut state = test_state();
        state.feedback = Some(Feedback {
            message: "old".into(),
            is_error: false,
            expires_at: Instant::now() - Duration::from_secs(1),
        });
        state.clear_expired_feedback();
        assert!(state.feedback.is_none());
    }

    #[test]
    fn clear_expired_feedback_keeps_fresh() {
        let mut state = test_state();
        state.dispatch(&Action::DaemonOk("fresh".into()));
        state.clear_expired_feedback();
        assert!(state.feedback.is_some());
    }
}
