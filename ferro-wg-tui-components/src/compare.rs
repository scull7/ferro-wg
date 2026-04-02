//! Compare tab: backend performance comparison.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Cell, Row, Table, TableState};

use ferro_wg_tui_core::{Action, AppState, Component};

/// Number of backend rows (boringtun, neptun, gotatun).
const BACKEND_COUNT: usize = 3;

/// Backend availability and performance comparison table.
///
/// Displays a fixed 3-row table of `WireGuard` backends with availability
/// status. Performance columns (Encap/s, Throughput, Latency) are
/// placeholder dashes until benchmarks are implemented in Phase 5.
pub struct CompareComponent {
    /// Per-component table selection state.
    table_state: TableState,
}

impl CompareComponent {
    /// Create a new compare component.
    #[must_use]
    pub fn new() -> Self {
        Self {
            table_state: TableState::default().with_selected(Some(0)),
        }
    }
}

impl Default for CompareComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for CompareComponent {
    fn handle_key(&mut self, key: KeyEvent, _state: &AppState) -> Option<Action> {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => Some(Action::NextRow),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::PrevRow),
            _ => None,
        }
    }

    fn update(&mut self, action: &Action, _state: &AppState) {
        let max = BACKEND_COUNT.saturating_sub(1);
        let current = self.table_state.selected().unwrap_or(0);

        match action {
            Action::NextRow => {
                self.table_state
                    .select(Some(current.saturating_add(1).min(max)));
            }
            Action::PrevRow => {
                self.table_state.select(Some(current.saturating_sub(1)));
            }
            Action::SelectTab(_) | Action::NextTab | Action::PrevTab => {
                self.table_state.select(Some(0));
            }
            _ => {}
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        let theme = &state.theme;

        let header = Row::new(vec![
            "Backend",
            "Available",
            "Encap/s",
            "Throughput",
            "Latency",
        ])
        .style(theme.header_style());

        let backends = [
            ("boringtun", cfg!(feature = "boringtun")),
            ("neptun", cfg!(feature = "neptun")),
            ("gotatun", cfg!(feature = "gotatun")),
        ];

        let rows: Vec<Row<'_>> = backends
            .iter()
            .map(|(name, available)| {
                let avail = if *available { "yes" } else { "no" };
                let avail_style = if *available {
                    Style::default().fg(theme.success)
                } else {
                    Style::default().fg(theme.error)
                };
                Row::new(vec![
                    Cell::from(*name),
                    Cell::from(avail).style(avail_style),
                    Cell::from("-"),
                    Cell::from("-"),
                    Cell::from("-"),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(20),
                Constraint::Percentage(15),
                Constraint::Percentage(20),
                Constraint::Percentage(25),
                Constraint::Percentage(20),
            ],
        )
        .header(header)
        .block(theme.panel_block("Compare (run benchmarks to populate)"))
        .row_highlight_style(theme.highlight_style());

        frame.render_stateful_widget(table, area, &mut self.table_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_wg_core::config::{InterfaceConfig, WgConfig};
    use ferro_wg_core::key::PrivateKey;

    fn test_state() -> AppState {
        AppState::new(WgConfig {
            interface: InterfaceConfig {
                private_key: PrivateKey::generate(),
                listen_port: 51820,
                addresses: Vec::new(),
                dns: Vec::new(),
                dns_search: Vec::new(),
                mtu: 1420,
                fwmark: 0,
                pre_up: Vec::new(),
                post_up: Vec::new(),
                pre_down: Vec::new(),
                post_down: Vec::new(),
            },
            peers: Vec::new(),
        })
    }

    #[test]
    fn row_count_is_backend_count() {
        // Compare always has exactly 3 rows, regardless of peer count.
        assert_eq!(BACKEND_COUNT, 3);
    }

    #[test]
    fn clamps_at_max_row() {
        let mut comp = CompareComponent::new();
        let state = test_state();
        comp.update(&Action::NextRow, &state);
        comp.update(&Action::NextRow, &state);
        assert_eq!(comp.table_state.selected(), Some(2));
        comp.update(&Action::NextRow, &state);
        assert_eq!(comp.table_state.selected(), Some(2)); // clamped at 2
    }
}
