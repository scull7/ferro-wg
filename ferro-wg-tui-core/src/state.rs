//! Centralized TUI application state.
//!
//! [`AppState`] owns all shared data (connections, logs, theme) and
//! processes [`Action`]s via [`dispatch()`](AppState::dispatch). Components
//! receive `&AppState` for read-only access during rendering and key
//! handling.

use std::collections::{HashSet, VecDeque};
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
    BENCHMARK_HISTORY_CAP, BENCHMARK_PROGRESS_HISTORY_CAP, BenchmarkResultMap, BenchmarkRun,
    cap_history,
};
use crate::config_edit::{
    ConfigDiffPending, ConfigEditState, ConfigSection, EditableField, apply_field, config_diff,
    field_current_value, fields_for_section, validate_field,
};
use crate::theme::{Theme, ThemeKind};

/// How long toast messages are displayed before expiring.
const TOAST_DURATION: Duration = Duration::from_secs(3);

/// Maximum number of visible toasts.
pub const MAX_VISIBLE_TOASTS: usize = 5;

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

/// A transient toast message shown in the bottom-right corner.
#[derive(Debug, Clone)]
pub struct Toast {
    /// The message text.
    pub message: String,
    /// Whether this is an error (`true`) or success (`false`).
    pub is_error: bool,
    /// When this toast expires and should be hidden.
    pub expires_at: Instant,
}

impl Toast {
    /// Create a success toast message.
    #[must_use]
    pub fn success(message: String) -> Self {
        Self {
            message,
            is_error: false,
            expires_at: Instant::now() + TOAST_DURATION,
        }
    }

    /// Create an error toast message.
    #[must_use]
    pub fn error(message: String) -> Self {
        Self {
            message,
            is_error: true,
            expires_at: Instant::now() + TOAST_DURATION,
        }
    }

    /// Whether this toast has expired.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

/// Centralized application state.
///
/// All shared data lives here. Components never own or duplicate this
/// data — they receive `&AppState` for read-only access during rendering.
#[allow(clippy::struct_excessive_bools)]
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
    /// Current theme kind (Mocha or Latte).
    pub theme_kind: ThemeKind,
    /// Active color theme.
    pub theme: Theme,
    /// Whether the help overlay is shown.
    pub show_help: bool,
    /// Whether the daemon is currently reachable.
    pub daemon_connected: bool,
    /// Transient toast messages (success or error) with expiry.
    pub toasts: VecDeque<Toast>,
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

