//! Diff preview overlay component.
//!
//! [`DiffPreviewComponent`] renders a scrollable overlay showing the
//! unified diff of config changes when `state.config_diff_pending` is `Some`.
//! It captures all key events while active:
//!
//! - `s`        → [`Action::SaveConfig { reconnect: false }`]
//! - `r`        → [`Action::SaveConfig { reconnect: true }`]
//! - `j` / `↓`  → [`Action::ConfigDiffScrollDown`]
//! - `k` / `↑`  → [`Action::ConfigDiffScrollUp`]
//! - `Esc` / `q` → [`Action::DiscardConfigEdits`]

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use ferro_wg_tui_core::config_edit::DiffLine;
use ferro_wg_tui_core::{Action, AppState, Component};

/// Diff preview overlay shown before saving a config edit.
///
/// Renders a scrollable 80%-wide overlay of up to 15 visible lines when
/// `state.config_diff_pending` is `Some`. Key bindings:
///
/// - `s`        → [`Action::SaveConfig { reconnect: false }`]
/// - `r`        → [`Action::SaveConfig { reconnect: true }`]
/// - `j` / `↓`  → [`Action::ConfigDiffScrollDown`]
/// - `k` / `↑`  → [`Action::ConfigDiffScrollUp`]
/// - `Esc` / `q` → [`Action::DiscardConfigEdits`]
pub struct DiffPreviewComponent;

impl DiffPreviewComponent {
    /// Create a new diff preview component.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for DiffPreviewComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for DiffPreviewComponent {
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Option<Action> {
        state.config_diff_pending.as_ref()?;
        match key.code {
            KeyCode::Char('s') => Some(Action::SaveConfig { reconnect: false }),
            KeyCode::Char('r') => Some(Action::SaveConfig { reconnect: true }),
            KeyCode::Char('j') | KeyCode::Down => Some(Action::ConfigDiffScrollDown),
            KeyCode::Char('k') | KeyCode::Up => Some(Action::ConfigDiffScrollUp),
            KeyCode::Esc | KeyCode::Char('q') => Some(Action::DiscardConfigEdits),
            _ => None,
        }
    }

    fn update(&mut self, _action: &Action, _state: &AppState) {}

    fn render(&mut self, frame: &mut Frame, area: Rect, _focused: bool, state: &AppState) {
        let Some(pending) = &state.config_diff_pending else {
            return;
        };
        // 80%-wide, up to 15 visible diff lines plus 4 rows for border + hint
        let overlay_height = u16::try_from(pending.diff_lines.len().min(15)).unwrap_or(15) + 4;
        let overlay_area = crate::util::centered_rect(80, overlay_height, area);
        frame.render_widget(Clear, overlay_area);
        let block = state
            .theme
            .overlay_block("Config Diff — preview")
            .borders(Borders::ALL);
        let inner = block.inner(overlay_area);
        frame.render_widget(block, overlay_area);
        // Render diff lines with colour: Added → theme.success, Removed → theme.error,
        // Context → theme.muted.
        // Bottom hint line: "[s] save   [r] save & reconnect   [Esc] discard"
        render_diff_lines(frame, inner, pending, &state.theme);
    }
}

