//! Central action enum for unidirectional TUI state changes.
//!
//! Components emit [`Action`] variants from `handle_key()`.
//! [`AppState`](crate::state::AppState) processes them in `dispatch()`,
//! then components receive the action again via `update()`.

use crossterm::event::KeyEvent;
use ferro_wg_core::ipc::{BenchmarkProgress, PeerStatus};
use ferro_wg_core::stats::BenchmarkResult;

use crate::app::Tab;

/// An action that requires user confirmation before executing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    /// Tear down all connections.
    DisconnectAll,
    /// Stop the daemon process.
    StopDaemon,
    /// Delete the peer at this index from the draft.
    DeletePeer(usize),
}

/// An action that can be dispatched through the TUI state machine.
///
/// All state changes flow through this enum — components never mutate
/// shared state directly.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Quit the application.
    Quit,
    /// Switch to the next tab (wrapping).
    NextTab,
    /// Switch to the previous tab (wrapping).
    PrevTab,
    /// Jump to a specific tab.
    SelectTab(Tab),
    /// Move the row selection down in the active table.
    NextRow,
    /// Move the row selection up in the active table.
    PrevRow,
    /// Enter search input mode.
    EnterSearch,
    /// Exit search input mode (keep query).
    ExitSearch,
    /// Exit search input mode and clear the query.
    ClearSearch,
    /// Append a character to the search query.
    SearchInput(char),
    /// Remove the last character from the search query.
    SearchBackspace,
    /// Periodic tick for background refresh.
    Tick,

    // -- Connection selection actions --
    /// Focus the next connection in the list (wraps).
    SelectNextConnection,
    /// Focus the previous connection in the list (wraps).
    SelectPrevConnection,
    /// Focus a specific connection by index.
    SelectConnection(usize),

    // -- Daemon integration actions --
    /// Update connection state from a daemon status response.
    UpdatePeers(Vec<PeerStatus>),
    /// Bring up the selected connection by name.
    ConnectPeer(String),
    /// Tear down the selected connection by name.
    DisconnectPeer(String),
    /// Cycle the backend for the selected connection.
    CyclePeerBackend(String),
    /// Daemon returned an error message.
    DaemonError(String),
    /// Daemon command succeeded with a description.
    DaemonOk(String),
    /// Daemon connectivity changed (true = reachable).
    DaemonConnectivityChanged(bool),

    // -- Bulk connection lifecycle actions --
    /// Bring all connections up (all-connections `Up`).
    ConnectAll,
    /// Tear down all connections (all-connections `Down`).
    DisconnectAll,

    // -- Daemon lifecycle actions --
    /// Start the daemon as a background subprocess.
    StartDaemon,
    /// Stop the running daemon.
    StopDaemon,

    // -- wg-quick import actions --
    /// Enter import path input mode.
    EnterImport,
    /// Forward a key event to the import path buffer in [`AppState`].
    ImportKey(KeyEvent),
    /// Submit the current import path for processing.
    SubmitImport,
    /// Cancel import and return to normal mode.
    ExitImport,

    // -- Benchmark actions --
    /// Start a benchmark for the active connection (all backends sequentially).
    ///
    /// Blocked when `AppState::benchmark_running` is `true`; emits
    /// `DaemonError("benchmark already running")` instead.
    StartBenchmark,

    /// Start a benchmark scoped to the named backend.
    ///
    /// Emitted when the user presses `Enter` on a specific backend row.
    StartBenchmarkForBackend(String),

    /// Forward a live progress update from the daemon to `AppState`.
    BenchmarkProgressUpdate(BenchmarkProgress),

    /// A benchmark run completed; store results and persist history.
    BenchmarkComplete(BenchmarkResult),

    /// Switch the active connection to the named backend.
    ///
    /// Emitted when the user presses `w` on a backend row.
    /// Delegates to `DaemonCommand::SwitchBackend`.
    SwitchBenchmarkBackend(String),

    /// Toggle `AppState::compare_view` between `Live` and `Historical`.
    ToggleCompareView,

    // -- Export actions --
    /// Enter export path input mode (opens the path prompt in the status bar).
    EnterExport,

    /// Forward a key event to the export path buffer.
    ExportKey(KeyEvent),

    /// Submit the current export path for processing.
    SubmitExport,

    /// Cancel export and return to `InputMode::Normal`.
    ExitExport,

    // -- Confirmation dialog actions --
    /// Show a confirmation dialog before executing a destructive action.
    RequestConfirm {
        /// The message shown in the confirmation overlay.
        message: String,
        /// The action to execute if the user confirms.
        action: ConfirmAction,
    },
    /// User confirmed the pending action.
    ConfirmYes,
    /// User cancelled the pending action.
    ConfirmNo,

    // -- Config editing --
    /// Enter edit mode for the focused field in the Config tab.
    /// Copies the current field value into `AppState::config_edit.edit_buffer`.
    EnterConfigEdit,

    /// Forward a key event to the active edit buffer.
    /// `AppState::dispatch` unpacks char/backspace; `Enter` → `CommitConfigEdit`;
    /// `Esc` → `CancelConfigEdit`.
    ConfigEditKey(KeyEvent),

    /// Commit the current buffer to the draft, run the field validator, and
    /// return to focused-but-not-editing state. Blocked if `field_error` is Some.
    CommitConfigEdit,

    /// Discard the current buffer and return to focused-but-not-editing state.
    CancelConfigEdit,

    /// Move field focus down within the current section (wraps).
    ConfigFocusNext,

    /// Move field focus up within the current section (wraps).
    ConfigFocusPrev,

    /// Move section focus to the Interface block.
    ConfigFocusInterface,

    /// Move section focus to peer at the given index.
    ConfigFocusPeer(usize),

    /// Append a new blank peer to the draft and enter EditField on its PublicKey.
    AddConfigPeer,

    /// Remove the peer at the given index from the draft (after confirmation).
    DeleteConfigPeer(usize),

    /// Request the diff preview: serialise the draft to TOML, diff against the
    /// original, and store the result in `AppState::config_diff_pending`.
    /// Blocked if any field has a pending `field_error` or `WgConfig::validate` fails.
    PreviewConfig,

    /// Scroll the diff preview overlay down by one line.
    ConfigDiffScrollDown,

    /// Scroll the diff preview overlay up by one line.
    ConfigDiffScrollUp,

    /// Save the pending draft to disk (backup first), then reload config state.
    /// Sent from within the diff preview; clears `config_diff_pending` on success.
    SaveConfig {
        /// When `true`, reconnect affected tunnels after saving.
        reconnect: bool,
    },

    /// Discard all pending edits and clear `AppState::config_edit`.
    DiscardConfigEdits,
}
