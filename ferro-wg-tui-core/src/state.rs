//! Centralized TUI application state.
//!
//! [`AppState`] owns all shared data (connections, logs, theme) and
//! processes [`Action`]s via [`dispatch()`](AppState::dispatch). Components
//! receive `&AppState` for read-only access during rendering and key
//! handling.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ferro_wg_core::config::{AppConfig, LogDisplayConfig, PeerConfig, WgConfig};
use ferro_wg_core::error::BackendKind;
use ferro_wg_core::ipc::{BenchmarkProgress, LogEntry};
use ferro_wg_core::key::PublicKey;
use ferro_wg_core::stats::TunnelStats;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;

use crossterm::event::KeyCode;

use crate::action::{Action, ConfirmAction};
use crate::app::{InputMode, Tab};
use crate::benchmark::{
    cap_history, BenchmarkResultMap, BenchmarkRun, BENCHMARK_HISTORY_CAP,
    BENCHMARK_PROGRESS_HISTORY_CAP,
};
use crate::config_edit::{
    fields_for_section, ConfigDiffPending, ConfigEditState, ConfigSection, DiffLine,
};
use crate::theme::Theme;

/// How long feedback messages are displayed before expiring.
const FEEDBACK_DURATION: Duration = Duration::from_secs(3);

/// Which view mode is active on the Compare tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompareView {
    /// Show live benchmark results and the running progress widgets.
    #[default]
    Live,
    /// Show the scrollable list of historical `BenchmarkRun` entries.
    Historical,
}

/// Whether a connection tunnel is currently active.
///
/// A plain `bool` is insufficient here because it cannot express the
/// absence of data (`None` on `ConnectionStatus`). The enum is kept
/// minimal for Phase 2; a `Connecting` variant may be added in Phase 4
/// if the daemon gains handshake-in-progress signalling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// The tunnel is up and routing traffic.
    Connected,
    /// The tunnel is down.
    Disconnected,
}

/// Stale-handshake threshold: a connected tunnel whose last successful
/// handshake is older than this is considered unhealthy.
const STALE_HANDSHAKE: Duration = Duration::from_secs(180);

/// Packet-loss threshold above which a connected tunnel is considered unhealthy.
const HIGH_PACKET_LOSS: f32 = 0.1;

/// Live status for one connection, sourced from a `PeerStatus` daemon response.
#[derive(Debug, Clone)]
pub struct ConnectionStatus {
    /// Whether the tunnel is currently active.
    pub state: ConnectionState,
    /// Which backend is active.
    pub backend: BackendKind,
    /// Current tunnel statistics.
    pub stats: TunnelStats,
    /// The peer's endpoint (hostname:port or ip:port).
    pub endpoint: Option<String>,
    /// The local TUN interface name (e.g. `utun4`).
    pub interface: Option<String>,
    /// Short human-readable health warning, `None` when the tunnel is healthy.
    ///
    /// Only set for [`ConnectionState::Connected`] tunnels; always `None`
    /// when the tunnel is down.
    pub health_warning: Option<String>,
}

/// Derive a health warning from tunnel statistics for a **connected** tunnel.
///
/// Returns `None` when the tunnel appears healthy. Stale handshake takes
/// priority over high packet loss when both conditions are present.
///
/// # Arguments
///
/// * `stats` — statistics snapshot for the active tunnel.
#[must_use]
pub fn compute_health_warning(stats: &TunnelStats) -> Option<String> {
    // Stale handshake: reported age exceeds the threshold.
    if stats
        .last_handshake
        .is_some_and(|age| age > STALE_HANDSHAKE)
    {
        return Some("stale handshake".to_owned());
    }
    // High packet loss.
    if stats.packet_loss > HIGH_PACKET_LOSS {
        return Some(format!(
            "high packet loss ({:.0}%)",
            stats.packet_loss * 100.0
        ));
    }
    None
}

/// Static config and live status for one named connection.
#[derive(Debug, Clone)]
pub struct ConnectionView {
    /// Connection name as it appears in `AppConfig` (e.g. `"mia"`).
    pub name: String,
    /// Static `WireGuard` config (interface + peers).
    pub config: WgConfig,
    /// Live status from the last daemon poll; `None` until the first poll
    /// completes.
    pub status: Option<ConnectionStatus>,
    /// Which peer row is selected in the Status/Peers tabs for this
    /// connection. Preserved when switching away and back.
    pub selected_peer_row: usize,
}

