//! Tab bar: top-of-screen tab selector.
//!
//! Also owns mouse hit-testing for the tab bar — the geometry (label widths)
//! is defined here so that rendering and event handling stay in the same layer.

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Tabs};

use ferro_wg_tui_core::{Action, AppState, Component, Tab};

/// Number of rows this component occupies in the layout.
///
/// One row for the [`Borders::ALL`] top border, one for the tab titles, and
/// one for the bottom border.
pub const TAB_BAR_HEIGHT: u16 = 3;

/// Number of tabs.
const TAB_COUNT: usize = Tab::ALL.len();

/// Width (in terminal columns) of each tab label when rendered as `" {n}:{title} "`.
///
/// For tab `i` (0-based) the rendered label is `format!(" {}:{} ", i + 1, tab.title())`.
/// All six indices are single digits, so width = 4 (space + digit + colon + space) +
/// `title.len()`.  Values: Overview=12, Status=10, Peers=9, Compare=11, Config=10, Logs=8.
const TAB_WIDTHS: [u16; TAB_COUNT] = {
    let mut w = [0u16; TAB_COUNT];
    let mut i = 0;
    while i < TAB_COUNT {
        let len = Tab::ALL[i].title().len();
        assert!(len < 250, "tab title too long to fit in u16");
        #[allow(clippy::cast_possible_truncation)]
        {
            w[i] = 4 + len as u16;
        }
        i += 1;
    }
    w
};

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