        let theme_kind = ThemeKind::default();
        Self {
            running: true,
            active_tab: Tab::Overview,
            input_mode: InputMode::Normal,
            search_query: String::new(),
            connections,
            selected_connection: 0,
            log_entries: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
            theme_kind,
            theme: theme_kind.into_theme(),
            show_help: false,
            daemon_connected: false,
            toasts: VecDeque::new(),
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
    pub fn dispatch(&mut self, action: &Action) {
        match action {
            Action::Quit => self.running = false,
            Action::NextTab | Action::PrevTab | Action::SelectTab(_) => {
                self.handle_tab_actions(action);
            }
            Action::EnterSearch
            | Action::ExitSearch
            | Action::ClearSearch
            | Action::SearchInput(_)
            | Action::SearchBackspace => self.handle_search_actions(action),
            Action::SelectNextConnection
            | Action::SelectPrevConnection
            | Action::SelectConnection(_) => self.handle_connection_actions(action),
            Action::UpdatePeers(_) => self.handle_peer_actions(action),
            Action::DaemonConnectivityChanged(_) => self.handle_daemon_actions(action),
            Action::DaemonOk(_) | Action::DaemonError(_) => self.handle_feedback_actions(action),
            Action::NextRow | Action::PrevRow => self.handle_row_actions(action),
            Action::EnterImport | Action::ImportKey(_) => self.handle_import_actions(action),
            Action::StartBenchmark
            | Action::StartBenchmarkForBackend(_)
            | Action::BenchmarkProgressUpdate(_)
            | Action::BenchmarkComplete(_) => self.handle_benchmark_actions(action),
            Action::ToggleCompareView => self.handle_compare_actions(action),
            Action::ToggleTheme => self.handle_theme_action(action),
            Action::ShowHelp | Action::HideHelp => self.handle_help_action(action),
            Action::EnterExport
            | Action::ExportKey(_)
            | Action::SubmitImport
            | Action::ExitImport
            | Action::SubmitExport
            | Action::ExitExport => self.handle_export_actions(action),
            Action::RequestConfirm { .. } | Action::ConfirmYes | Action::ConfirmNo => {
                self.handle_confirm_actions(action);
            }

            Action::EnterConfigEdit { .. }
            | Action::ConfigEditKey(_)
            | Action::ConfigFocusNext
            | Action::ConfigFocusPrev
            | Action::ConfigFocusInterface
            | Action::ConfigFocusPeer(_)
            | Action::AddConfigPeer
            | Action::DeleteConfigPeer(_)
            | Action::PreviewConfig
            | Action::ConfigDiffScrollDown
            | Action::ConfigDiffScrollUp
            | Action::SaveConfig { .. }
            | Action::DiscardConfigEdits => self.handle_config_actions(action),
            // These are handled by the event loop (maybe_spawn_command) or
            // components. They carry no state-machine side-effects here.
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

    /// Handle a key event when in [`InputMode::Export`].
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

    /// Handle tab-related actions.
    fn handle_tab_actions(&mut self, action: &Action) {
        if self.input_mode == InputMode::EditField {
            if let Some(edit) = self.config_edit.as_mut() {
                edit.edit_buffer = None;
                edit.field_error = None;
            }
            self.input_mode = InputMode::Normal;
        }
        match action {
            Action::NextTab => {
                self.active_tab = self.active_tab.next();
            }
            Action::PrevTab => {
                self.active_tab = self.active_tab.prev();
            }
            Action::SelectTab(tab) => {
                self.active_tab = *tab;
            }
            _ => {}
        }
    }

    /// Handle search-related actions.
    fn handle_search_actions(&mut self, action: &Action) {
        match action {
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
            _ => {}
        }
    }

    /// Handle connection selection actions.
    fn handle_connection_actions(&mut self, action: &Action) {
        match action {
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
            _ => {}
        }
    }

    /// Handle peer-related actions.
    fn handle_peer_actions(&mut self, action: &Action) {
        if let Action::UpdatePeers(statuses) = action {
            self.apply_peer_updates(statuses);
        }
    }

    /// Handle daemon connectivity actions.
    fn handle_daemon_actions(&mut self, action: &Action) {
        if let Action::DaemonConnectivityChanged(connected) = action {
            self.daemon_connected = *connected;
        }
    }

    /// Handle feedback actions.
    fn handle_feedback_actions(&mut self, action: &Action) {
        match action {
            Action::DaemonOk(msg) => {
                self.push_toast(Toast::success(msg.clone()));
            }
            Action::DaemonError(msg) => {
                self.push_toast(Toast::error(msg.clone()));
            }
            _ => {}
        }
    }

    /// Handle row navigation actions.
    fn handle_row_actions(&mut self, action: &Action) {
        match action {
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
            _ => {}
        }
    }

    /// Handle import-related actions.
    fn handle_import_actions(&mut self, action: &Action) {
        match action {
            Action::EnterImport => {
                self.input_mode = InputMode::Import(String::new());
            }
            Action::ImportKey(key) => self.apply_import_key(key),
            _ => {}
        }
    }

    /// Handle benchmark-related actions.
    fn handle_benchmark_actions(&mut self, action: &Action) {
        match action {
            Action::StartBenchmark | Action::StartBenchmarkForBackend(_) => {
                if self.benchmark_running {
                    self.push_toast(Toast::error("benchmark already running".to_owned()));
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
                let timestamp_ms = i64::try_from(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis(),
                )
                .expect("timestamp fits in i64");
                let run = BenchmarkRun {
                    timestamp_ms,
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
            _ => {}
        }
    }

    /// Handle compare view actions.
    fn handle_compare_actions(&mut self, action: &Action) {
        if action == &Action::ToggleCompareView {
            self.compare_view = match self.compare_view {
                CompareView::Live => CompareView::Historical,
                CompareView::Historical => CompareView::Live,
            };
        }
    }

    /// Handle export-related actions.
    fn handle_export_actions(&mut self, action: &Action) {
        match action {
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
            _ => {}
        }
    }

    /// Handle confirmation dialog actions.
    fn handle_confirm_actions(&mut self, action: &Action) {
        match action {
            Action::RequestConfirm { message, action } => {
                self.pending_confirm = Some(ConfirmPending {
                    message: message.clone(),
                    action: action.clone(),
                });
            }
            Action::ConfirmYes => {
                if let Some(pending) = self.pending_confirm.take()
                    && let ConfirmAction::DeletePeer(i) = pending.action
                {
                    self.dispatch(&Action::DeleteConfigPeer(i));
                }
            }
            Action::ConfirmNo => {
                self.pending_confirm = None;
            }
            _ => {}
        }
    }

    /// Handle config editing actions.
    #[allow(clippy::too_many_lines)]
    fn handle_config_actions(&mut self, action: &Action) {
        match action {
            Action::EnterConfigEdit { section, field_idx } => {
                if let Some(conn) = self.active_connection() {
                    let fields = fields_for_section(*section, false);
                    let field: EditableField = fields
                        .get(*field_idx)
                        .copied()
                        .unwrap_or(EditableField::ListenPort);
                    let initial_value = field_current_value(field, *section, &conn.config);
                    self.config_edit = Some(ConfigEditState {
                        connection_name: conn.name.clone(),
                        draft: conn.config.clone(),
                        focused_section: *section,
                        focused_field_idx: *field_idx,
                        edit_buffer: Some(initial_value),
                        field_error: None,
                        new_peer_indices: HashSet::new(),
                        session_error: None,
                    });
                    self.input_mode = InputMode::EditField;
                }
            }
            Action::ConfigEditKey(key) => {
                if let Some(edit) = self.config_edit.as_mut() {
                    // Handle char/backspace while buffer is active.
                    if let Some(ref mut buf) = edit.edit_buffer {
                        match key.code {
                            KeyCode::Char(c) => buf.push(c),
                            KeyCode::Backspace => {
                                buf.pop();
                            }
                            _ => {}
                        }
                    }
                    // Handle Enter/Esc after releasing the buf borrow so we
                    // can access edit.draft for validation.
                    match key.code {
                        KeyCode::Enter => {
                            if let Some(ref buf) = edit.edit_buffer {
                                let value = buf.clone();
                                let is_new = matches!(edit.focused_section, ConfigSection::Peer(i) if edit.new_peer_indices.contains(&i));
                                let fields = fields_for_section(edit.focused_section, is_new);
                                let field = fields
                                    .get(edit.focused_field_idx)
                                    .copied()
                                    .unwrap_or(EditableField::ListenPort);
                                let section = edit.focused_section;
                                match validate_field(field, &value, &edit.draft, section) {
                                    Ok(()) => {
                                        if field == EditableField::PeerPublicKey {
                                            // Write PeerPublicKey back to draft before clearing
                                            // the buffer. Also removes from new_peer_indices —
                                            // this is the only field that tracks the "new peer"
                                            // lifecycle.
                                            if let ConfigSection::Peer(idx) = edit.focused_section {
                                                if let Ok(key) = PublicKey::from_base64(&value)
                                                    && let Some(peer) =
                                                        edit.draft.peers.get_mut(idx)
                                                {
                                                    peer.public_key = key;
                                                }
                                                edit.new_peer_indices.remove(&idx);
                                                if edit.new_peer_indices.is_empty() {
                                                    edit.session_error = None;
                                                }
                                            }
                                        } else {
                                            // Write all other fields back to draft.
                                            apply_field(field, &value, &mut edit.draft, section);
                                        }
                                        edit.edit_buffer = None;
                                        edit.field_error = None;
                                        self.input_mode = InputMode::Normal;
                                    }
                                    Err(e) => {
                                        edit.field_error = Some(e.to_string());
                                    }
                                }
                            }
                        }
                        KeyCode::Esc => {
                            if edit.edit_buffer.is_some() {
                                edit.edit_buffer = None;
                                edit.field_error = None;
                                self.input_mode = InputMode::Normal;
                            }
                        }
                        _ => {}
                    }
                }
            }

            Action::ConfigFocusNext => {
                if let Some(edit) = self.config_edit.as_mut() {
                    let is_new = matches!(edit.focused_section, ConfigSection::Peer(i) if edit.new_peer_indices.contains(&i));
                    let fields = fields_for_section(edit.focused_section, is_new);
                    edit.focused_field_idx = (edit.focused_field_idx + 1) % fields.len();
                }
            }
            Action::ConfigFocusPrev => {
                if let Some(edit) = self.config_edit.as_mut() {
                    let is_new = matches!(edit.focused_section, ConfigSection::Peer(i) if edit.new_peer_indices.contains(&i));
                    let fields = fields_for_section(edit.focused_section, is_new);
                    edit.focused_field_idx = if edit.focused_field_idx == 0 {
                        fields.len().saturating_sub(1)
                    } else {
                        edit.focused_field_idx - 1
                    };
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
                    let new_peer = PeerConfig {
                        name: String::new(),
                        // Placeholder bytes — tracked by `new_peer_indices` until
                        // the user confirms a real key via `PeerPublicKey` Enter.
                        public_key: PublicKey::from_bytes([0u8; 32]),
                        endpoint: None,
                        allowed_ips: vec![],
                        persistent_keepalive: 0,
                        preshared_key: None,
                    };
                    edit.draft.peers.push(new_peer);
                    let idx = edit.draft.peers.len() - 1;
                    edit.new_peer_indices.insert(idx);
                    edit.focused_section = ConfigSection::Peer(idx);
                    edit.focused_field_idx = 0;
                    // Open the edit buffer immediately so the first field
                    // (PeerPublicKey for new peers) is ready to receive input.
                    edit.edit_buffer = Some(String::new());
                    edit.field_error = None;
                    self.input_mode = InputMode::EditField;
                }
            }
            Action::DeleteConfigPeer(i) => {
                if let Some(edit) = self.config_edit.as_mut()
                    && *i < edit.draft.peers.len()
                {
                    edit.draft.peers.remove(*i);
                    // Remove the deleted index and shift down all indices > i.
                    let shifted: HashSet<usize> = edit
                        .new_peer_indices
                        .iter()
                        .filter(|&&idx| idx != *i)
                        .map(|&idx| if idx > *i { idx - 1 } else { idx })
                        .collect();
                    edit.new_peer_indices = shifted;
                    // Reset focus: if no peers remain, move to interface;
                    // otherwise clamp focused_section to a valid peer index and
                    // clamp the field index within the surviving section's field count.
                    if edit.draft.peers.is_empty() {
                        edit.focused_section = ConfigSection::Interface;
                        edit.focused_field_idx = 0;
                    } else {
                        if let ConfigSection::Peer(j) = edit.focused_section {
                            let clamped = j.min(edit.draft.peers.len() - 1);
                            edit.focused_section = ConfigSection::Peer(clamped);
                        }
                        let is_new = matches!(edit.focused_section, ConfigSection::Peer(j) if edit.new_peer_indices.contains(&j));
                        let max_field = fields_for_section(edit.focused_section, is_new)
                            .len()
                            .saturating_sub(1);
                        edit.focused_field_idx = edit.focused_field_idx.min(max_field);
                    }
                }
            }
            Action::PreviewConfig => {
                // Block preview if any newly added peer still has an unconfirmed public key.
                if let Some(edit) = self.config_edit.as_mut()
                    && !edit.new_peer_indices.is_empty()
                {
                    edit.session_error =
                        Some("All new peers must have a public key before saving".to_string());
                    return;
                }
                if let Some(edit) = self.config_edit.as_ref()
                    && edit.field_error.is_none()
                {
                    let original_toml = self
                        .connections
                        .iter()
                        .find(|c| c.name == edit.connection_name)
                        .and_then(|c| ferro_wg_core::config::toml::save_to_string(&c.config).ok())
                        .unwrap_or_default();
                    let draft_toml = ferro_wg_core::config::toml::save_to_string(&edit.draft)
                        .unwrap_or_default();
                    let diff_lines = config_diff(&original_toml, &draft_toml);
                    self.config_diff_pending = Some(ConfigDiffPending {
                        connection_name: edit.connection_name.clone(),
                        draft: edit.draft.clone(),
                        diff_lines,
                        scroll_offset: 0,
                    });
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
            _ => {}
        }
    }

    /// Handle theme-related actions.
    fn handle_theme_action(&mut self, action: &Action) {
        if action == &Action::ToggleTheme {
            self.theme_kind = self.theme_kind.toggle();
            self.theme = self.theme_kind.into_theme();
        }
    }

    /// Handle help-related actions.
    fn handle_help_action(&mut self, action: &Action) {
        match action {
            Action::ShowHelp => self.show_help = true,
            Action::HideHelp => self.show_help = false,
            _ => {}
        }
    }

    /// Push a new toast, evicting the oldest if at capacity.
    pub fn push_toast(&mut self, toast: Toast) {
        self.toasts.push_back(toast);
        if self.toasts.len() > MAX_VISIBLE_TOASTS {
            self.toasts.pop_front();
        }
    }

    /// Clear expired toasts from the front. Called on each tick.
    pub fn clear_expired_toasts(&mut self) {
        while let Some(t) = self.toasts.front() {
            if t.is_expired() {
                self.toasts.pop_front();
            } else {
                break;
            }
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
    use ratatui::style::Color;
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
        assert_eq!(state.toasts.len(), 1);
        let toast = state.toasts.back().unwrap();
        assert!(!toast.is_error);
        assert_eq!(toast.message, "tunnel up");
        assert!(!toast.is_expired());

        state.dispatch(&Action::DaemonError("not found".into()));
        assert_eq!(state.toasts.len(), 2);
        let toast = state.toasts.back().unwrap();
        assert!(toast.is_error);
        assert_eq!(toast.message, "not found");
    }

    #[test]
    fn clear_expired_toasts_removes_old() {
        let mut state = two_connection_state();
        state.toasts.push_back(Toast {
            message: "old".into(),
            is_error: false,
            expires_at: Instant::now().checked_sub(Duration::from_secs(1)).unwrap(),
        });
        state.clear_expired_toasts();
        assert!(state.toasts.is_empty());
    }

    #[test]
    fn clear_expired_toasts_keeps_fresh() {
        let mut state = two_connection_state();
        state.dispatch(&Action::DaemonOk("fresh".into()));
        state.clear_expired_toasts();
        assert!(!state.toasts.is_empty());
    }

    #[test]
    fn toast_success_constructor() {
        let toast = Toast::success("msg".into());
        assert_eq!(toast.message, "msg");
        assert!(!toast.is_error);
        assert!(!toast.is_expired());
    }

    #[test]
    fn toast_error_constructor() {
        let toast = Toast::error("err".into());
        assert_eq!(toast.message, "err");
        assert!(toast.is_error);
        assert!(!toast.is_expired());
    }

    #[test]
    fn push_toast_evicts_oldest_when_full() {
        let mut state = two_connection_state();
        for i in 0..6 {
            state.push_toast(Toast::success(format!("msg{i}")));
        }
        assert_eq!(state.toasts.len(), 5);
        assert_eq!(state.toasts.front().unwrap().message, "msg1");
        assert_eq!(state.toasts.back().unwrap().message, "msg5");
    }

    #[test]
    fn clear_expired_toasts_removes_only_front_expired() {
        let mut state = two_connection_state();
        let expired = Toast {
            message: "expired".into(),
            is_error: false,
            expires_at: Instant::now().checked_sub(Duration::from_secs(1)).unwrap(),
        };
        let fresh = Toast::success("fresh".into());
        state.toasts.push_back(expired.clone());
        state.toasts.push_back(fresh.clone());
        state.toasts.push_back(expired.clone());
        state.clear_expired_toasts();
        assert_eq!(state.toasts.len(), 2);
        assert_eq!(state.toasts.front().unwrap().message, "fresh");
    }

    #[test]
    fn dispatch_daemon_ok_pushes_success_toast() {
        let mut state = two_connection_state();
        state.dispatch(&Action::DaemonOk("msg".into()));
        assert_eq!(state.toasts.back().unwrap().message, "msg");
        assert!(!state.toasts.back().unwrap().is_error);
    }

    #[test]
    fn dispatch_daemon_error_pushes_error_toast() {
        let mut state = two_connection_state();
        state.dispatch(&Action::DaemonError("err".into()));
        assert_eq!(state.toasts.back().unwrap().message, "err");
        assert!(state.toasts.back().unwrap().is_error);
    }

    #[test]
    fn dispatch_twice_adds_two_toasts() {
        let mut state = two_connection_state();
        state.dispatch(&Action::DaemonOk("first".into()));
        state.dispatch(&Action::DaemonOk("second".into()));
        assert_eq!(state.toasts.len(), 2);
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
    fn dispatch_toggle_theme_from_mocha_to_latte() {
        let mut state = two_connection_state();
        assert_eq!(state.theme_kind, ThemeKind::Mocha);
        state.dispatch(&Action::ToggleTheme);
        assert_eq!(state.theme_kind, ThemeKind::Latte);
        assert_eq!(state.theme.accent, Color::Rgb(114, 135, 253));
    }

    #[test]
    fn dispatch_toggle_theme_twice_back_to_mocha() {
        let mut state = two_connection_state();
        state.dispatch(&Action::ToggleTheme);
        state.dispatch(&Action::ToggleTheme);
        assert_eq!(state.theme_kind, ThemeKind::Mocha);
        assert_eq!(state.theme.accent, Color::Rgb(180, 190, 254));
    }

    #[test]
    fn dispatch_show_help() {
        let mut state = two_connection_state();
        assert!(!state.show_help);
        state.dispatch(&Action::ShowHelp);
        assert!(state.show_help);
    }

    #[test]
    fn dispatch_hide_help() {
        let mut state = two_connection_state();
        state.show_help = true;
        state.dispatch(&Action::HideHelp);
        assert!(!state.show_help);
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

    // ── Phase 6: Config editing dispatch tests ────────────────────────────────

    #[test]
    fn enter_config_edit_creates_config_edit_state() {
        let mut state = two_connection_state();
        assert!(state.config_edit.is_none());
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        let edit = state.config_edit.as_ref().expect("config_edit must be set");
        assert_eq!(edit.connection_name, "mia");
        assert_eq!(edit.focused_section, ConfigSection::Interface);
        assert_eq!(edit.focused_field_idx, 0);
        assert!(edit.edit_buffer.is_some());
        assert_eq!(state.input_mode, InputMode::EditField);
    }

    #[test]
    fn enter_config_edit_no_connection_is_noop() {
        let mut state = AppState::new(AppConfig::default());
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        assert!(state.config_edit.is_none());
    }

    #[test]
    fn config_edit_key_char_appends_to_buffer() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        let key = crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char('9'));
        state.dispatch(&Action::ConfigEditKey(key));
        let buf = state
            .config_edit
            .as_ref()
            .unwrap()
            .edit_buffer
            .as_ref()
            .unwrap();
        assert!(buf.ends_with('9'));
    }

    #[test]
    fn config_edit_key_backspace_removes_last_char() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        // Ensure buffer has content first
        let key_x = crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char('x'));
        state.dispatch(&Action::ConfigEditKey(key_x));
        let before_len = state
            .config_edit
            .as_ref()
            .unwrap()
            .edit_buffer
            .as_ref()
            .unwrap()
            .len();
        let bs = crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Backspace);
        state.dispatch(&Action::ConfigEditKey(bs));
        let after_len = state
            .config_edit
            .as_ref()
            .unwrap()
            .edit_buffer
            .as_ref()
            .unwrap()
            .len();
        assert_eq!(after_len, before_len - 1);
    }

    #[test]
    fn config_edit_key_enter_clears_buffer_and_reverts_mode() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        assert_eq!(state.input_mode, InputMode::EditField);
        let enter = crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Enter);
        state.dispatch(&Action::ConfigEditKey(enter));
        assert!(state.config_edit.as_ref().unwrap().edit_buffer.is_none());
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn config_edit_key_esc_cancels_buffer() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        let esc = crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Esc);
        state.dispatch(&Action::ConfigEditKey(esc));
        assert!(state.config_edit.as_ref().unwrap().edit_buffer.is_none());
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn add_config_peer_pushes_peer_and_sets_edit_mode() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        let initial_len = state
            .config_edit
            .as_ref()
            .expect("config_edit must be set")
            .draft
            .peers
            .len();
        state.dispatch(&Action::AddConfigPeer);
        let edit = state
            .config_edit
            .as_ref()
            .expect("config_edit must still be set after AddConfigPeer");
        assert_eq!(edit.draft.peers.len(), initial_len + 1);
        assert_eq!(edit.focused_section, ConfigSection::Peer(initial_len));
        assert!(
            edit.new_peer_indices.contains(&initial_len),
            "new peer index must be tracked in new_peer_indices"
        );
        assert_eq!(state.input_mode, InputMode::EditField);
    }

    #[test]
    fn add_config_peer_without_active_edit_is_a_no_op() {
        // Arrange: no config_edit session started
        let mut state = two_connection_state();
        assert!(
            state.config_edit.is_none(),
            "config_edit must be None before the action"
        );

        // Act
        state.dispatch(&Action::AddConfigPeer);

        // Assert: state is unchanged
        assert!(
            state.config_edit.is_none(),
            "AddConfigPeer without an active edit session must be a no-op"
        );
    }

    #[test]
    fn confirm_public_key_removes_from_new_peer_indices() {
        // Arrange: open edit, add a new peer (which lands in new_peer_indices)
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        state.dispatch(&Action::AddConfigPeer);
        let peer_idx = state
            .config_edit
            .as_ref()
            .expect("config_edit must be set")
            .draft
            .peers
            .len()
            - 1;
        assert!(
            state
                .config_edit
                .as_ref()
                .expect("config_edit must be set")
                .new_peer_indices
                .contains(&peer_idx),
            "peer must be in new_peer_indices before key confirmation"
        );

        // Act: type a valid 44-char base64 key and press Enter
        let valid_key = "/yt5f1nclaUwO75kn6KosqO2ZD6kJ4Ld4SrYuG1csZg=";
        for ch in valid_key.chars() {
            let key_event = crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char(ch));
            state.dispatch(&Action::ConfigEditKey(key_event));
        }
        let enter = crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Enter);
        state.dispatch(&Action::ConfigEditKey(enter));

        // Assert: index removed from new_peer_indices
        let edit = state
            .config_edit
            .as_ref()
            .expect("config_edit must still be present after key confirmation");
        assert!(
            !edit.new_peer_indices.contains(&peer_idx),
            "peer index must be removed from new_peer_indices after public key is confirmed"
        );
    }

    #[test]
    fn delete_new_peer_removes_from_new_peer_indices() {
        // Arrange: open edit, add a new peer
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        state.dispatch(&Action::AddConfigPeer);
        let peer_idx = state
            .config_edit
            .as_ref()
            .expect("config_edit must be set")
            .draft
            .peers
            .len()
            - 1;

        // Act: delete the newly added peer
        state.dispatch(&Action::DeleteConfigPeer(peer_idx));

        // Assert: new_peer_indices is now empty
        let edit = state
            .config_edit
            .as_ref()
            .expect("config_edit must still be present after delete");
        assert!(
            edit.new_peer_indices.is_empty(),
            "new_peer_indices must be empty after deleting the new peer"
        );
    }

    #[test]
    fn preview_config_blocked_when_new_peer_key_unconfirmed() {
        // Arrange: open edit, add a new peer (key not yet confirmed)
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        state.dispatch(&Action::AddConfigPeer);
        assert!(
            state.config_diff_pending.is_none(),
            "no pending diff before PreviewConfig"
        );

        // Act: attempt to preview before confirming the public key
        state.dispatch(&Action::PreviewConfig);

        // Assert: preview is blocked and a session_error is set
        assert!(
            state.config_diff_pending.is_none(),
            "PreviewConfig must be blocked when new peer has no confirmed public key"
        );
        let error = state
            .config_edit
            .as_ref()
            .expect("config_edit must still be present")
            .session_error
            .as_deref()
            .expect("session_error must be set when preview is blocked");
        assert!(
            error.contains("public key"),
            "session_error must mention the public key requirement, got: {error}"
        );
    }

    #[test]
    fn delete_config_peer_removes_peer() {
        let mut state = AppState::new(make_app_config(&[(
            "mia",
            vec![make_peer("p1"), make_peer("p2")],
        )]));
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        assert_eq!(state.config_edit.as_ref().unwrap().draft.peers.len(), 2);
        state.dispatch(&Action::DeleteConfigPeer(0));
        assert_eq!(state.config_edit.as_ref().unwrap().draft.peers.len(), 1);
    }

    #[test]
    fn confirm_yes_with_delete_peer_removes_peer() {
        let mut state = AppState::new(make_app_config(&[(
            "mia",
            vec![make_peer("p1"), make_peer("p2")],
        )]));
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        state.dispatch(&Action::RequestConfirm {
            message: "Delete peer 0?".into(),
            action: ConfirmAction::DeletePeer(0),
        });
        state.dispatch(&Action::ConfirmYes);
        assert!(state.pending_confirm.is_none());
        assert_eq!(state.config_edit.as_ref().unwrap().draft.peers.len(), 1);
    }

    #[test]
    fn preview_config_sets_diff_pending() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        assert!(state.config_diff_pending.is_none());
        state.dispatch(&Action::PreviewConfig);
        assert!(state.config_diff_pending.is_some());
    }

