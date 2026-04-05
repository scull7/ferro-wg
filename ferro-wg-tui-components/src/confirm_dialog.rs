//! Confirmation overlay dialog component.
//!
//! [`ConfirmDialogComponent`] renders a centered modal box when
//! `state.pending_confirm` is `Some`. It captures all key events
//! while active: `y`/`Y` emits [`Action::ConfirmYes`]; every other
//! key emits [`Action::ConfirmNo`], ensuring no key leaks through to
//! the underlying tab.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Clear, Paragraph};

use ferro_wg_tui_core::{Action, AppState, Component};

/// A modal confirmation dialog overlaid on the content area.
///
/// Activated when `state.pending_confirm` is `Some`. Renders a centered
/// 60%-wide × 5-row box containing the pending message and
/// `[y] confirm   [n] cancel` hints.
pub struct ConfirmDialogComponent;

impl ConfirmDialogComponent {
    /// Create a new confirmation dialog component.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ConfirmDialogComponent {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute a centered [`Rect`] with a percentage width and fixed height.
///
/// `pct_x` is the desired width as a percentage of `area.width` (0–100).
/// The returned rect is clamped to fit within `area`.
fn centered_rect(pct_x: u16, height: u16, area: Rect) -> Rect {
    let width = area.width * pct_x / 100;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

impl Component for ConfirmDialogComponent {
    /// Route keys when the confirmation dialog is active.
    ///
    /// Returns `None` when no dialog is pending (acts as a no-op).
    /// `y`/`Y` → [`Action::ConfirmYes`]; all other keys → [`Action::ConfirmNo`]
    /// (including `n`, `N`, `Esc`), which both cancels and swallows the event.
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action> {
        state.pending_confirm.as_ref()?;
        match key.code {
            KeyCode::Char('y' | 'Y') => Some(Action::ConfirmYes),
            _ => Some(Action::ConfirmNo),
        }
    }

    fn update(&mut self, _action: &Action, _state: &AppState) {}

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        let Some(pending) = &state.pending_confirm else {
            return;
        };
        let overlay_area = centered_rect(60, 5, area);
        frame.render_widget(Clear, overlay_area);
        let block = state.theme.overlay_block("Confirm");
        let inner = block.inner(overlay_area);
        frame.render_widget(block, overlay_area);
        let text = format!("{}\n\n[y] confirm   [n] cancel", pending.message);
        frame.render_widget(Paragraph::new(text).centered(), inner);
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use ferro_wg_core::config::AppConfig;
    use ferro_wg_tui_core::{Action, ConfirmAction};

    fn base_state() -> AppState {
        AppState::new(AppConfig::default())
    }

    fn state_with_confirm(msg: &str, action: ConfirmAction) -> AppState {
        let mut state = AppState::new(AppConfig::default());
        state.dispatch(&Action::RequestConfirm {
            message: msg.to_owned(),
            action,
        });
        state
    }

    // ── handle_key ─────────────────────────────────────────────────────────────

    #[test]
    fn returns_none_when_no_dialog_pending() {
        let mut comp = ConfirmDialogComponent::new();
        let state = base_state();
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('y')), &state),
            None,
        );
    }

    #[test]
    fn y_returns_confirm_yes() {
        let mut comp = ConfirmDialogComponent::new();
        let state = state_with_confirm("Are you sure?", ConfirmAction::DisconnectAll);
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('y')), &state),
            Some(Action::ConfirmYes),
        );
    }

    #[test]
    fn uppercase_y_returns_confirm_yes() {
        let mut comp = ConfirmDialogComponent::new();
        let state = state_with_confirm("Stop daemon?", ConfirmAction::StopDaemon);
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('Y')), &state),
            Some(Action::ConfirmYes),
        );
    }

    #[test]
    fn n_returns_confirm_no() {
        let mut comp = ConfirmDialogComponent::new();
        let state = state_with_confirm("Are you sure?", ConfirmAction::DisconnectAll);
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Char('n')), &state),
            Some(Action::ConfirmNo),
        );
    }

    #[test]
    fn esc_returns_confirm_no() {
        let mut comp = ConfirmDialogComponent::new();
        let state = state_with_confirm("Are you sure?", ConfirmAction::DisconnectAll);
        assert_eq!(
            comp.handle_key(KeyEvent::from(KeyCode::Esc), &state),
            Some(Action::ConfirmNo),
        );
    }

    #[test]
    fn other_keys_return_confirm_no() {
        let mut comp = ConfirmDialogComponent::new();
        let state = state_with_confirm("Are you sure?", ConfirmAction::DisconnectAll);
        for key in [KeyCode::Enter, KeyCode::Tab, KeyCode::Char('x')] {
            assert_eq!(
                comp.handle_key(KeyEvent::from(key), &state),
                Some(Action::ConfirmNo),
                "expected ConfirmNo for {key:?}",
            );
        }
    }

    // ── render ─────────────────────────────────────────────────────────────────

    #[test]
    fn render_no_dialog_no_panic() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = base_state();
        terminal
            .draw(|frame| {
                let mut comp = ConfirmDialogComponent::new();
                comp.render(frame, frame.area(), false, &state);
            })
            .unwrap();
        // No content should be drawn when no dialog is pending.
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(
            !content.contains("Confirm"),
            "no dialog overlay expected when pending_confirm is None"
        );
    }

    #[test]
    fn render_dialog_shows_message_and_hints() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_confirm("Disconnect all?", ConfirmAction::DisconnectAll);
        terminal
            .draw(|frame| {
                let mut comp = ConfirmDialogComponent::new();
                comp.render(frame, frame.area(), false, &state);
            })
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(content.contains("Confirm"), "expected 'Confirm' title");
        assert!(
            content.contains("confirm"),
            "expected '[y] confirm' hint in: {content:?}"
        );
        assert!(
            content.contains("cancel"),
            "expected '[n] cancel' hint in: {content:?}"
        );
    }

    #[test]
    fn render_narrow_terminal_no_panic() {
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_confirm("Are you sure?", ConfirmAction::StopDaemon);
        terminal
            .draw(|frame| {
                let mut comp = ConfirmDialogComponent::new();
                comp.render(frame, frame.area(), false, &state);
            })
            .unwrap();
    }
}
