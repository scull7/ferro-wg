//! Compare tab: backend performance comparison.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{
    BarChart, Cell, Gauge, List, ListItem, ListState, Row, Sparkline, Table, TableState,
};

use ferro_wg_tui_core::state::CompareView;
use ferro_wg_tui_core::{Action, AppState, Component};

use ferro_wg_tui_core::benchmark::{
    best_backend, format_latency, format_throughput, throughput_bar_data, throughput_sparkline_data,
};

/// Number of backend rows (boringtun, neptun, gotatun).
const BACKEND_COUNT: usize = 3;

/// Backend performance comparison tab.
///
/// Displays either live benchmark results or historical runs.
pub struct CompareComponent {
    /// Selection state for live table.
    table_state: TableState,
    /// Selection state for historical list.
    list_state: ListState,
}

impl CompareComponent {
    /// Create a new compare component.
    #[must_use]
    pub fn new() -> Self {
        Self {
            table_state: TableState::default().with_selected(Some(0)),
            list_state: ListState::default().with_selected(Some(0)),
        }
    }
}

impl Default for CompareComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl CompareComponent {
    /// Get the backend name for the currently selected row.
    fn selected_backend(&self) -> &'static str {
        const BACKENDS: [&str; 3] = ["boringtun", "neptun", "gotatun"];
        BACKENDS
            .get(self.table_state.selected().unwrap_or(0))
            .copied()
            .unwrap_or("boringtun")
    }
}

