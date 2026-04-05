//! Toast notifications: bottom-right corner overlays.

use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use ferro_wg_tui_core::{Action, AppState, Component};

/// Toast component for displaying transient messages in the bottom-right corner.
pub struct ToastComponent;

impl ToastComponent {
    /// Create a new toast component.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ToastComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for ToastComponent {
    fn handle_key(&mut self, _key: KeyEvent, _state: &AppState) -> Option<Action> {
        None
    }

    fn update(&mut self, _action: &Action, _state: &AppState) {
        // No local state.
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        let mut y = area.height.saturating_sub(1);
        for toast in state.toasts.iter().rev() {
            if y == 0 {
                break;
            }
            let style = if toast.is_error {
                Style::default().fg(state.theme.error)
            } else {
                Style::default().fg(state.theme.success)
            };
            let indicator = if toast.is_error { "x " } else { "* " };
            let text = format!("{}{}", indicator, toast.message);
            let line = Line::from(Span::styled(text, style));
            let toast_width = 40.min(area.width);
            let x = area.width.saturating_sub(toast_width);
            let rect = Rect {
                x,
                y,
                width: toast_width,
                height: 1,
            };
            frame.render_widget(Clear, rect);
            frame.render_widget(Paragraph::new(line), rect);
            y = y.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_wg_core::config::AppConfig;
    use ferro_wg_tui_core::theme::ThemeKind;
    use ferro_wg_tui_core::Toast;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn terminal_to_string(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let width = buffer.area().width as usize;
        buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<Vec<_>>()
            .chunks(width)
            .map(|line| line.join(""))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }

    fn state_with_toasts(toasts: Vec<Toast>) -> AppState {
        let mut state = AppState::new(AppConfig::default());
        state.toasts = toasts.into();
        state.theme_kind = ThemeKind::Mocha;
        state
    }

    #[test]
    fn render_no_toasts() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut comp = ToastComponent::new();
        let state = state_with_toasts(vec![]);

        terminal
            .draw(|frame| comp.render(frame, frame.area(), false, &state))
            .unwrap();

        insta::assert_snapshot!(terminal_to_string(&terminal));
    }

    #[test]
    fn render_single_success_toast() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut comp = ToastComponent::new();
        let state = state_with_toasts(vec![Toast::success("tunnel up".into())]);

        terminal
            .draw(|frame| comp.render(frame, frame.area(), false, &state))
            .unwrap();

        insta::assert_snapshot!(terminal_to_string(&terminal));
    }

    #[test]
    fn render_single_error_toast() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut comp = ToastComponent::new();
        let state = state_with_toasts(vec![Toast::error("connection failed".into())]);

        terminal
            .draw(|frame| comp.render(frame, frame.area(), false, &state))
            .unwrap();

        insta::assert_snapshot!(terminal_to_string(&terminal));
    }

    #[test]
    fn render_multiple_toasts() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut comp = ToastComponent::new();
        let state = state_with_toasts(vec![
            Toast::success("connected".into()),
            Toast::error("timeout".into()),
        ]);

        terminal
            .draw(|frame| comp.render(frame, frame.area(), false, &state))
            .unwrap();

        insta::assert_snapshot!(terminal_to_string(&terminal));
    }

    #[test]
    fn render_toast_truncated() {
        let mut terminal = Terminal::new(TestBackend::new(40, 24)).unwrap();
        let mut comp = ToastComponent::new();
        let state = state_with_toasts(vec![Toast::success(
            "very long message that will be truncated".into(),
        )]);

        terminal
            .draw(|frame| comp.render(frame, frame.area(), false, &state))
            .unwrap();

        insta::assert_snapshot!(terminal_to_string(&terminal));
    }
}