/// A pending action awaiting user confirmation.
///
/// Stored on [`AppState`] while the confirmation overlay is visible.
/// Cleared by [`Action::ConfirmYes`] or [`Action::ConfirmNo`].
#[derive(Debug, Clone)]
pub struct ConfirmPending {
    /// The message shown in the confirmation overlay.
    pub message: String,
    /// The action to execute if the user confirms.
    pub action: ConfirmAction,
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
/// data — they receive `&AppState` for read-only access during rendering.
pub struct AppState {
    /// Whether the app is still running.
    pub running: bool,
    /// Currently selected tab.
    pub active_tab: Tab,
    /// Input mode (normal vs search).
    pub input_mode: InputMode,
    /// Search query string.
    pub search_query: String,
    /// All configured connections in display order (sorted by name).
    pub connections: Vec<ConnectionView>,
    /// Index into `connections` for the currently focused connection.
    /// Always 0 when `connections` is empty.
    pub selected_connection: usize,
    /// Structured log entries for the Logs tab.
    pub log_entries: Arc<Mutex<VecDeque<LogEntry>>>,
    /// Active color theme.
    pub theme: Theme,
    /// Whether the daemon is currently reachable.
    pub daemon_connected: bool,
    /// Transient feedback message (success or error) with expiry.
    pub feedback: Option<Feedback>,
    /// Log display preferences forwarded from [`AppConfig`].
    pub log_display: LogDisplayConfig,
    /// Pending confirmation dialog, or `None` when no dialog is active.
    pub pending_confirm: Option<ConfirmPending>,
    /// Latest benchmark result per backend name for the **current active connection**.
    ///
    /// Keyed by `BenchmarkResult::backend` (a `String`).
    /// Cleared (set to empty `HashMap`) in `dispatch(StartBenchmark)` and in
    /// `dispatch(StartBenchmarkForBackend(_))` so stale results from a previous
    /// run never appear next to results from a new run. Each individual
    /// `BenchmarkComplete` result is inserted by backend key, so a partial
    /// all-backends run accumulates results incrementally.
    pub benchmark_results: BenchmarkResultMap,
    /// Benchmark history, capped at 50 runs; loaded from `benchmarks.json`
    /// at startup and appended to on `BenchmarkComplete`.
    pub benchmark_history: Vec<BenchmarkRun>,
    /// `true` while a benchmark task is running; prevents concurrent runs.
    ///
    /// Set to `true` in `dispatch(StartBenchmark)` **only when not already
    /// running**. Set back to `false` in `dispatch(BenchmarkComplete)`.
    /// The action/effect layer's `maybe_spawn_command` calls
    /// `spawn_benchmark_task` when it sees `StartBenchmark` AND
    /// `state.benchmark_running` is still `false` at that point (checked
    /// against pre-dispatch state captured before `dispatch_all`).
    pub benchmark_running: bool,
    /// Ring buffer of live progress samples from the current benchmark run.
    ///
    /// `VecDeque` is used so the oldest sample can be dropped from the front
    /// in O(1) when the buffer is capped (maximum 60 samples — one minute of
    /// one-per-second updates). Cleared to empty on `BenchmarkComplete` and on
    /// `StartBenchmark`.
    ///
    /// The `Sparkline` and `Gauge` widgets are driven from this field via
    /// `benchmark::throughput_sparkline_data(&state.benchmark_progress_history)`.
    pub benchmark_progress_history: VecDeque<BenchmarkProgress>,
    /// Which view mode is active on the Compare tab.
    pub compare_view: CompareView,
    /// Pending config edit session, `Some` while the Config tab is in edit mode.
    ///
    /// Cleared on `DiscardConfigEdits`, `SaveConfig`, or `ConfirmNo` after
    /// a `DeletePeer` dialog.
    pub config_edit: Option<ConfigEditState>,
    /// Pending diff preview, `Some` when the diff overlay is shown.
    ///
    /// Cleared on `SaveConfig` (success or error) or `Esc` in the overlay.
    pub config_diff_pending: Option<ConfigDiffPending>,
}

impl AppState {
    /// Create a new state from the full application config.
    ///
    /// Connections are stored in alphabetical order by name.
    /// An empty `AppConfig` produces `connections: vec![]` and
    /// `selected_connection: 0`; all accessors return `None` gracefully.
    #[must_use]
    pub fn new(app_config: AppConfig) -> Self {
        // BTreeMap is already sorted by key, so iteration order is alphabetical.
        let connections = app_config
            .connections
            .into_iter()
            .map(|(name, config)| ConnectionView {
                name,
                config,
                status: None,
                selected_peer_row: 0,
            })
            .collect();

        Self {
            running: true,
            active_tab: Tab::Overview,
            input_mode: InputMode::Normal,
            search_query: String::new(),
            connections,
            selected_connection: 0,
            log_entries: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
            theme: Theme::mocha(),
            daemon_connected: false,
            feedback: None,
            log_display: app_config.log_display,
            pending_confirm: None,
            benchmark_results: BenchmarkResultMap::new(),
            benchmark_history: Vec::new(),
            benchmark_running: false,
            benchmark_progress_history: VecDeque::new(),
            compare_view: CompareView::default(),
            config_edit: None,
            config_diff_pending: None,
        }
    }

    /// Returns the currently focused connection, if any.
    ///
    /// Returns `None` when `connections` is empty.
    #[must_use]
    pub fn active_connection(&self) -> Option<&ConnectionView> {
        self.connections.get(self.selected_connection)
    }

    /// Append a structured log entry to the buffer, evicting the oldest when full.
    pub fn append_log(&self, entry: LogEntry) {
        match self.log_entries.lock() {
            Ok(mut buf) => {
                if buf.len() == buf.capacity() {
                    buf.pop_front();
                }
                buf.push_back(entry);
            }
            Err(_) => {
                warn!("Log buffer mutex poisoned, skipping log append");
            }
        }
    }