    #[test]
    fn preview_config_blocked_when_field_error() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        state.config_edit.as_mut().unwrap().field_error = Some("bad value".into());
        state.dispatch(&Action::PreviewConfig);
        assert!(state.config_diff_pending.is_none());
    }

    #[test]
    fn config_diff_scroll_down_increments_offset() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        state.dispatch(&Action::PreviewConfig);
        state.dispatch(&Action::ConfigDiffScrollDown);
        assert_eq!(state.config_diff_pending.as_ref().unwrap().scroll_offset, 1);
    }

    #[test]
    fn config_diff_scroll_up_saturates_at_zero() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        state.dispatch(&Action::PreviewConfig);
        state.dispatch(&Action::ConfigDiffScrollUp);
        assert_eq!(state.config_diff_pending.as_ref().unwrap().scroll_offset, 0);
    }

    #[test]
    fn save_config_clears_both_edit_and_diff_pending() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        state.dispatch(&Action::PreviewConfig);
        assert!(state.config_edit.is_some());
        assert!(state.config_diff_pending.is_some());
        state.dispatch(&Action::SaveConfig { reconnect: false });
        assert!(state.config_edit.is_none());
        assert!(state.config_diff_pending.is_none());
    }

    #[test]
    fn discard_config_edits_clears_state_and_resets_mode() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        assert_eq!(state.input_mode, InputMode::EditField);
        state.dispatch(&Action::DiscardConfigEdits);
        assert!(state.config_edit.is_none());
        assert!(state.config_diff_pending.is_none());
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn next_tab_while_editing_clears_edit_buffer() {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        assert_eq!(state.input_mode, InputMode::EditField);
        state.dispatch(&Action::NextTab);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(state.config_edit.as_ref().unwrap().edit_buffer.is_none());
    }

    // ── Phase 2 review remediation tests ─────────────────────────────────────

    const VALID_KEY: &str = "/yt5f1nclaUwO75kn6KosqO2ZD6kJ4Ld4SrYuG1csZg=";

    /// Helper: open config edit on `two_connection_state`, add N new peers, and
    /// return the state plus the indices of those new peers.
    fn open_edit_and_add_peers(n: usize) -> (AppState, Vec<usize>) {
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        let mut indices = Vec::with_capacity(n);
        for _ in 0..n {
            state.dispatch(&Action::AddConfigPeer);
            let idx = state
                .config_edit
                .as_ref()
                .expect("config_edit must be set")
                .draft
                .peers
                .len()
                - 1;
            indices.push(idx);
        }
        (state, indices)
    }

    /// Helper: type a string into the current edit buffer and press Enter.
    fn type_and_enter(state: &mut AppState, s: &str) {
        for ch in s.chars() {
            state.dispatch(&Action::ConfigEditKey(crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Char(ch),
            )));
        }
        state.dispatch(&Action::ConfigEditKey(crossterm::event::KeyEvent::from(
            crossterm::event::KeyCode::Enter,
        )));
    }

    // Test A
    #[test]
    fn two_new_peers_second_still_unconfirmed_after_first_confirmed() {
        // Arrange: use a connection with NO existing peers so new peers land at
        // indices 0 and 1 with no offset confusion.
        let mut state = AppState::new(make_app_config(&[("mia", vec![])]));
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        // Add two new peers: after each AddConfigPeer the edit buffer is open on
        // that peer's PeerPublicKey field.
        state.dispatch(&Action::AddConfigPeer); // peer 0 — buffer open on it
        state.dispatch(&Action::AddConfigPeer); // peer 1 — focus + buffer moves here

        // Confirm the key for the SECOND new peer (peer 1) only.
        // The buffer is already open on peer 1 from the second AddConfigPeer.
        type_and_enter(&mut state, VALID_KEY);

        // Act: attempt preview — peer 0 still has no confirmed key.
        state.dispatch(&Action::PreviewConfig);

        // Assert: peer 1 was confirmed (removed from set); peer 0 remains; preview blocked.
        let edit = state
            .config_edit
            .as_ref()
            .expect("config_edit must be present");
        assert!(
            !edit.new_peer_indices.contains(&1),
            "peer 1 must be removed from new_peer_indices after key confirmation"
        );
        assert!(
            edit.new_peer_indices.contains(&0),
            "peer 0 must still be in new_peer_indices (key not confirmed)"
        );
        assert!(
            state.config_diff_pending.is_none(),
            "PreviewConfig must be blocked while peer 0 has no confirmed key"
        );
        assert!(
            state.config_edit.as_ref().unwrap().session_error.is_some(),
            "session_error must be set while unconfirmed peers remain"
        );
    }

    // Test B
    #[test]
    fn config_focus_next_wraps_at_5_fields_for_new_peer() {
        // Arrange: open edit, add a new peer (5 fields: PeerPublicKey first).
        let (mut state, indices) = open_edit_and_add_peers(1);
        let peer_idx = indices[0];

        // Verify we are focused on the new peer.
        let edit = state.config_edit.as_ref().expect("config_edit must be set");
        assert_eq!(edit.focused_section, ConfigSection::Peer(peer_idx));
        assert_eq!(edit.focused_field_idx, 0);

        // Act: ConfigFocusNext × 5 (one full cycle through 5 fields).
        for _ in 0..5 {
            state.dispatch(&Action::ConfigFocusNext);
        }

        // Assert: wrapped back to 0.
        let edit = state.config_edit.as_ref().expect("config_edit must be set");
        assert_eq!(
            edit.focused_field_idx, 0,
            "focused_field_idx must wrap back to 0 after 5 increments on a 5-field new peer"
        );
    }

    // Test C
    #[test]
    fn config_focus_prev_wraps_at_4_fields_for_existing_peer() {
        // Arrange: open edit on the existing peer at index 0 (4 fields).
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Peer(0),
            field_idx: 0,
        });
        let edit = state.config_edit.as_ref().expect("config_edit must be set");
        assert_eq!(edit.focused_field_idx, 0);

        // Act: ConfigFocusPrev from index 0 — must wrap to last field (index 3).
        state.dispatch(&Action::ConfigFocusPrev);

        // Assert: wrapped to 3 (4 fields, 0-indexed).
        let edit = state.config_edit.as_ref().expect("config_edit must be set");
        assert_eq!(
            edit.focused_field_idx, 3,
            "ConfigFocusPrev at idx 0 on a 4-field existing peer must wrap to 3"
        );
    }

    // Test D
    #[test]
    fn delete_first_of_two_new_peers_shifts_second_index_down() {
        // Arrange: open edit, add two new peers.
        let (mut state, indices) = open_edit_and_add_peers(2);
        let first_idx = indices[0];
        let second_idx = indices[1];
        assert_eq!(
            second_idx,
            first_idx + 1,
            "second new peer must be at first_idx + 1"
        );

        // Act: delete the first new peer.
        state.dispatch(&Action::DeleteConfigPeer(first_idx));

        // Assert: new_peer_indices contains the shifted index (second peer moved down).
        let edit = state
            .config_edit
            .as_ref()
            .expect("config_edit must be present after DeleteConfigPeer");
        assert!(
            edit.new_peer_indices.contains(&first_idx),
            "shifted second peer must now appear at index {first_idx}"
        );
        assert!(
            !edit.new_peer_indices.contains(&second_idx),
            "old unshifted index {second_idx} must not remain in new_peer_indices"
        );
    }

    // Test E
    #[test]
    fn preview_config_proceeds_after_new_peer_key_confirmed() {
        // Arrange: open edit, add a new peer, confirm its key.
        let (mut state, _indices) = open_edit_and_add_peers(1);
        // The edit buffer is open on the new peer's PeerPublicKey field.
        type_and_enter(&mut state, VALID_KEY);

        // Verify new_peer_indices is empty.
        let edit = state.config_edit.as_ref().expect("config_edit must be set");
        assert!(
            edit.new_peer_indices.is_empty(),
            "new_peer_indices must be empty after key confirmation"
        );
        assert!(
            edit.session_error.is_none(),
            "session_error must be None after all keys confirmed"
        );

        // Act: preview should now proceed.
        state.dispatch(&Action::PreviewConfig);

        // Assert.
        assert!(
            state.config_diff_pending.is_some(),
            "config_diff_pending must be Some after successful PreviewConfig"
        );
        assert!(
            state.config_edit.as_ref().unwrap().session_error.is_none(),
            "session_error must remain None after successful preview"
        );
    }

    // Test F
    #[test]
    fn confirm_public_key_writes_key_to_draft() {
        // Arrange: open edit, add a new peer.
        let (mut state, indices) = open_edit_and_add_peers(1);
        let peer_idx = indices[0];

        // Act: type a valid 44-char key and press Enter.
        type_and_enter(&mut state, VALID_KEY);

        // Assert: the key was written back to the draft.
        let edit = state
            .config_edit
            .as_ref()
            .expect("config_edit must be set after key confirmation");
        let actual_key = edit
            .draft
            .peers
            .get(peer_idx)
            .expect("peer must exist in draft")
            .public_key
            .to_base64();
        assert_eq!(
            actual_key, VALID_KEY,
            "public_key in draft must match the confirmed key"
        );
    }

    // ── Phase 3 review remediation: apply_field write-back tests ─────────────

    #[test]
    fn apply_field_listen_port_writes_to_draft() {
        // Arrange
        let mut state = two_connection_state();
        // ListenPort is field index 0 in the interface section.
        // Arrange: open a fresh edit session before each field edit.
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });

        // Clear the pre-populated value ("51820" from make_interface).
        let buf_len = state
            .config_edit
            .as_ref()
            .expect("config_edit must be set")
            .edit_buffer
            .as_ref()
            .expect("edit_buffer must be Some")
            .len();
        for _ in 0..buf_len {
            state.dispatch(&Action::ConfigEditKey(crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Backspace,
            )));
        }

        // Act: type a new port and press Enter.
        type_and_enter(&mut state, "51820");

        // Assert
        let port = state
            .config_edit
            .as_ref()
            .expect("config_edit must be present after commit")
            .draft
            .interface
            .listen_port;
        assert_eq!(port, 51820, "listen_port must be written back to draft");
    }

    #[test]
    fn apply_field_addresses_writes_to_draft() {
        // Arrange: use a fresh state with a known address list.
        let mut state = AppState::new(make_app_config(&[("mia", vec![make_peer("p1")])]));
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 1, // Addresses
        });

        // Clear pre-populated buffer ("10.0.0.2/24" from make_interface).
        let buf_len = state
            .config_edit
            .as_ref()
            .expect("config_edit must be set")
            .edit_buffer
            .as_ref()
            .expect("edit_buffer must be Some")
            .len();
        for _ in 0..buf_len {
            state.dispatch(&Action::ConfigEditKey(crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Backspace,
            )));
        }

        // Act
        type_and_enter(&mut state, "10.0.0.1/24");

        // Assert
        let addresses = &state
            .config_edit
            .as_ref()
            .expect("config_edit must be present")
            .draft
            .interface
            .addresses;
        assert_eq!(
            addresses,
            &["10.0.0.1/24"],
            "addresses must be written back to draft"
        );
    }

    #[test]
    fn apply_field_peer_name_writes_to_draft() {
        // Arrange: two_connection_state has "mia" with one peer named "mia-dc".
        // Existing peer fields (no PeerPublicKey): [PeerName=0, PeerEndpoint=1,
        // PeerAllowedIps=2, PeerPersistentKeepalive=3].
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        // Switch focus to existing peer 0, field 0 (PeerName).
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Peer(0),
            field_idx: 0,
        });

        // Clear pre-populated buffer ("mia-dc").
        let buf_len = state
            .config_edit
            .as_ref()
            .expect("config_edit must be set")
            .edit_buffer
            .as_ref()
            .expect("edit_buffer must be Some")
            .len();
        for _ in 0..buf_len {
            state.dispatch(&Action::ConfigEditKey(crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Backspace,
            )));
        }

        // Act
        type_and_enter(&mut state, "new-peer-name");

        // Assert
        let name = &state
            .config_edit
            .as_ref()
            .expect("config_edit must be present")
            .draft
            .peers[0]
            .name;
        assert_eq!(
            name, "new-peer-name",
            "peer name must be written back to draft"
        );
    }

    #[test]
    fn apply_field_peer_endpoint_writes_to_draft() {
        // Arrange: existing peer has endpoint "198.51.100.1:51820".
        // PeerEndpoint is field index 1 in the existing-peer field list.
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Peer(0),
            field_idx: 1, // PeerEndpoint
        });

        // Clear pre-populated buffer.
        let buf_len = state
            .config_edit
            .as_ref()
            .expect("config_edit must be set")
            .edit_buffer
            .as_ref()
            .expect("edit_buffer must be Some")
            .len();
        for _ in 0..buf_len {
            state.dispatch(&Action::ConfigEditKey(crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Backspace,
            )));
        }

        // Act
        type_and_enter(&mut state, "198.51.100.1:51820");

        // Assert
        let endpoint = state
            .config_edit
            .as_ref()
            .expect("config_edit must be present")
            .draft
            .peers[0]
            .endpoint
            .as_deref();
        assert_eq!(
            endpoint,
            Some("198.51.100.1:51820"),
            "peer endpoint must be written back to draft"
        );
    }

    #[test]
    fn apply_field_peer_endpoint_empty_clears_endpoint() {
        // Arrange: existing peer has endpoint "198.51.100.1:51820".
        // PeerEndpoint is field index 1 in the existing-peer field list.
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Peer(0),
            field_idx: 1, // PeerEndpoint
        });

        // Clear pre-populated buffer entirely (resulting in empty string).
        let buf_len = state
            .config_edit
            .as_ref()
            .expect("config_edit must be set")
            .edit_buffer
            .as_ref()
            .expect("edit_buffer must be Some")
            .len();
        for _ in 0..buf_len {
            state.dispatch(&Action::ConfigEditKey(crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Backspace,
            )));
        }

        // Act: commit an empty endpoint value.
        state.dispatch(&Action::ConfigEditKey(crossterm::event::KeyEvent::from(
            crossterm::event::KeyCode::Enter,
        )));

        // Assert
        let endpoint = state
            .config_edit
            .as_ref()
            .expect("config_edit must be present")
            .draft
            .peers[0]
            .endpoint
            .as_deref();
        assert_eq!(
            endpoint, None,
            "empty endpoint string must set peer.endpoint to None"
        );
    }

    // ── dispatch-path integration tests (Phase 3 review remediation) ─────────

    #[test]
    fn apply_field_peer_allowed_ips_writes_to_draft() {
        // Arrange: open config edit on "mia" (Peer 0 = "mia-dc"), focus
        // PeerAllowedIps (index 2 in PEER_FIELDS_EXISTING).
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Peer(0),
            field_idx: 2, // PeerAllowedIps
        });

        // Clear the pre-populated allowed-IPs buffer ("10.100.0.0/16").
        let buf_len = state
            .config_edit
            .as_ref()
            .expect("config_edit must be set")
            .edit_buffer
            .as_ref()
            .expect("edit_buffer must be Some")
            .len();
        for _ in 0..buf_len {
            state.dispatch(&Action::ConfigEditKey(crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Backspace,
            )));
        }

        // Act: type the new value and commit with Enter.
        type_and_enter(&mut state, "10.0.0.0/8");

        // Assert
        let allowed_ips = &state
            .config_edit
            .as_ref()
            .expect("config_edit must be present")
            .draft
            .peers[0]
            .allowed_ips;
        assert_eq!(
            allowed_ips,
            &["10.0.0.0/8"],
            "allowed_ips must be written back to the draft peer"
        );
    }

    #[test]
    fn apply_field_peer_persistent_keepalive_writes_to_draft() {
        // Arrange: open config edit on "mia" (Peer 0 = "mia-dc"), focus
        // PeerPersistentKeepalive (index 3 in PEER_FIELDS_EXISTING).
        let mut state = two_connection_state();
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Interface,
            field_idx: 0,
        });
        state.dispatch(&Action::EnterConfigEdit {
            section: ConfigSection::Peer(0),
            field_idx: 3, // PeerPersistentKeepalive
        });

        // Clear pre-populated buffer ("25" from make_peer).
        let buf_len = state
            .config_edit
            .as_ref()
            .expect("config_edit must be set")
            .edit_buffer
            .as_ref()
            .expect("edit_buffer must be Some")
            .len();
        for _ in 0..buf_len {
            state.dispatch(&Action::ConfigEditKey(crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Backspace,
            )));
        }

        // Act: type "25" and commit with Enter.
        type_and_enter(&mut state, "25");

        // Assert
        let keepalive = state
            .config_edit
            .as_ref()
            .expect("config_edit must be present")
            .draft
            .peers[0]
            .persistent_keepalive;
        assert_eq!(
            keepalive, 25,
            "persistent_keepalive must be written back to the draft peer"
        );
    }
}