/// Pure render helper: converts `DiffLine` slice + scroll offset into
/// coloured `Line` spans and a hint footer. Separated from `render` so
/// it can be unit-tested with a `TestBackend`.
fn render_diff_lines(
    frame: &mut Frame,
    area: Rect,
    pending: &ferro_wg_tui_core::config_edit::ConfigDiffPending,
    theme: &ferro_wg_tui_core::theme::Theme,
) {
    let visible_lines = 15.min(area.height.saturating_sub(1)); // 1 for hint
    let start_idx = pending.scroll_offset;
    let end_idx = (start_idx + visible_lines as usize).min(pending.diff_lines.len());

    let mut lines: Vec<Line> = pending.diff_lines[start_idx..end_idx]
        .iter()
        .map(|line| match line {
            DiffLine::Added(content) => Line::from(vec![
                Span::styled("+", Style::default().fg(theme.success)),
                Span::styled(content, Style::default().fg(theme.success)),
            ]),
            DiffLine::Removed(content) => Line::from(vec![
                Span::styled("-", Style::default().fg(theme.error)),
                Span::styled(content, Style::default().fg(theme.error)),
            ]),
            DiffLine::Context(content) => {
                Line::from(Span::styled(content, Style::default().fg(theme.muted)))
            }
        })
        .collect();

    // Add hint line
    lines.push(Line::from(Span::styled(
        "[s] save   [r] save & reconnect   [j/k] scroll   [Esc] discard",
        Style::default().fg(theme.muted),
    )));

    let paragraph = Paragraph::new(lines).block(Block::default());
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_wg_tui_core::AppState;

    fn pending_diff() -> ferro_wg_tui_core::config_edit::ConfigDiffPending {
        ferro_wg_tui_core::config_edit::ConfigDiffPending {
            connection_name: "test".to_string(),
            draft: ferro_wg_core::config::WgConfig {
                interface: ferro_wg_core::config::InterfaceConfig {
                    private_key: ferro_wg_core::key::PrivateKey::from_base64(
                        "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=",
                    )
                    .unwrap(),
                    listen_port: 51820,
                    addresses: vec![],
                    dns: vec![],
                    dns_search: vec![],
                    mtu: 1420,
                    fwmark: 0,
                    pre_up: vec![],
                    post_up: vec![],
                    pre_down: vec![],
                    post_down: vec![],
                },
                peers: vec![],
            },
            diff_lines: vec![DiffLine::Added("test".to_string())],
            scroll_offset: 0,
        }
    }

    fn state_with_pending() -> AppState {
        let mut state = AppState::new(ferro_wg_core::config::AppConfig::default());
        state.config_diff_pending = Some(pending_diff());
        state
    }

    #[test]
    fn handle_key_returns_none_when_no_pending_diff() {
        let mut component = DiffPreviewComponent;
        let state = AppState::new(ferro_wg_core::config::AppConfig::default());
        assert!(state.config_diff_pending.is_none());
        let action = component.handle_key(
            crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char('s')),
            &state,
        );
        assert!(action.is_none());
    }

    #[test]
    fn handle_key_s_emits_save_config_no_reconnect() {
        let mut component = DiffPreviewComponent;
        let state = state_with_pending();
        let action =
            component.handle_key(crossterm::event::KeyEvent::from(KeyCode::Char('s')), &state);
        assert_eq!(
            action,
            Some(ferro_wg_tui_core::Action::SaveConfig { reconnect: false })
        );
    }

    #[test]
    fn handle_key_r_emits_save_config_with_reconnect() {
        let mut component = DiffPreviewComponent;
        let state = state_with_pending();
        let action =
            component.handle_key(crossterm::event::KeyEvent::from(KeyCode::Char('r')), &state);
        assert_eq!(
            action,
            Some(ferro_wg_tui_core::Action::SaveConfig { reconnect: true })
        );
    }

    #[test]
    fn handle_key_j_emits_config_diff_scroll_down() {
        let mut component = DiffPreviewComponent;
        let state = state_with_pending();
        let action =
            component.handle_key(crossterm::event::KeyEvent::from(KeyCode::Char('j')), &state);
        assert_eq!(
            action,
            Some(ferro_wg_tui_core::Action::ConfigDiffScrollDown)
        );
    }

    #[test]
    fn handle_key_down_emits_config_diff_scroll_down() {
        let mut component = DiffPreviewComponent;
        let state = state_with_pending();
        let action = component.handle_key(crossterm::event::KeyEvent::from(KeyCode::Down), &state);
        assert_eq!(
            action,
            Some(ferro_wg_tui_core::Action::ConfigDiffScrollDown)
        );
    }

    #[test]
    fn handle_key_k_emits_config_diff_scroll_up() {
        let mut component = DiffPreviewComponent;
        let state = state_with_pending();
        let action =
            component.handle_key(crossterm::event::KeyEvent::from(KeyCode::Char('k')), &state);
        assert_eq!(action, Some(ferro_wg_tui_core::Action::ConfigDiffScrollUp));
    }

    #[test]
    fn handle_key_up_emits_config_diff_scroll_up() {
        let mut component = DiffPreviewComponent;
        let state = state_with_pending();
        let action = component.handle_key(crossterm::event::KeyEvent::from(KeyCode::Up), &state);
        assert_eq!(action, Some(ferro_wg_tui_core::Action::ConfigDiffScrollUp));
    }

    #[test]
    fn handle_key_esc_emits_discard_config_edits() {
        let mut component = DiffPreviewComponent;
        let state = state_with_pending();
        let action = component.handle_key(crossterm::event::KeyEvent::from(KeyCode::Esc), &state);
        assert_eq!(action, Some(ferro_wg_tui_core::Action::DiscardConfigEdits));
    }

    #[test]
    fn handle_key_q_emits_discard_config_edits() {
        let mut component = DiffPreviewComponent;
        let state = state_with_pending();
        let action =
            component.handle_key(crossterm::event::KeyEvent::from(KeyCode::Char('q')), &state);
        assert_eq!(action, Some(ferro_wg_tui_core::Action::DiscardConfigEdits));
    }

    #[test]
    fn render_diff_lines_added_line_has_success_color() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = ratatui::layout::Rect::new(0, 0, 80, 10);
        let theme = ferro_wg_tui_core::theme::Theme::mocha();
        let mut pending = pending_diff();
        pending.diff_lines = vec![DiffLine::Added("test line".to_string())];

        terminal
            .draw(|frame| {
                render_diff_lines(frame, area, &pending, &theme);
            })
            .unwrap();

        let cells = terminal.backend().buffer().content().to_vec();
        let plus_cell = cells.iter().find(|c| c.symbol() == "+").unwrap();
        assert_eq!(plus_cell.fg, theme.success);
        let t_cell = cells
            .iter()
            .find(|c| c.symbol() == "t" && c.fg == theme.success)
            .unwrap();
        assert_eq!(t_cell.fg, theme.success);
    }

    #[test]
    fn render_diff_lines_removed_line_has_error_color() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = ratatui::layout::Rect::new(0, 0, 80, 10);
        let theme = ferro_wg_tui_core::theme::Theme::mocha();
        let mut pending = pending_diff();
        pending.diff_lines = vec![DiffLine::Removed("removed line".to_string())];

        terminal
            .draw(|frame| {
                render_diff_lines(frame, area, &pending, &theme);
            })
            .unwrap();

        let cells = terminal.backend().buffer().content().to_vec();
        let minus_cell = cells.iter().find(|c| c.symbol() == "-").unwrap();
        assert_eq!(minus_cell.fg, theme.error);
    }

    #[test]
    fn render_diff_lines_context_line_has_muted_color() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = ratatui::layout::Rect::new(0, 0, 80, 10);
        let theme = ferro_wg_tui_core::theme::Theme::mocha();
        let mut pending = pending_diff();
        pending.diff_lines = vec![DiffLine::Context("ctx".to_string())];

        terminal
            .draw(|frame| {
                render_diff_lines(frame, area, &pending, &theme);
            })
            .unwrap();

        let cells = terminal.backend().buffer().content().to_vec();
        let c_cell = cells
            .iter()
            .find(|c| c.symbol() == "c" && c.fg == theme.muted)
            .unwrap();
        assert_eq!(c_cell.fg, theme.muted);
    }
}
