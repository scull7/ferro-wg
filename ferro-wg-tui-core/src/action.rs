//! Central action enum for unidirectional TUI state changes.
//!
//! Components emit [`Action`] variants from `handle_key()`.
//! [`AppState`](crate::state::AppState) processes them in `dispatch()`,
//! then components receive the action again via `update()`.

use crate::app::Tab;

/// An action that can be dispatched through the TUI state machine.
///
/// All state changes flow through this enum — components never mutate
/// shared state directly.
#[derive(Debug, Clone, PartialEq, Eq)]
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
}
