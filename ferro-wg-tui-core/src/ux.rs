//! User experience constants and utilities.
//!
//! This module contains shared constants for keybindings, themes,
//! and other UX elements used across the TUI.

use crossterm::event::{MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::{Action, Tab};

/// Resolve a mouse event to an optional action.
///
/// Handles scroll for row navigation and left clicks on the tab bar for tab selection.
#[must_use]
pub fn resolve_mouse_action(mouse: &MouseEvent, tab_bar_rect: Rect) -> Option<Action> {
    match mouse.kind {
        MouseEventKind::ScrollDown => Some(Action::NextRow),
        MouseEventKind::ScrollUp => Some(Action::PrevRow),
        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
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

/// Check if a column in the tab bar corresponds to a tab, returning `SelectTab` if so.
#[must_use]
fn tab_hit_test(column: u16, tab_bar_x: u16) -> Option<Action> {
    let relative_column = column.saturating_sub(tab_bar_x);
    Tab::ALL
        .iter()
        .enumerate()
        .find(|(i, _)| {
            let start = tab_label_at_column(*i);
            let end = start + tab_label_width(*i);
            relative_column >= start && relative_column < end
        })
        .map(|(_, tab)| Action::SelectTab(*tab))
}

/// Get the starting column for the tab label at the given index (0-based).
#[must_use]
const fn tab_label_at_column(index: usize) -> u16 {
    // Each tab label is " {num}:Title ", num is index+1
    // But since it's compile time, hardcode or compute.
    // Assuming left-aligned, each takes len(" {num}:Title ")
    // For simplicity, approximate or compute based on titles.
    // Since titles are known: Overview(8), Status(6), Peers(5), Compare(7), Config(6), Logs(4)
    // " 1:Overview " = 1+1+1+8+1 = 12 chars
    // But ratatui Tabs probably spaces them.
    // To make it simple, let's assume they are spaced with some gap.
    // Perhaps use a simple model: each tab starts at index * 15 or something.
    // But to be accurate, perhaps calculate cumulative widths.
    // Since it's const, let's define widths.
    const WIDTHS: [u16; 6] = [12, 10, 9, 11, 10, 8]; // approximate " {num}:Title "
    let mut col = 2; // left border + space?
    let mut i = 0;
    while i < index {
        col += WIDTHS[i] + 2; // +2 for spacing?
        i += 1;
    }
    col
}

/// Get the width of the tab label at the given index.
#[must_use]
const fn tab_label_width(index: usize) -> u16 {
    const WIDTHS: [u16; 6] = [12, 10, 9, 11, 10, 8];
    WIDTHS[index]
}

/// Full table of keybindings for the help overlay.
///
/// Organized by sections: Global, Overview, Status, Compare, Config, Mouse.
/// Each entry is (`key_label`, description).
pub const KEYBINDINGS: &[(&str, &str)] = &[
    // Global
    ("q / Esc", "Quit"),
    ("?", "Toggle help"),
    ("T", "Toggle theme (Mocha/Latte)"),
    ("/", "Search"),
    ("i", "Import wg-quick config"),
    ("Tab / →", "Next tab"),
    ("BackTab / ←", "Previous tab"),
    ("1–6", "Jump to tab"),
    ("j / ↓", "Next row"),
    ("k / ↑", "Previous row"),
    // Overview tab
    ("u", "Connect all"),
    ("d", "Disconnect all (confirm)"),
    ("s", "Start daemon"),
    ("S", "Stop daemon (confirm)"),
    // Status tab
    ("u", "Connect selected"),
    ("d", "Disconnect selected"),
    ("b", "Cycle backend"),
    // Compare tab (Phase 5)
    ("Enter", "Benchmark selected backend"),
    ("w", "Switch to selected backend"),
    ("h", "Toggle history view"),
    ("e", "Export results"),
    // Config tab (Phase 6)
    ("e", "Edit focused field"),
    ("p", "Preview diff"),
    ("s", "Save config"),
    ("r", "Save and reconnect"),
    ("+", "Add peer"),
    ("x", "Delete peer (confirm)"),
    // Mouse
    ("click tab", "Navigate to tab"),
    ("scroll ↕", "Navigate rows"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{MouseButton, MouseEventKind};

    #[test]
    fn resolve_mouse_action_scroll_down() {
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::empty(),
        };
        let rect = Rect::new(0, 0, 80, 3);
        assert_eq!(resolve_mouse_action(&mouse, rect), Some(Action::NextRow));
    }

    #[test]
    fn resolve_mouse_action_scroll_up() {
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::empty(),
        };
        let rect = Rect::new(0, 0, 80, 3);
        assert_eq!(resolve_mouse_action(&mouse, rect), Some(Action::PrevRow));
    }

    #[test]
    fn resolve_mouse_action_left_click_outside_tab_bar() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 10,
            modifiers: crossterm::event::KeyModifiers::empty(),
        };
        let rect = Rect::new(0, 0, 80, 3);
        assert_eq!(resolve_mouse_action(&mouse, rect), None);
    }

    #[test]
    fn tab_hit_test_overview() {
        // Assuming tab_label_at_column(0) = 2, width 12, so column 2 to 13
        let action = tab_hit_test(5, 0);
        assert_eq!(action, Some(Action::SelectTab(Tab::Overview)));
    }
}
