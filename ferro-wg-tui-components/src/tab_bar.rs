//! Tab bar: top-of-screen tab selector.

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Tabs};

use ferro_wg_tui_core::{Action, AppState, Component, Tab};

/// Top-of-screen tab bar showing all available tabs with numeric
/// labels. Tab switching is handled by the global key handler, so
/// this component is render-only.
pub struct TabBarComponent;

impl TabBarComponent {
    /// Create a new tab bar component.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for TabBarComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for TabBarComponent {
    fn handle_key(&mut self, _key: KeyEvent, _state: &AppState) -> Option<Action> {
        // Tab switching is handled globally, not by this component.
        None
    }

    fn update(&mut self, _action: &Action, _state: &AppState) {
        // No local state.
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        let theme = &state.theme;

        let titles: Vec<Line<'_>> = Tab::ALL
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let num = i + 1;
                Line::from(format!(" {num}:{} ", t.title()))
            })
            .collect();

        let tabs = Tabs::new(titles)
            .block(Block::default().borders(Borders::ALL).title(" ferro-wg "))
            .select(state.active_tab.index())
            .style(theme.inactive_tab_style())
            .highlight_style(theme.active_tab_style());

        frame.render_widget(tabs, area);
    }
}