    /// Dispatch an action to update the application state.
    ///
    /// This is the central hub for all state mutations. After dispatch
    /// returns, the caller should forward the action to all components
    /// via [`Component::update()`](crate::component::Component::update).
    ///
    /// `SelectConnection(i)` with an out-of-bounds index is silently
    /// ignored and emits a `tracing::warn!` log entry; it does not panic.
    /// # Panics
    ///
    /// Panics if `SystemTime` is before `UNIX_EPOCH` (impossible in practice).
    #[allow(clippy::too_many_lines, clippy::cast_possible_truncation)]
    pub fn dispatch(&mut self, action: &Action) {
        match action {
            Action::Quit => self.running = false,
            Action::NextTab => {
                if self.input_mode == InputMode::EditField {
                    if let Some(edit) = self.config_edit.as_mut() {
                        edit.edit_buffer = None;
                        edit.field_error = None;
                    }
                    self.input_mode = InputMode::Normal;
                }
                self.active_tab = self.active_tab.next();
            }
            Action::PrevTab => {
                if self.input_mode == InputMode::EditField {
                    if let Some(edit) = self.config_edit.as_mut() {
                        edit.edit_buffer = None;
                        edit.field_error = None;
                    }
                    self.input_mode = InputMode::Normal;
                }
                self.active_tab = self.active_tab.prev();
            }
            Action::SelectTab(tab) => {
                if self.input_mode == InputMode::EditField {
                    if let Some(edit) = self.config_edit.as_mut() {
                        edit.edit_buffer = None;
                        edit.field_error = None;
                    }
                    self.input_mode = InputMode::Normal;
                }
                self.active_tab = *tab;
            }
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
            Action::SelectNextConnection => {
                if !self.connections.is_empty() {
                    self.selected_connection =
                        (self.selected_connection + 1) % self.connections.len();
                    self.search_query.clear();
                }
            }
            Action::SelectPrevConnection => {
                if !self.connections.is_empty() {
                    self.selected_connection = self
                        .selected_connection
                        .checked_sub(1)
                        .unwrap_or(self.connections.len() - 1);
                    self.search_query.clear();
                }
            }
            Action::SelectConnection(i) => {
                if *i >= self.connections.len() {
                    warn!(
                        i,
                        len = self.connections.len(),
                        "SelectConnection index out of bounds; ignoring"
                    );
                } else {
                    self.selected_connection = *i;
                    self.search_query.clear();
                }
            }
            Action::UpdatePeers(statuses) => self.apply_peer_updates(statuses),
            Action::DaemonConnectivityChanged(connected) => {
                self.daemon_connected = *connected;
            }
            Action::DaemonOk(msg) => {
                self.feedback = Some(Feedback::success(msg.clone()));
            }
            Action::DaemonError(msg) => {
                self.feedback = Some(Feedback::error(msg.clone()));
            }
            Action::NextRow => {
                if let Some(conn) = self.connections.get_mut(self.selected_connection) {
                    let max = conn.config.peers.len().saturating_sub(1);
                    conn.selected_peer_row = (conn.selected_peer_row + 1).min(max);
                }
            }
            Action::PrevRow => {
                if let Some(conn) = self.connections.get_mut(self.selected_connection) {
                    conn.selected_peer_row = conn.selected_peer_row.saturating_sub(1);
                }
            }
            // -- wg-quick import --
            Action::EnterImport => {
                self.input_mode = InputMode::Import(String::new());
            }
            Action::ImportKey(key) => self.apply_import_key(key),

            // -- Benchmark actions --
            Action::StartBenchmark | Action::StartBenchmarkForBackend(_) => {
                if self.benchmark_running {
                    self.feedback = Some(Feedback::error("benchmark already running".to_owned()));
                } else {
                    self.benchmark_running = true;
                    self.benchmark_results.clear();
                    self.benchmark_progress_history.clear();
                }
            }
            Action::BenchmarkProgressUpdate(p) => {
                if self.benchmark_progress_history.len() >= BENCHMARK_PROGRESS_HISTORY_CAP {
                    self.benchmark_progress_history.pop_front();
                }
                self.benchmark_progress_history.push_back(p.clone());
            }
            Action::BenchmarkComplete(result) => {
                self.benchmark_running = false;
                self.benchmark_results
                    .insert(result.backend.clone(), result.clone());
                let run = BenchmarkRun {
                    timestamp_ms: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as i64, // Safe as millis since epoch fits in i64
                    connection_name: self
                        .active_connection()
                        .map(|c| c.name.clone())
                        .unwrap_or_default(),
                    results: self.benchmark_results.clone(),
                };
                self.benchmark_history.push(run);
                self.benchmark_history = cap_history(
                    std::mem::take(&mut self.benchmark_history),
                    BENCHMARK_HISTORY_CAP,
                );
            }
            Action::ToggleCompareView => {
                self.compare_view = match self.compare_view {
                    CompareView::Live => CompareView::Historical,
                    CompareView::Historical => CompareView::Live,
                };
            }
            Action::EnterExport => {
                self.input_mode = InputMode::Export(String::new());
            }
            Action::ExportKey(key) => self.apply_export_key(key),
            Action::SubmitImport
            | Action::ExitImport
            | Action::SubmitExport
            | Action::ExitExport => {
                self.input_mode = InputMode::Normal;
            }
            // -- Confirmation dialog --
            Action::RequestConfirm { message, action } => {
                self.pending_confirm = Some(ConfirmPending {
                    message: message.clone(),
                    action: action.clone(),
                });
            }
            Action::ConfirmYes => {
                if let Some(pending) = self.pending_confirm.take() {
                    if let ConfirmAction::DeletePeer(i) = pending.action {
                        self.dispatch(&Action::DeleteConfigPeer(i));
                    }
                }
            }
            Action::ConfirmNo => {
                self.pending_confirm = None;
            }

            // -- Config editing --
            Action::ConfigEditKey(key) => {
                if let Some(edit) = self.config_edit.as_mut() {
                    if let Some(ref mut buf) = edit.edit_buffer {
                        match key.code {
                            KeyCode::Char(c) => buf.push(c),
                            KeyCode::Backspace => {
                                buf.pop();
                            }
                            KeyCode::Enter => {
                                // Commit
                                let _ = edit.edit_buffer.take();
                                edit.field_error = None;
                                self.input_mode = InputMode::Normal;
                            }
                            KeyCode::Esc => {
                                // Cancel
                                edit.edit_buffer = None;
                                edit.field_error = None;
                                self.input_mode = InputMode::Normal;
                            }
                            _ => {}
                        }
                    }
                }
            }

            Action::ConfigFocusNext => {
                if let Some(edit) = self.config_edit.as_mut() {
                    let fields = fields_for_section(edit.focused_section, false); // Assume not new peer
                    edit.focused_field_idx = (edit.focused_field_idx + 1) % fields.len();
                }
            }
            Action::ConfigFocusPrev => {
                if let Some(edit) = self.config_edit.as_mut() {
                    let fields = fields_for_section(edit.focused_section, false);
                    edit.focused_field_idx = edit.focused_field_idx.saturating_sub(1);
                }
            }
            Action::ConfigFocusInterface => {
                if let Some(edit) = self.config_edit.as_mut() {
                    edit.focused_section = ConfigSection::Interface;
                    edit.focused_field_idx = 0;
                }
            }
            Action::ConfigFocusPeer(i) => {
                if let Some(edit) = self.config_edit.as_mut() {
                    edit.focused_section = ConfigSection::Peer(*i);
                    edit.focused_field_idx = 0;
                }
            }
            Action::AddConfigPeer => {
                if let Some(edit) = self.config_edit.as_mut() {
                    let dummy_key =
                        PublicKey::from_base64("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")
                            .unwrap();
                    let new_peer = PeerConfig {
                        name: "".to_string(),
                        public_key: dummy_key,
                        endpoint: None,
                        allowed_ips: vec![],
                        persistent_keepalive: 0,
                        preshared_key: None,
                    };
                    edit.draft.peers.push(new_peer);
                    let idx = edit.draft.peers.len() - 1;
                    edit.focused_section = ConfigSection::Peer(idx);
                    edit.focused_field_idx = 0;
                    self.input_mode = InputMode::EditField;
                }
            }
            Action::DeleteConfigPeer(i) => {
                if let Some(edit) = self.config_edit.as_mut() {
                    if *i < edit.draft.peers.len() {
                        edit.draft.peers.remove(*i);
                        // Clamp focus
                        if edit.draft.peers.is_empty() {
                            edit.focused_field_idx = 0;
                        } else {
                            edit.focused_field_idx =
                                edit.focused_field_idx.min(edit.draft.peers.len() - 1);
                        }
                    }
                }
            }
            Action::PreviewConfig => {
                if let Some(edit) = self.config_edit.as_ref() {
                    if edit.field_error.is_none() {
                        // Mock diff for now
                        let diff_lines = vec![DiffLine::Context("mock".to_string())];
                        self.config_diff_pending = Some(ConfigDiffPending {
                            connection_name: edit.connection_name.clone(),
                            draft: edit.draft.clone(),
                            diff_lines,
                            scroll_offset: 0,
                        });
                    }
                }
            }
            Action::ConfigDiffScrollDown => {
                if let Some(pending) = self.config_diff_pending.as_mut() {
                    pending.scroll_offset = pending.scroll_offset.saturating_add(1);
                }
            }
            Action::ConfigDiffScrollUp => {
                if let Some(pending) = self.config_diff_pending.as_mut() {
                    pending.scroll_offset = pending.scroll_offset.saturating_sub(1);
                }
            }
            Action::SaveConfig { .. } => {
                self.config_edit = None;
                self.config_diff_pending = None;
            }
            Action::DiscardConfigEdits => {
                self.config_edit = None;
                self.config_diff_pending = None;
                self.input_mode = InputMode::Normal;
            }
            // These are handled by the event loop (maybe_spawn_command) or
            // components. They carry no state-machine side-effects here.
            Action::Tick
            | Action::ConnectPeer(_)
            | Action::DisconnectPeer(_)
            | Action::CyclePeerBackend(_)
            | Action::ConnectAll
            | Action::DisconnectAll
            | Action::StartDaemon
            | Action::StopDaemon
            | Action::SwitchBenchmarkBackend(_) => {}
            _ => {}
        }
    }

