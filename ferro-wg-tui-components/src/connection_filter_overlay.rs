//! Connection filter overlay component.
//!
//! [`ConnectionFilterOverlayComponent`] renders a centered modal overlay
//! for filtering connection visibility. Displays a searchable list of connections
//! with checkboxes for toggling visibility.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use ferro_wg_tui_core::{Action, AppState, Component};

/// A modal overlay for filtering connection visibility.
///
/// Activated when `state.show_connection_filter` is `true`. Renders a centered
/// overlay with a search input and a scrollable list of connections with checkboxes.
pub struct ConnectionFilterOverlayComponent;

impl ConnectionFilterOverlayComponent {
    /// Create a new connection filter overlay component.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ConnectionFilterOverlayComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for ConnectionFilterOverlayComponent {
    /// Route keys when the connection filter overlay is active.
    ///
    /// Returns `None` when the overlay is not shown (acts as a no-op).
    /// `Esc` or `q` → [`Action::HideConnectionFilter`]; other keys are swallowed.
    /// TODO: Implement search input, navigation, toggling.
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action> {
        if !state.show_connection_filter {
            return None;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Some(Action::HideConnectionFilter),
            // TODO: Add search, toggle, navigation
            _ => None,
        }
    }

    fn update(&mut self, _action: &Action, _state: &AppState) {}

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        if !state.show_connection_filter {
            return;
        }
        let overlay_area = crate::util::centered_rect(80, 20, area);
        frame.render_widget(Clear, overlay_area);
        let block = state.theme.overlay_block("Connection Filter");
        let inner = block.inner(overlay_area);
        frame.render_widget(block, overlay_area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(inner);

        // Search input
        let search_para = Paragraph::new(format!("Search: {}", state.connection_filter_search))
            .block(Block::default().borders(Borders::ALL).title("Search"));
        frame.render_widget(search_para, chunks[0]);

        // Connection list
        let items: Vec<ListItem> = state
            .connections
            .iter()
            .map(|conn| {
                let checked = state.visible_connections.contains(&conn.name);
                let checkbox = if checked { "[x]" } else { "[ ]" };
                ListItem::new(format!("{} {}", checkbox, conn.name))
            })
            .collect();
        let list =
            List::new(items).block(Block::default().borders(Borders::ALL).title("Connections"));
        frame.render_widget(list, chunks[1]);
    }
}
