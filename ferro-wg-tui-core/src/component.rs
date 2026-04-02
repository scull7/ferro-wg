//! Component trait for TUI panels.
//!
//! Every tab view and chrome element (tab bar, status bar) implements
//! [`Component`]. The event loop routes key events to the active
//! component, dispatches the resulting [`Action`] through
//! [`AppState`](crate::state::AppState), then notifies all components
//! via `update()`.

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::action::Action;
use crate::state::AppState;

/// A self-contained TUI panel with its own state and rendering.
///
/// Components follow a unidirectional data flow:
///
/// 1. `handle_key()` — convert input into an [`Action`] (or `None`)
/// 2. `dispatch()` on [`AppState`] — mutate shared state
/// 3. `update()` — react to the dispatched action
/// 4. `render()` — draw the current state to the terminal
pub trait Component {
    /// Handle a key event, optionally producing an [`Action`].
    ///
    /// Return `Some(action)` to trigger a state change, or `None`
    /// if the key is not handled by this component.
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action>;

    /// React to a dispatched action.
    ///
    /// Called after [`AppState::dispatch`] has already mutated shared
    /// state. Use this to update component-local state (e.g. reset
    /// a table selection on tab change).
    fn update(&mut self, action: &Action, state: &AppState);

    /// Render the component into the given area.
    ///
    /// Receives the full [`AppState`] for read-only access to shared
    /// data (peers, config, theme, etc.).
    fn render(&mut self, frame: &mut Frame, area: Rect, focused: bool, state: &AppState);
}