    /// Handle a key event when in [`InputMode::Import`].
    ///
    /// Appends typed characters to the path buffer and removes the last
    /// character on `Backspace`. All other keys are silently ignored.
    fn apply_import_key(&mut self, key: &crossterm::event::KeyEvent) {
        if let InputMode::Import(ref mut buf) = self.input_mode {
            match key.code {
                KeyCode::Char(c) => buf.push(c),
                KeyCode::Backspace => {
                    buf.pop();
                }
                _ => {}
            }
        }
    }

    ///
    /// Appends typed characters to the path buffer and removes the last
    /// character on `Backspace`. All other keys are silently ignored.
    fn apply_export_key(&mut self, key: &crossterm::event::KeyEvent) {
        if let InputMode::Export(ref mut buf) = self.input_mode {
            match key.code {
                KeyCode::Char(c) => buf.push(c),
                KeyCode::Backspace => {
                    buf.pop();
                }
                _ => {}
            }
        }
    }

    /// Apply a batch of peer status updates from a daemon poll.
    ///
    /// Marks the daemon as reachable, updates each named connection, and
    /// clamps `selected_connection` into bounds in case the list changed.
    fn apply_peer_updates(&mut self, statuses: &[ferro_wg_core::ipc::PeerStatus]) {
        self.daemon_connected = true;
        for s in statuses {
            if let Some(conn) = self.connections.iter_mut().find(|c| c.name == s.name) {
                let state = if s.connected {
                    ConnectionState::Connected
                } else {
                    ConnectionState::Disconnected
                };
                let health_warning = if s.connected {
                    compute_health_warning(&s.stats)
                } else {
                    None
                };
                conn.status = Some(ConnectionStatus {
                    state,
                    backend: s.backend,
                    stats: s.stats.clone(),
                    endpoint: s.endpoint.clone(),
                    interface: s.interface.clone(),
                    health_warning,
                });
            } else {
                warn!(name = %s.name, "UpdatePeers received status for unknown connection");
            }
        }
        // Clamp in case connections changed (defensive; static in Phase 2).
        self.selected_connection = self
            .selected_connection
            .min(self.connections.len().saturating_sub(1));
    }

