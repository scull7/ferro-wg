//! Terminal UI for ferro-wg.
//!
//! This crate provides the entry point ([`run`]) that sets up the
//! terminal, creates components and state, and drives the event loop.
//! It wires together types from [`ferro_wg_tui_core`] and component
//! implementations from [`ferro_wg_tui_components`].

mod event;

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use ferro_wg_core::client;
use ferro_wg_core::config::WgConfig;
use ferro_wg_core::error::BackendKind;
use ferro_wg_core::ipc::{DaemonCommand, DaemonResponse, PeerStatus};
use ferro_wg_tui_components::{
    CompareComponent, ConfigComponent, LogsComponent, PeersComponent, StatusBarComponent,
    StatusComponent, TabBarComponent,
};
use ferro_wg_tui_core::{Action, AppState, Component, InputMode, Tab};

use event::{AppEvent, EventHandler};

/// Messages sent from background daemon tasks to the event loop.
enum DaemonMessage {
    /// Status poll returned peer statuses.
    StatusUpdate(Vec<PeerStatus>),
    /// A command completed successfully.
    CommandOk(String),
    /// A command failed with an error message.
    CommandError(String),
    /// Daemon is unreachable.
    Unreachable,
}

/// Convert a daemon client error into a [`DaemonMessage`].
///
/// Centralizes the boundary between typed errors and UI-facing
/// string messages. `NotRunning` maps to `Unreachable`; all other
/// errors are displayed as `CommandError`.
fn error_to_message(err: &client::DaemonClientError) -> DaemonMessage {
    if err.is_not_running() {
        DaemonMessage::Unreachable
    } else {
        DaemonMessage::CommandError(err.to_string())
    }
}

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

    // Channel for receiving daemon responses from background tasks.
    let (daemon_tx, mut daemon_rx) = mpsc::unbounded_channel::<DaemonMessage>();

    // Guard to prevent multiple concurrent status polls.
    let poll_in_flight = Arc::new(AtomicBool::new(false));

    // Track background tasks for clean shutdown.
    let mut tasks = JoinSet::new();

    while state.running {
        // Drain daemon messages (non-blocking).
        while let Ok(msg) = daemon_rx.try_recv() {
            let actions: Vec<Action> = match msg {
                DaemonMessage::StatusUpdate(peers) => vec![Action::UpdatePeers(peers)],
                DaemonMessage::CommandOk(msg) => vec![Action::DaemonOk(msg)],
                DaemonMessage::CommandError(msg) => vec![Action::DaemonError(msg)],
                DaemonMessage::Unreachable => vec![
                    Action::DaemonConnectivityChanged(false),
                    Action::DaemonError("daemon is not running".into()),
                ],
            };
            for action in &actions {
                dispatch_all(
                    &mut state,
                    action,
                    &mut components,
                    &mut tab_bar,
                    &mut status_bar,
                );
            }
        }

        // Clear expired feedback.
        state.clear_expired_feedback();

        // Render.
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
                    dispatch_all(
                        &mut state,
                        action,
                        &mut components,
                        &mut tab_bar,
                        &mut status_bar,
                    );
                    maybe_spawn_command(action, &daemon_tx, &mut tasks);
                }
            }
            Some(AppEvent::Tick) => {
                spawn_status_poll(&daemon_tx, &poll_in_flight, &mut tasks);
            }
            None => break,
        }
    }

    // Abort remaining background tasks on exit.
    tasks.abort_all();

    Ok(())
}

/// Dispatch an action to `AppState` and all components.
fn dispatch_all(
    state: &mut AppState,
    action: &Action,
    components: &mut [Box<dyn Component>],
    tab_bar: &mut TabBarComponent,
    status_bar: &mut StatusBarComponent,
) {
    state.dispatch(action);
    for comp in components.iter_mut() {
        comp.update(action, state);
    }
    tab_bar.update(action, state);
    status_bar.update(action, state);
}

/// Handle global key events that apply regardless of which component
/// is focused.
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

/// Spawn a background status poll if none is already in-flight.
fn spawn_status_poll(
    tx: &mpsc::UnboundedSender<DaemonMessage>,
    in_flight: &Arc<AtomicBool>,
    tasks: &mut JoinSet<()>,
) {
    if in_flight.swap(true, Ordering::SeqCst) {
        return;
    }

    let tx = tx.clone();
    let in_flight = Arc::clone(in_flight);

    tasks.spawn(async move {
        let msg = match client::send_command(&DaemonCommand::Status).await {
            Ok(DaemonResponse::Status(peers)) => DaemonMessage::StatusUpdate(peers),
            Err(e) => error_to_message(&e),
            Ok(_) => {
                in_flight.store(false, Ordering::SeqCst);
                return;
            }
        };
        let _ = tx.send(msg);
        in_flight.store(false, Ordering::SeqCst);
    });
}

/// If the action is a peer command, spawn a background daemon task.
fn maybe_spawn_command(
    action: &Action,
    tx: &mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
) {
    let (cmd, description) = match action {
        Action::ConnectPeer(name) => (
            DaemonCommand::Up {
                peer_name: Some(name.clone()),
                backend: BackendKind::Boringtun,
            },
            format!("Brought up: {name}"),
        ),
        Action::DisconnectPeer(name) => (
            DaemonCommand::Down {
                peer_name: Some(name.clone()),
            },
            format!("Tore down: {name}"),
        ),
        Action::CyclePeerBackend(name) => (
            DaemonCommand::SwitchBackend {
                peer_name: name.clone(),
                backend: BackendKind::Neptun, // TODO: cycle through available backends
            },
            format!("Switched backend: {name}"),
        ),
        _ => return,
    };

    let tx = tx.clone();
    tasks.spawn(async move {
        let msg = match client::send_command(&cmd).await {
            Ok(DaemonResponse::Ok) => DaemonMessage::CommandOk(description),
            Ok(DaemonResponse::Error(e)) => DaemonMessage::CommandError(e),
            Err(e) => error_to_message(&e),
            Ok(DaemonResponse::Status(_)) => return,
        };
        let _ = tx.send(msg);
    });
}
