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
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use ferro_wg_core::client;
use ferro_wg_core::config::AppConfig;
use ferro_wg_core::error::BackendKind;
use ferro_wg_core::ipc::{DaemonCommand, DaemonResponse, PeerStatus};
use ferro_wg_tui_components::connection_bar::{CONNECTION_BAR_HEIGHT, MIN_USEFUL_WIDTH};
use ferro_wg_tui_components::status_bar::STATUS_BAR_HEIGHT;
use ferro_wg_tui_components::tab_bar::TAB_BAR_HEIGHT;
use ferro_wg_tui_components::{
    CompareComponent, ConfigComponent, ConnectionBarComponent, LogsComponent, OverviewComponent,
    PeersComponent, StatusBarComponent, StatusComponent, TabBarComponent,
};
use ferro_wg_tui_core::{Action, AppState, Component, InputMode, Tab};
use tracing::warn;

use event::{AppEvent, EventHandler};

/// Minimum rows reserved for the main content area.
///
/// Policy constant: ensures the content pane is never squeezed to zero rows
/// when the connection bar is shown.
const MIN_CONTENT_HEIGHT: u16 = 1;

/// Minimum terminal height at which the connection bar is shown.
///
/// Derived directly from each component's own height measurement so this
/// threshold automatically tracks any layout changes to surrounding
/// components.  Below this value the bar is suppressed so the content area
/// never collapses below [`MIN_CONTENT_HEIGHT`].
const MIN_HEIGHT_FOR_CONNECTION_BAR: u16 =
    TAB_BAR_HEIGHT + CONNECTION_BAR_HEIGHT + MIN_CONTENT_HEIGHT + STATUS_BAR_HEIGHT;

/// Minimum terminal width at which the connection bar is shown.
///
/// Delegates to [`MIN_USEFUL_WIDTH`] so this threshold stays in sync with any
/// changes to the bar's prefix or indicator strings.
const MIN_WIDTH_FOR_CONNECTION_BAR: u16 = MIN_USEFUL_WIDTH;

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

/// All TUI components, grouped to reduce function parameter counts.
///
/// Components are split into tab content (`tabs`) and fixed chrome
/// (`tab_bar`, `status_bar`, `connection_bar`).  The index into `tabs`
/// corresponds to [`Tab::index()`](ferro_wg_tui_core::Tab).
struct ComponentBundle {
    /// Tab content components in tab-index order.
    tabs: Vec<Box<dyn Component>>,
    /// Fixed chrome: top tab navigation bar.
    tab_bar: TabBarComponent,
    /// Fixed chrome: bottom status / search bar.
    status_bar: StatusBarComponent,
    /// Optional chrome: multi-connection selector bar.
    connection_bar: ConnectionBarComponent,
}

impl ComponentBundle {
    fn new() -> Self {
        Self {
            tabs: vec![
                Box::new(OverviewComponent::new()), // Tab::Overview (index 0)
                Box::new(StatusComponent::new()),   // Tab::Status (index 1)
                Box::new(PeersComponent::new()),    // Tab::Peers (index 2)
                Box::new(CompareComponent::new()),  // Tab::Compare (index 3)
                Box::new(ConfigComponent::new()),   // Tab::Config (index 4)
                Box::new(LogsComponent::new()),     // Tab::Logs (index 5)
            ],
            tab_bar: TabBarComponent::new(),
            status_bar: StatusBarComponent::new(),
            connection_bar: ConnectionBarComponent::new(),
        }
    }
}

/// Process pending daemon messages and dispatch actions.
fn handle_daemon_messages(
    daemon_rx: &mut mpsc::UnboundedReceiver<DaemonMessage>,
    state: &mut AppState,
    bundle: &mut ComponentBundle,
) {
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
            dispatch_all(state, action, bundle);
        }
    }
}