    /// The current import path buffer, when in [`InputMode::Import`].
    ///
    /// Returns `None` when not in import mode.
    #[must_use]
    pub fn import_buffer(&self) -> Option<&str> {
        if let InputMode::Import(ref buf) = self.input_mode {
            Some(buf.as_str())
        } else {
            None
        }
    }

    /// The current export path buffer, when in [`InputMode::Export`].
    ///
    /// Returns `None` when not in export mode.
    #[must_use]
    pub fn export_buffer(&self) -> Option<&str> {
        if let InputMode::Export(ref buf) = self.input_mode {
            Some(buf.as_str())
        } else {
            None
        }
    }

    /// Rebuild the connection list from a new [`AppConfig`].
    ///
    /// Preserves `selected_connection` by clamping it into bounds.
    /// All other connection state (live status, peer row) is reset because
    /// the daemon will push fresh status on the next poll.
    pub fn reload_from_config(&mut self, app_config: AppConfig) {
        let AppConfig {
            connections,
            log_display,
        } = app_config;
        self.connections = connections
            .into_iter()
            .map(|(name, config)| ConnectionView {
                name,
                config,
                status: None,
                selected_peer_row: 0,
            })
            .collect();
        self.log_display = log_display;
        self.selected_connection = self
            .selected_connection
            .min(self.connections.len().saturating_sub(1));
    }

    /// Clear expired feedback messages. Called on each tick.
    pub fn clear_expired_feedback(&mut self) {
        if self.feedback.as_ref().is_some_and(Feedback::is_expired) {
            self.feedback = None;
        }
    }