impl Component for CompareComponent {
    fn handle_key(&mut self, key: KeyEvent, _state: &AppState) -> Option<Action> {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => Some(Action::NextRow),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::PrevRow),
            KeyCode::Char('b') => Some(Action::StartBenchmark),
            KeyCode::Enter => Some(Action::StartBenchmarkForBackend(
                self.selected_backend().to_string(),
            )),
            KeyCode::Char('w') => Some(Action::SwitchBenchmarkBackend(
                self.selected_backend().to_string(),
            )),
            KeyCode::Char('h') => Some(Action::ToggleCompareView),
            KeyCode::Char('e') => Some(Action::EnterExport),
            _ => None,
        }
    }

    fn update(&mut self, action: &Action, state: &AppState) {
        match state.compare_view {
            CompareView::Live => {
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
            CompareView::Historical => {
                let max = state.benchmark_history.len().saturating_sub(1);
                let current = self.list_state.selected().unwrap_or(0);
                match action {
                    Action::NextRow => {
                        self.list_state
                            .select(Some(current.saturating_add(1).min(max)));
                    }
                    Action::PrevRow => {
                        self.list_state.select(Some(current.saturating_sub(1)));
                    }
                    Action::SelectTab(_) | Action::NextTab | Action::PrevTab => {
                        self.list_state.select(Some(0));
                    }
                    _ => {}
                }
            }
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, focused: bool, state: &AppState) {
        if area.height == 0 || area.width < 20 {
            return;
        }
        match state.compare_view {
            CompareView::Live => self.render_live(frame, area, focused, state),
            CompareView::Historical => self.render_historical(frame, area, state),
        }
    }
}

impl CompareComponent {
    fn render_live(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        let theme = &state.theme;

        // Split area vertically: table, barchart, sparkline, gauge
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(6), // table: header + 3 rows + padding
                Constraint::Length(6), // barchart
                Constraint::Length(3), // sparkline
                Constraint::Length(1), // gauge (if running)
            ])
            .split(area);

        // Table
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
                let result = state.benchmark_results.get(*name);
                let throughput =
                    result.map_or("—".to_string(), |r| format_throughput(r.throughput_bps));
                let latency = result.map_or("—".to_string(), |r| format_latency(r.avg_latency));
                let placeholder = Cell::from("—").style(Style::default().fg(theme.muted));
                Row::new(vec![
                    Cell::from(*name),
                    Cell::from(avail).style(avail_style),
                    placeholder.clone(),
                    Cell::from(throughput),
                    Cell::from(latency),
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
        .block(theme.panel_block("Compare"))
        .row_highlight_style(theme.highlight_style());

        frame.render_stateful_widget(table, chunks[0], &mut self.table_state);

        // BarChart
        let bar_data = throughput_bar_data(&state.benchmark_results);
        let barchart = BarChart::default()
            .block(theme.panel_block("Throughput"))
            .data(&bar_data)
            .bar_width(3)
            .bar_gap(1)
            .bar_style(Style::default().fg(theme.accent))
            .value_style(Style::default().fg(theme.text));

        frame.render_widget(barchart, chunks[1]);

        // Sparkline
        let (left, right) = state.benchmark_progress_history.as_slices();
        let progress = if left.is_empty() { right } else { left };
        let spark_data = throughput_sparkline_data(progress);
        let sparkline = Sparkline::default()
            .block(theme.panel_block("Live Throughput"))
            .data(&spark_data)
            .style(Style::default().fg(theme.accent));

        frame.render_widget(sparkline, chunks[2]);

        // Gauge (progress bar)
        if state.benchmark_running {
            if let Some(progress) = state.benchmark_progress_history.back() {
                let ratio = f64::from(progress.elapsed_secs) / f64::from(progress.total_secs);
                let title = format!(
                    "Progress ({}/{})",
                    progress.elapsed_secs, progress.total_secs
                );
                let gauge = Gauge::default()
                    .block(theme.panel_block(&title))
                    .gauge_style(Style::default().fg(theme.success))
                    .ratio(ratio.min(1.0));
                frame.render_widget(gauge, chunks[3]);
            } else {
                let gauge = Gauge::default()
                    .block(theme.panel_block("Progress (0/10)"))
                    .gauge_style(Style::default().fg(theme.success))
                    .ratio(0.0);
                frame.render_widget(gauge, chunks[3]);
            }
        }
    }

    fn render_historical(&mut self, frame: &mut Frame, area: Rect, state: &AppState) {
        let theme = &state.theme;

        // Reverse history to show newest first
        let items: Vec<ListItem> = state
            .benchmark_history
            .iter()
            .rev()
            .map(|run| {
                let timestamp = run.timestamp_ms;
                let best = best_backend(&run.results).unwrap_or("none");
                let text = format!("{} - {} - best: {}", timestamp, run.connection_name, best);
                ListItem::new(text)
            })
            .collect();

        let list = List::new(items)
            .block(theme.panel_block("Compare (historical)"))
            .highlight_style(theme.highlight_style());

        frame.render_stateful_widget(list, area, &mut self.list_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_wg_core::config::AppConfig;

    fn test_state() -> AppState {
        AppState::new(AppConfig::default())
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

    #[test]
    fn handle_key_b_emits_start_benchmark() {
        let mut comp = CompareComponent::new();
        let state = test_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('b')), &state),
            Some(Action::StartBenchmark)
        );
    }

    #[test]
    fn handle_key_enter_emits_start_benchmark_for_backend() {
        let mut comp = CompareComponent::new();
        let state = test_state();
        // selected 0 -> boringtun
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Enter), &state),
            Some(Action::StartBenchmarkForBackend("boringtun".to_string()))
        );
    }

    #[test]
    fn handle_key_w_emits_switch_backend() {
        let mut comp = CompareComponent::new();
        let state = test_state();
        // set selected to 2
        comp.table_state.select(Some(2));
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('w')), &state),
            Some(Action::SwitchBenchmarkBackend("gotatun".to_string()))
        );
    }

    #[test]
    fn handle_key_h_emits_toggle_compare_view() {
        let mut comp = CompareComponent::new();
        let state = test_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('h')), &state),
            Some(Action::ToggleCompareView)
        );
    }

    #[test]
    fn handle_key_e_emits_enter_export() {
        let mut comp = CompareComponent::new();
        let state = test_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('e')), &state),
            Some(Action::EnterExport)
        );
    }
}