/// Render the UI to the terminal.
fn render_ui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &AppState,
    bundle: &mut ComponentBundle,
    chunks: &[ratatui::layout::Rect],
    show_bar: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    terminal.draw(|frame| {
        bundle.tab_bar.render(frame, chunks[0], false, state);
        if show_bar {
            bundle.connection_bar.render(frame, chunks[1], false, state);
        }
        bundle.tabs[state.active_tab.index()].render(frame, chunks[2], true, state);
        bundle.status_bar.render(frame, chunks[3], false, state);
    })?;
    Ok(())
}

/// Handle a key event: resolve it to an action, dispatch, and spawn any command.
fn handle_key_event(
    key: KeyEvent,
    state: &mut AppState,
    bundle: &mut ComponentBundle,
    daemon_tx: &mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
) {
    let action = if state.input_mode == InputMode::Search {
        bundle.status_bar.handle_key(key, state)
    } else {
        handle_global_key(key)
            .or_else(|| bundle.connection_bar.handle_key(key, state))
            .or_else(|| bundle.tabs[state.active_tab.index()].handle_key(key, state))
    };
    if let Some(ref action) = action {
        dispatch_all(state, action, bundle);
        maybe_spawn_command(action, daemon_tx, tasks);
    }
}

/// Compute the top-level layout and connection-bar visibility from the terminal area.
fn compute_layout(
    area: ratatui::layout::Rect,
    connections: usize,
) -> (Vec<ratatui::layout::Rect>, bool) {
    let show_bar = connections > 1
        && area.height >= MIN_HEIGHT_FOR_CONNECTION_BAR
        && area.width >= MIN_WIDTH_FOR_CONNECTION_BAR;
    let bar_height = u16::from(show_bar);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(TAB_BAR_HEIGHT),
            Constraint::Length(bar_height),
            Constraint::Min(0),
            Constraint::Length(STATUS_BAR_HEIGHT),
        ])
        .split(area)
        .to_vec();
    debug_assert_layout(&chunks, area, show_bar);
    (chunks, show_bar)
}

/// Spawn a background task that streams structured log entries from the daemon.
fn spawn_log_stream(tasks: &mut JoinSet<()>, state_ref: &AppState) {
    let log_entries = Arc::clone(&state_ref.log_entries);
    tasks.spawn(async move {
        let mut rx = match client::stream_logs().await {
            Ok(rx) => rx,
            Err(e) => {
                warn!("Failed to stream logs: {e}");
                return;
            }
        };
        while let Some(entry) = rx.recv().await {
            let Ok(mut buf) = log_entries.lock() else {
                warn!("Log buffer mutex poisoned, skipping log append");
                continue;
            };
            if buf.len() == buf.capacity() {
                buf.pop_front();
            }
            buf.push_back(entry);
        }
    });
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
/// An empty `AppConfig` is valid — the TUI renders a placeholder.
///
/// # Errors
///
/// Returns an error if:
/// - The terminal cannot be put into raw mode (`crossterm::terminal::enable_raw_mode`)
/// - Entering or leaving the alternate screen fails (`crossterm::execute!`)
/// - The [`Terminal`] backend cannot be created or cleared
/// - A terminal draw call fails (e.g. the underlying writer returns an I/O error)
/// - The event handler fails to read a terminal event
pub async fn run(app_config: AppConfig) -> Result<(), Box<dyn std::error::Error>> {
    // Setup terminal.
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Run the event loop, then restore terminal regardless of outcome.
    let result = event_loop(&mut terminal, app_config).await;

    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    result
}

/// Drive the TUI event loop until the user quits.
async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app_config: AppConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut state = AppState::new(app_config);
    let mut bundle = ComponentBundle::new();
    let mut events = EventHandler::new(Duration::from_millis(250));
    let (daemon_tx, mut daemon_rx) = mpsc::unbounded_channel::<DaemonMessage>();
    let poll_in_flight = Arc::new(AtomicBool::new(false));
    let mut tasks = JoinSet::new();

    spawn_log_stream(&mut tasks, &state);

    while state.running {
        handle_daemon_messages(&mut daemon_rx, &mut state, &mut bundle);
        state.clear_expired_feedback();

        let size = terminal.size()?;
        let area = Rect::new(0, 0, size.width, size.height);
        let (chunks, show_bar) = compute_layout(area, state.connections.len());
        render_ui(terminal, &state, &mut bundle, &chunks, show_bar)?;

        match events.next().await {
            Some(AppEvent::Key(key)) => {
                handle_key_event(key, &mut state, &mut bundle, &daemon_tx, &mut tasks);
            }
            Some(AppEvent::Tick) => spawn_status_poll(&daemon_tx, &poll_in_flight, &mut tasks),
            None => break,
        }
    }
    tasks.abort_all();
    Ok(())
}

