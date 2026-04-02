//! Terminal UI for ferro-wg.
//!
//! This crate provides the entry point ([`run`]) that sets up the
//! terminal, creates components and state, and drives the event loop.
//! It wires together types from [`ferro_wg_tui_core`] and component
//! implementations from [`ferro_wg_tui_components`].

mod event;

use std::io;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};

use ferro_wg_core::config::WgConfig;
use ferro_wg_tui_components::{
    CompareComponent, ConfigComponent, LogsComponent, PeersComponent, StatusBarComponent,
    StatusComponent, TabBarComponent,
};
use ferro_wg_tui_core::{Action, AppState, Component, InputMode, Tab};

use event::{AppEvent, EventHandler};

/// Run the interactive TUI.
///
/// Sets up the terminal, creates the component tree and application
/// state, and drives the event loop until the user quits.
///
/// # Errors
///
/// Returns an error if terminal setup, event handling, or teardown
/// fails.
pub async fn run(wg_config: WgConfig) -> Result<(), Box<dyn std::error::Error>> {
    // Setup terminal.
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Run the event loop, then restore terminal regardless of outcome.
    let result = event_loop(&mut terminal, wg_config).await;

    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    result
}

/// Drive the TUI event loop until the user quits.
async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    wg_config: WgConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut state = AppState::new(wg_config);

    // Components are stored separately from AppState to avoid
    // split-borrow issues during rendering (&mut component + &state).
    let mut components: Vec<Box<dyn Component>> = vec![
        Box::new(StatusComponent::new()),
        Box::new(PeersComponent::new()),
        Box::new(CompareComponent::new()),
        Box::new(ConfigComponent::new()),
        Box::new(LogsComponent::new()),
    ];
    let mut tab_bar = TabBarComponent::new();
    let mut status_bar = StatusBarComponent::new();

    let mut events = EventHandler::new(Duration::from_millis(250));

    while state.running {
        terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // Tab bar
                    Constraint::Min(0),    // Main content
                    Constraint::Length(3), // Status bar / search
                ])
                .split(frame.area());

            tab_bar.render(frame, chunks[0], false, &state);
            components[state.active_tab.index()].render(frame, chunks[1], true, &state);
            status_bar.render(frame, chunks[2], false, &state);
        })?;

        match events.next().await {
            Some(AppEvent::Key(key)) => {
                let action = if state.input_mode == InputMode::Search {
                    status_bar.handle_key(key, &state)
                } else {
                    handle_global_key(key)
                        .or_else(|| components[state.active_tab.index()].handle_key(key, &state))
                };

                if let Some(ref action) = action {
                    state.dispatch(action);
                    for comp in &mut components {
                        comp.update(action, &state);
                    }
                    tab_bar.update(action, &state);
                    status_bar.update(action, &state);
                }
            }
            Some(AppEvent::Tick) => {
                // Future: refresh stats from daemon here.
            }
            None => break,
        }
    }

    Ok(())
}

/// Handle global key events that apply regardless of which component
/// is focused.
///
/// Returns `Some(Action)` if the key was handled, `None` otherwise
/// (to fall through to the active component).
fn handle_global_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(Action::Quit),
        KeyCode::Tab | KeyCode::Right => Some(Action::NextTab),
        KeyCode::BackTab | KeyCode::Left => Some(Action::PrevTab),
        KeyCode::Char('1') => Some(Action::SelectTab(Tab::Status)),
        KeyCode::Char('2') => Some(Action::SelectTab(Tab::Peers)),
        KeyCode::Char('3') => Some(Action::SelectTab(Tab::Compare)),
        KeyCode::Char('4') => Some(Action::SelectTab(Tab::Config)),
        KeyCode::Char('5') => Some(Action::SelectTab(Tab::Logs)),
        KeyCode::Char('/') => Some(Action::EnterSearch),
        _ => None,
    }
}