/// Resolve a mouse event to an optional [`Action`].
///
/// Handles scroll for row navigation and left-clicks on the tab bar for tab
/// selection.  Returns `None` for all other event kinds.
#[must_use]
pub fn resolve_mouse_action(mouse: &MouseEvent, tab_bar_rect: Rect) -> Option<Action> {
    match mouse.kind {
        MouseEventKind::ScrollDown => Some(Action::NextRow),
        MouseEventKind::ScrollUp => Some(Action::PrevRow),
        MouseEventKind::Down(MouseButton::Left) => {
            if mouse.row >= tab_bar_rect.y
                && mouse.row < tab_bar_rect.y + tab_bar_rect.height
                && mouse.column >= tab_bar_rect.x
            {
                tab_hit_test(mouse.column, tab_bar_rect.x)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Map an absolute `column` inside the tab bar to a [`Action::SelectTab`].
///
/// `tab_bar_x` is the left edge of the tab bar rect.  Returns `None` when the
/// column does not fall inside any label.
#[must_use]
fn tab_hit_test(column: u16, tab_bar_x: u16) -> Option<Action> {
    let relative = column.saturating_sub(tab_bar_x);
    Tab::ALL
        .iter()
        .enumerate()
        .find(|(i, _)| {
            let start = tab_label_start(*i);
            let end = start + TAB_WIDTHS[*i];
            relative >= start && relative < end
        })
        .map(|(_, tab)| Action::SelectTab(*tab))
}

/// Starting column (relative to the tab bar's left edge) of the label for tab `index`.
const fn tab_label_start(index: usize) -> u16 {
    let mut col = 2u16; // ratatui Tabs widget adds a 2-column left margin
    let mut i = 0;
    while i < index {
        col += TAB_WIDTHS[i] + 1; // +1 for the inter-tab divider character
        i += 1;
    }
    col
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn left_click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::empty(),
        }
    }

    fn scroll_event(down: bool) -> MouseEvent {
        MouseEvent {
            kind: if down {
                MouseEventKind::ScrollDown
            } else {
                MouseEventKind::ScrollUp
            },
            column: 0,
            row: 0,
            modifiers: KeyModifiers::empty(),
        }
    }

    // --- TAB_WIDTHS correctness ---

    #[test]
    fn tab_widths_match_rendered_labels() {
        for (i, tab) in Tab::ALL.iter().enumerate() {
            let label = format!(" {}:{} ", i + 1, tab.title());
            assert_eq!(
                TAB_WIDTHS[i],
                u16::try_from(label.len()).unwrap(),
                "TAB_WIDTHS[{i}] mismatch for {tab:?}"
            );
        }
    }

    // --- scroll ---

    #[test]
    fn scroll_down_returns_next_row() {
        let rect = Rect::new(0, 0, 80, 3);
        assert_eq!(
            resolve_mouse_action(&scroll_event(true), rect),
            Some(Action::NextRow)
        );
    }

    #[test]
    fn scroll_up_returns_prev_row() {
        let rect = Rect::new(0, 0, 80, 3);
        assert_eq!(
            resolve_mouse_action(&scroll_event(false), rect),
            Some(Action::PrevRow)
        );
    }

    // --- click outside tab bar ---

    #[test]
    fn click_below_tab_bar_returns_none() {
        let rect = Rect::new(0, 0, 80, 3);
        let mouse = left_click(5, 10);
        assert_eq!(resolve_mouse_action(&mouse, rect), None);
    }

    // --- tab hit tests for each tab ---

    fn click_tab(tab_index: usize) -> Option<Action> {
        // Click in the centre of the label for the given tab.
        let start = tab_label_start(tab_index);
        let col = start + TAB_WIDTHS[tab_index] / 2;
        let rect = Rect::new(0, 0, 80, 3);
        let mouse = left_click(col, 1);
        resolve_mouse_action(&mouse, rect)
    }

    #[test]
    fn click_tab_overview() {
        assert_eq!(click_tab(0), Some(Action::SelectTab(Tab::Overview)));
    }

    #[test]
    fn click_tab_status() {
        assert_eq!(click_tab(1), Some(Action::SelectTab(Tab::Status)));
    }

    #[test]
    fn click_tab_peers() {
        assert_eq!(click_tab(2), Some(Action::SelectTab(Tab::Peers)));
    }

    #[test]
    fn click_tab_compare() {
        assert_eq!(click_tab(3), Some(Action::SelectTab(Tab::Compare)));
    }

    #[test]
    fn click_tab_config() {
        assert_eq!(click_tab(4), Some(Action::SelectTab(Tab::Config)));
    }

    #[test]
    fn click_tab_logs() {
        assert_eq!(click_tab(5), Some(Action::SelectTab(Tab::Logs)));
    }

    // --- out-of-range click ---

    #[test]
    fn click_past_all_tabs_returns_none() {
        // Column 79 is past all tab labels (total label span ~65 cols).
        let rect = Rect::new(0, 0, 80, 3);
        let mouse = left_click(79, 1);
        assert_eq!(resolve_mouse_action(&mouse, rect), None);
    }

    #[test]
    fn click_before_first_tab_returns_none() {
        // Column 0 is before the 2-column margin, so no tab should match.
        let rect = Rect::new(0, 0, 80, 3);
        let mouse = left_click(0, 1);
        assert_eq!(resolve_mouse_action(&mouse, rect), None);
    }

    // --- label boundary precision ---

    #[test]
    fn click_left_edge_of_overview_selects_overview() {
        let start = tab_label_start(0);
        let rect = Rect::new(0, 0, 80, 3);
        assert_eq!(
            resolve_mouse_action(&left_click(start, 1), rect),
            Some(Action::SelectTab(Tab::Overview))
        );
    }

    #[test]
    fn click_right_edge_of_overview_selects_overview() {
        let last = tab_label_start(0) + TAB_WIDTHS[0] - 1;
        let rect = Rect::new(0, 0, 80, 3);
        assert_eq!(
            resolve_mouse_action(&left_click(last, 1), rect),
            Some(Action::SelectTab(Tab::Overview))
        );
    }

    // --- non-zero rect x-offset ---

    #[test]
    fn click_with_nonzero_rect_x_offset() {
        let rect = Rect::new(5, 0, 80, 3);
        let abs_col = 5 + tab_label_start(0) + TAB_WIDTHS[0] / 2;
        assert_eq!(
            resolve_mouse_action(&left_click(abs_col, 1), rect),
            Some(Action::SelectTab(Tab::Overview))
        );
    }
}