/// Assert that the top-level layout chunks have the expected dimensions.
///
/// Only active in debug builds (`debug_assert!`). Called once per frame
/// immediately after the layout split so mismatches surface in testing.
fn debug_assert_layout(
    chunks: &[ratatui::layout::Rect],
    area: ratatui::layout::Rect,
    show_bar: bool,
) {
    debug_assert_eq!(
        chunks.len(),
        4,
        "top-level layout must yield exactly 4 chunks"
    );
    // Length constraints are satisfied in full when the terminal has
    // enough rows for the two fixed chrome bands.
    if area.height >= TAB_BAR_HEIGHT + STATUS_BAR_HEIGHT {
        debug_assert_eq!(
            chunks[0].height, TAB_BAR_HEIGHT,
            "tab bar chunk height should be {TAB_BAR_HEIGHT}, got {}",
            chunks[0].height
        );
        debug_assert_eq!(
            chunks[3].height, STATUS_BAR_HEIGHT,
            "status bar chunk height should be {STATUS_BAR_HEIGHT}, got {}",
            chunks[3].height
        );
    }
    if show_bar {
        debug_assert_eq!(
            chunks[1].height, CONNECTION_BAR_HEIGHT,
            "connection bar chunk height should be {CONNECTION_BAR_HEIGHT}, got {}",
            chunks[1].height
        );
    }
}

/// Dispatch an action to `AppState` and all components.
fn dispatch_all(state: &mut AppState, action: &Action, bundle: &mut ComponentBundle) {
    state.dispatch(action);
    for comp in &mut bundle.tabs {
        comp.update(action, state);
    }
    bundle.tab_bar.update(action, state);
    bundle.status_bar.update(action, state);
}

/// Handle global key events that apply regardless of which component
/// is focused.
fn handle_global_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(Action::Quit),
        KeyCode::Tab | KeyCode::Right => Some(Action::NextTab),
        KeyCode::BackTab | KeyCode::Left => Some(Action::PrevTab),
        // Tab shortcuts shift by one to accommodate Overview at index 0.
        KeyCode::Char('1') => Some(Action::SelectTab(Tab::Overview)),
        KeyCode::Char('2') => Some(Action::SelectTab(Tab::Status)),
        KeyCode::Char('3') => Some(Action::SelectTab(Tab::Peers)),
        KeyCode::Char('4') => Some(Action::SelectTab(Tab::Compare)),
        KeyCode::Char('5') => Some(Action::SelectTab(Tab::Config)),
        KeyCode::Char('6') => Some(Action::SelectTab(Tab::Logs)),
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
                connection_name: Some(name.clone()),
                backend: BackendKind::Boringtun,
            },
            format!("Brought up: {name}"),
        ),
        Action::DisconnectPeer(name) => (
            DaemonCommand::Down {
                connection_name: Some(name.clone()),
            },
            format!("Tore down: {name}"),
        ),
        Action::CyclePeerBackend(name) => (
            DaemonCommand::SwitchBackend {
                connection_name: name.clone(),
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
            Ok(DaemonResponse::LogEntry(_)) => {
                warn!("Received unexpected LogEntry response for command");
                return;
            }
            Err(e) => error_to_message(&e),
            Ok(DaemonResponse::Status(_)) => return,
        };
        let _ = tx.send(msg);
    });
}