    /// Peers from the active connection matching the current search query.
    ///
    /// Returns an empty iterator when there is no active connection.
    /// Returns all peers when the query is empty. Matches against
    /// the peer name and endpoint (case-insensitive substring).
    pub fn filtered_peers(&self) -> impl Iterator<Item = &ferro_wg_core::config::PeerConfig> {
        let query = self.search_query.to_lowercase();
        let peers: &[ferro_wg_core::config::PeerConfig] = self
            .connections
            .get(self.selected_connection)
            .map_or(&[], |c| c.config.peers.as_slice());
        peers.iter().filter(move |p| {
            query.is_empty()
                || p.name.to_lowercase().contains(&query)
                || p.endpoint
                    .as_ref()
                    .is_some_and(|ep| ep.to_lowercase().contains(&query))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ferro_wg_core::config::{InterfaceConfig, PeerConfig};
    use ferro_wg_core::ipc::PeerStatus;
    use ferro_wg_core::key::PrivateKey;
    use ferro_wg_core::stats::TunnelStats;

    use super::*;

    // ── Helpers ──────────────────────────────────────────────────────────────

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

    fn make_wg_config(peers: Vec<PeerConfig>) -> WgConfig {
        WgConfig {
            interface: make_interface(),
            peers,
        }
    }

    /// Build an `AppConfig` from a list of `(name, peers)` pairs.
    fn make_app_config(entries: &[(&str, Vec<PeerConfig>)]) -> AppConfig {
        let mut connections = BTreeMap::new();
        for (name, peers) in entries {
            connections.insert((*name).to_string(), make_wg_config(peers.clone()));
        }
        AppConfig {
            connections,
            log_display: LogDisplayConfig::default(),
        }
    }

    fn make_peer_status(name: &str, connected: bool) -> PeerStatus {
        PeerStatus {
            name: name.into(),
            connected,
            backend: BackendKind::Boringtun,
            stats: TunnelStats::default(),
            endpoint: None,
            interface: None,
        }
    }

    fn two_connection_state() -> AppState {
        AppState::new(make_app_config(&[
            ("mia", vec![make_peer("mia-dc")]),
            ("ord01", vec![make_peer("ord01-dc")]),
        ]))
    }

    // ── Existing tests (updated for new structure) ────────────────────────────

    #[test]
    fn initial_state() {
        let state = two_connection_state();
        assert!(state.running);
        assert_eq!(state.active_tab, Tab::Overview);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert_eq!(state.connections.len(), 2);
        assert_eq!(state.selected_connection, 0);
    }

    #[test]
    fn dispatch_quit() {
        let mut state = two_connection_state();
        state.dispatch(&Action::Quit);
        assert!(!state.running);
    }

    #[test]
    fn dispatch_tab_navigation() {
        let mut state = two_connection_state();
        state.dispatch(&Action::NextTab);
        assert_eq!(state.active_tab, Tab::Status);
        state.dispatch(&Action::PrevTab);
        assert_eq!(state.active_tab, Tab::Overview);
        state.dispatch(&Action::SelectTab(Tab::Compare));
        assert_eq!(state.active_tab, Tab::Compare);
    }

    #[test]
    fn dispatch_search_lifecycle() {
        let mut state = two_connection_state();
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
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterSearch);
        state.dispatch(&Action::SearchInput('x'));
        state.dispatch(&Action::ClearSearch);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn dispatch_daemon_connectivity() {
        let mut state = two_connection_state();
        state.dispatch(&Action::DaemonConnectivityChanged(true));
        assert!(state.daemon_connected);
        state.dispatch(&Action::DaemonConnectivityChanged(false));
        assert!(!state.daemon_connected);
    }

    #[test]
    fn dispatch_daemon_feedback() {
        let mut state = two_connection_state();

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
        let mut state = two_connection_state();
        state.feedback = Some(Feedback {
            message: "old".into(),
            is_error: false,
            expires_at: Instant::now().checked_sub(Duration::from_secs(1)).unwrap(),
        });
        state.clear_expired_feedback();
        assert!(state.feedback.is_none());
    }

    #[test]
    fn clear_expired_feedback_keeps_fresh() {
        let mut state = two_connection_state();
        state.dispatch(&Action::DaemonOk("fresh".into()));
        state.clear_expired_feedback();
        assert!(state.feedback.is_some());
    }

    // ── New Phase 2 Step 1 tests ──────────────────────────────────────────────

    #[test]
    fn connections_sorted_on_new() {
        let state = AppState::new(make_app_config(&[
            ("zzz", vec![]),
            ("aaa", vec![]),
            ("mmm", vec![]),
        ]));
        let names: Vec<&str> = state.connections.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["aaa", "mmm", "zzz"]);
    }

    #[test]
    fn new_empty_config() {
        let state = AppState::new(AppConfig::default());
        assert!(state.connections.is_empty());
        assert_eq!(state.selected_connection, 0);
        assert!(state.active_connection().is_none());
    }

    #[test]
    fn update_peers_routes_by_name() {
        let mut state = two_connection_state();
        let statuses = vec![
            make_peer_status("mia", true),
            make_peer_status("ord01", false),
        ];
        state.dispatch(&Action::UpdatePeers(statuses));

        let mia = state.connections.iter().find(|c| c.name == "mia").unwrap();
        assert_eq!(
            mia.status.as_ref().unwrap().state,
            ConnectionState::Connected
        );

        let ord = state
            .connections
            .iter()
            .find(|c| c.name == "ord01")
            .unwrap();
        assert_eq!(
            ord.status.as_ref().unwrap().state,
            ConnectionState::Disconnected
        );
    }

    #[test]
    fn update_peers_partial() {
        let mut state = two_connection_state();
        // Update only mia.
        state.dispatch(&Action::UpdatePeers(vec![make_peer_status("mia", true)]));

        let mia = state.connections.iter().find(|c| c.name == "mia").unwrap();
        assert!(mia.status.is_some());

        // ord01 still has no status.
        let ord = state
            .connections
            .iter()
            .find(|c| c.name == "ord01")
            .unwrap();
        assert!(ord.status.is_none());
    }

    #[test]
    fn update_peers_unknown_name_ignored() {
        let mut state = two_connection_state();
        // Should not panic; connections remain unaffected.
        state.dispatch(&Action::UpdatePeers(vec![make_peer_status(
            "unknown-connection",
            true,
        )]));
        for conn in &state.connections {
            assert!(conn.status.is_none());
        }
    }

    #[test]
    fn select_next_wraps() {
        let mut state = two_connection_state();
        state.selected_connection = 1; // last index
        state.dispatch(&Action::SelectNextConnection);
        assert_eq!(state.selected_connection, 0);
    }

    #[test]
    fn select_prev_wraps() {
        let mut state = two_connection_state();
        assert_eq!(state.selected_connection, 0);
        state.dispatch(&Action::SelectPrevConnection);
        assert_eq!(state.selected_connection, 1); // wraps to last
    }

    #[test]
    fn select_next_empty() {
        let mut state = AppState::new(AppConfig::default());
        // Must not panic.
        state.dispatch(&Action::SelectNextConnection);
        assert_eq!(state.selected_connection, 0);
    }

    #[test]
    fn select_connection_out_of_bounds() {
        let mut state = two_connection_state();
        state.dispatch(&Action::SelectConnection(99));
        // Silently ignored; selection unchanged.
        assert_eq!(state.selected_connection, 0);
    }

    #[test]
    fn select_connection_clears_search_query() {
        let mut state = two_connection_state();
        state.search_query = "mia".into();
        state.dispatch(&Action::SelectConnection(1));
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn select_connection_out_of_bounds_does_not_clear_search_query() {
        // Out-of-bounds SelectConnection is a no-op; search is preserved.
        let mut state = two_connection_state();
        state.search_query = "mia".into();
        state.dispatch(&Action::SelectConnection(99));
        assert_eq!(state.search_query, "mia");
    }

    #[test]
    fn select_next_clears_search_query() {
        let mut state = two_connection_state();
        state.search_query = "sjc".into();
        state.dispatch(&Action::SelectNextConnection);
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn select_prev_clears_search_query() {
        let mut state = two_connection_state();
        state.selected_connection = 1;
        state.search_query = "ord".into();
        state.dispatch(&Action::SelectPrevConnection);
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn next_prev_row_per_connection() {
        let mut state = AppState::new(make_app_config(&[
            ("mia", vec![make_peer("p1"), make_peer("p2")]),
            ("ord01", vec![make_peer("p3"), make_peer("p4")]),
        ]));
        // Move row cursor on connection 0 (mia).
        state.dispatch(&Action::NextRow);
        assert_eq!(state.connections[0].selected_peer_row, 1);
        // Connection 1 (ord01) is unaffected.
        assert_eq!(state.connections[1].selected_peer_row, 0);
    }

    #[test]
    fn next_row_clamps_at_end() {
        let mut state = AppState::new(make_app_config(&[("mia", vec![make_peer("p1")])]));
        state.dispatch(&Action::NextRow);
        state.dispatch(&Action::NextRow); // already at last index
        assert_eq!(state.connections[0].selected_peer_row, 0); // only 1 peer → max index 0
    }

    #[test]
    fn prev_row_clamps_at_zero() {
        let mut state = two_connection_state();
        state.dispatch(&Action::PrevRow);
        assert_eq!(state.connections[0].selected_peer_row, 0);
    }

    #[test]
    fn next_row_no_connection_no_panic() {
        let mut state = AppState::new(AppConfig::default());
        state.dispatch(&Action::NextRow);
        // No panic, no state change.
        assert_eq!(state.selected_connection, 0);
    }

    #[test]
    fn filtered_peers_matches_active_connection() {
        let mut state = AppState::new(make_app_config(&[
            ("mia", vec![make_peer("sjc01"), make_peer("ord01")]),
            ("tus1", vec![make_peer("tus1-dc")]),
        ]));
        state.search_query = "sjc".into();
        let matched: Vec<_> = state.filtered_peers().collect();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].name, "sjc01");
    }

    #[test]
    fn filtered_peers_empty_query_returns_all() {
        let state = AppState::new(make_app_config(&[(
            "mia",
            vec![make_peer("p1"), make_peer("p2")],
        )]));
        let matched: Vec<_> = state.filtered_peers().collect();
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn filtered_peers_no_connection_returns_empty() {
        let state = AppState::new(AppConfig::default());
        let matched: Vec<_> = state.filtered_peers().collect();
        assert!(matched.is_empty());
    }

    #[test]
    fn connections_initialized_no_status() {
        let state = two_connection_state();
        for conn in &state.connections {
            assert!(conn.status.is_none());
            assert_eq!(conn.selected_peer_row, 0);
        }
    }

    #[test]
    fn update_peers_sets_daemon_connected() {
        let mut state = two_connection_state();
        assert!(!state.daemon_connected);
        state.dispatch(&Action::UpdatePeers(vec![make_peer_status("mia", true)]));
        assert!(state.daemon_connected);
    }

    // ── Log entry tests ───────────────────────────────────────────────────────

    fn make_log_entry(msg: &str) -> LogEntry {
        LogEntry {
            timestamp_ms: 0,
            level: ferro_wg_core::ipc::LogLevel::Info,
            connection_name: None,
            message: msg.to_owned(),
        }
    }

    #[test]
    fn append_log_grows_buffer() {
        let state = two_connection_state();
        assert_eq!(state.log_entries.lock().unwrap().len(), 0);
        state.append_log(make_log_entry("hello"));
        state.append_log(make_log_entry("world"));
        assert_eq!(state.log_entries.lock().unwrap().len(), 2);
    }

    #[test]
    fn append_log_evicts_oldest_at_capacity() {
        let state = two_connection_state();
        // Fill the buffer to capacity.
        for i in 0..1000 {
            state.append_log(make_log_entry(&format!("line{i}")));
        }
        assert_eq!(state.log_entries.lock().unwrap().len(), 1000);
        // One more: oldest should be evicted.
        state.append_log(make_log_entry("overflow"));
        let buf = state.log_entries.lock().unwrap();
        assert_eq!(buf.len(), 1000);
        assert_eq!(buf.back().unwrap().message, "overflow");
        assert_eq!(buf.front().unwrap().message, "line1");
    }

    // ── Health indicator tests ────────────────────────────────────────────────

    #[test]
    fn compute_health_warning_healthy_tunnel() {
        let stats = TunnelStats {
            last_handshake: Some(Duration::from_secs(30)),
            packet_loss: 0.0,
            ..TunnelStats::default()
        };
        assert!(compute_health_warning(&stats).is_none());
    }

    #[test]
    fn compute_health_warning_stale_handshake() {
        let stats = TunnelStats {
            last_handshake: Some(Duration::from_secs(200)),
            packet_loss: 0.0,
            ..TunnelStats::default()
        };
        let warning = compute_health_warning(&stats);
        assert!(warning.is_some());
        assert_eq!(warning.unwrap(), "stale handshake");
    }

    #[test]
    fn compute_health_warning_high_packet_loss() {
        let stats = TunnelStats {
            last_handshake: Some(Duration::from_secs(30)),
            packet_loss: 0.5,
            ..TunnelStats::default()
        };
        let warning = compute_health_warning(&stats);
        assert!(warning.is_some());
        assert!(warning.unwrap().contains("packet loss"));
    }

    #[test]
    fn compute_health_warning_stale_takes_priority_over_packet_loss() {
        // Both conditions; stale handshake must win.
        let stats = TunnelStats {
            last_handshake: Some(Duration::from_secs(300)),
            packet_loss: 0.9,
            ..TunnelStats::default()
        };
        let warning = compute_health_warning(&stats).unwrap();
        assert_eq!(warning, "stale handshake");
    }

    #[test]
    fn compute_health_warning_no_handshake_yet() {
        // No handshake recorded → cannot be stale.
        let stats = TunnelStats {
            last_handshake: None,
            packet_loss: 0.0,
            ..TunnelStats::default()
        };
        assert!(compute_health_warning(&stats).is_none());
    }

    #[test]
    fn update_peers_sets_health_warning_for_stale_connected_peer() {
        let mut state = two_connection_state();
        let mut statuses = vec![PeerStatus {
            name: "mia".into(),
            connected: true,
            backend: BackendKind::Boringtun,
            stats: TunnelStats {
                last_handshake: Some(Duration::from_secs(300)), // stale
                packet_loss: 0.0,
                ..TunnelStats::default()
            },
            endpoint: None,
            interface: None,
        }];
        state.dispatch(&Action::UpdatePeers(statuses.clone()));
        let conn = state.connections.iter().find(|c| c.name == "mia").unwrap();
        let warning = conn.status.as_ref().unwrap().health_warning.as_deref();
        assert_eq!(warning, Some("stale handshake"));

        // A healthy reconnect must clear the warning.
        statuses[0].stats.last_handshake = Some(Duration::from_secs(10));
        state.dispatch(&Action::UpdatePeers(statuses));
        let conn = state.connections.iter().find(|c| c.name == "mia").unwrap();
        assert!(conn.status.as_ref().unwrap().health_warning.is_none());
    }

    #[test]
    fn update_peers_no_health_warning_when_disconnected() {
        let mut state = two_connection_state();
        // Even with stale stats, a disconnected peer must have no warning.
        state.dispatch(&Action::UpdatePeers(vec![PeerStatus {
            name: "mia".into(),
            connected: false,
            backend: BackendKind::Boringtun,
            stats: TunnelStats {
                last_handshake: Some(Duration::from_secs(9999)),
                packet_loss: 1.0,
                ..TunnelStats::default()
            },
            endpoint: None,
            interface: None,
        }]));
        let conn = state.connections.iter().find(|c| c.name == "mia").unwrap();
        assert!(conn.status.as_ref().unwrap().health_warning.is_none());
    }

    // ── Confirmation dialog tests ─────────────────────────────────────────────

    use crate::action::ConfirmAction;

    #[test]
    fn request_confirm_sets_pending() {
        let mut state = two_connection_state();
        assert!(state.pending_confirm.is_none());
        state.dispatch(&Action::RequestConfirm {
            message: "Disconnect all?".into(),
            action: ConfirmAction::DisconnectAll,
        });
        let pending = state.pending_confirm.as_ref().expect("pending must be set");
        assert_eq!(pending.message, "Disconnect all?");
        assert_eq!(pending.action, ConfirmAction::DisconnectAll);
    }

    #[test]
    fn confirm_yes_clears_pending() {
        let mut state = two_connection_state();
        state.dispatch(&Action::RequestConfirm {
            message: "Stop daemon?".into(),
            action: ConfirmAction::StopDaemon,
        });
        assert!(state.pending_confirm.is_some());
        state.dispatch(&Action::ConfirmYes);
        assert!(state.pending_confirm.is_none());
    }

    #[test]
    fn confirm_no_clears_pending() {
        let mut state = two_connection_state();
        state.dispatch(&Action::RequestConfirm {
            message: "Disconnect all?".into(),
            action: ConfirmAction::DisconnectAll,
        });
        assert!(state.pending_confirm.is_some());
        state.dispatch(&Action::ConfirmNo);
        assert!(state.pending_confirm.is_none());
    }

    #[test]
    fn confirm_no_pending_is_noop() {
        let mut state = two_connection_state();
        // Dispatching ConfirmYes/No with no pending is a no-op (must not panic).
        state.dispatch(&Action::ConfirmYes);
        state.dispatch(&Action::ConfirmNo);
        assert!(state.pending_confirm.is_none());
    }

    // ── wg-quick import tests ─────────────────────────────────────────────────

    #[test]
    fn enter_import_sets_mode() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterImport);
        assert_eq!(state.input_mode, InputMode::Import(String::new()));
    }

    #[test]
    fn import_key_appends_char() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterImport);
        let key = crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char('a'));
        state.dispatch(&Action::ImportKey(key));
        assert_eq!(state.import_buffer(), Some("a"));
    }

