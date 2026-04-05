//! Central action enum for unidirectional TUI state changes.
//!
//! Components emit [`Action`] variants from `handle_key()`.
//! [`AppState`](crate::state::AppState) processes them in `dispatch()`,
//! then components receive the action again via `update()`.

use crossterm::event::KeyEvent;
use ferro_wg_core::ipc::PeerStatus;

use crate::app::Tab;

/// An action that requires user confirmation before executing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    /// Tear down all connections.
    DisconnectAll,
    /// Stop the daemon process.
    StopDaemon,
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
}