    #[test]
    fn import_key_backspace_removes_last_char() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterImport);
        let a = crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char('a'));
        let b = crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char('b'));
        let bs = crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Backspace);
        state.dispatch(&Action::ImportKey(a));
        state.dispatch(&Action::ImportKey(b));
        state.dispatch(&Action::ImportKey(bs));
        assert_eq!(state.import_buffer(), Some("a"));
    }

    #[test]
    fn submit_import_returns_to_normal() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterImport);
        state.dispatch(&Action::SubmitImport);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn exit_import_returns_to_normal() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterImport);
        state.dispatch(&Action::ExitImport);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn import_buffer_none_when_normal() {
        let state = two_connection_state();
        assert!(state.import_buffer().is_none());
    }

    #[test]
    fn reload_from_config_replaces_connections() {
        let mut state = two_connection_state();
        // Reload with a single-connection config.
        let new_config = make_app_config(&[("new-conn", vec![make_peer("p1")])]);
        state.reload_from_config(new_config);
        assert_eq!(state.connections.len(), 1);
        assert_eq!(state.connections[0].name, "new-conn");
    }

    #[test]
    fn reload_from_config_clamps_selection() {
        let mut state = two_connection_state();
        state.selected_connection = 1; // points at second connection
                                       // Reload with only one connection — selection must clamp to 0.
        let new_config = make_app_config(&[("only", vec![])]);
        state.reload_from_config(new_config);
        assert_eq!(state.selected_connection, 0);
    }

    #[test]
    fn second_request_confirm_replaces_pending() {
        let mut state = two_connection_state();
        state.dispatch(&Action::RequestConfirm {
            message: "First?".into(),
            action: ConfirmAction::DisconnectAll,
        });
        state.dispatch(&Action::RequestConfirm {
            message: "Second?".into(),
            action: ConfirmAction::StopDaemon,
        });
        let pending = state.pending_confirm.as_ref().unwrap();
        assert_eq!(pending.message, "Second?");
        assert_eq!(pending.action, ConfirmAction::StopDaemon);
    }
}
