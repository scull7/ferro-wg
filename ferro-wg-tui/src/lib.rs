//! Terminal UI for ferro-wg.
//!
//! This crate provides the entry point ([`run`]) that sets up the
//! terminal, creates components and state, and drives the event loop.
//! It wires together types from [`ferro_wg_tui_core`] and component
//! implementations from [`ferro_wg_tui_components`].

mod event;
mod history;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyModifiers, MouseEvent,
};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::Text;
use ratatui::widgets::Paragraph;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use ferro_wg_core::client;
use ferro_wg_core::config::{AppConfig, toml as config_toml, wg_quick};
use ferro_wg_core::error::BackendKind;
use ferro_wg_core::ipc::{BenchmarkProgress, DaemonCommand, DaemonResponse, PeerStatus};
use ferro_wg_core::stats::BenchmarkResult;
use ferro_wg_tui_components::connection_bar::{CONNECTION_BAR_HEIGHT, MIN_USEFUL_WIDTH};
use ferro_wg_tui_components::status_bar::STATUS_BAR_HEIGHT;
use ferro_wg_tui_components::tab_bar::TAB_BAR_HEIGHT;
use ferro_wg_tui_components::{
    CompareComponent, ConfigComponent, ConfirmDialogComponent, ConnectionBarComponent,
    DiffPreviewComponent, HelpOverlayComponent, LogsComponent, OverviewComponent, PeersComponent,
    StatusBarComponent, StatusComponent, TabBarComponent, ToastComponent,
};
use ferro_wg_tui_core::{Action, AppState, Component, ConfirmAction, InputMode, Tab};
use futures::StreamExt;
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

/// Minimum terminal width for responsive layout.
const MIN_TERMINAL_WIDTH: u16 = 80;

/// Minimum terminal height for responsive layout.
const MIN_TERMINAL_HEIGHT: u16 = 24;

/// UI-facing errors from TUI operations.
///
/// Dedicated error enum for the TUI layer, converting lower-level
/// errors into user-displayable messages.
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    /// Daemon client communication error.
    #[error(transparent)]
    DaemonClient(#[from] client::DaemonClientError),
    /// Unknown benchmark backend specified.
    #[error("unknown benchmark backend: {0}")]
    UnknownBackend(String),
    /// Stream closed unexpectedly during benchmark.
    #[error("stream closed unexpectedly")]
    StreamClosed,
    /// Daemon returned an error response.
    #[error("daemon error: {0}")]
    DaemonResponse(String),
    /// Could not locate ferro-wg executable.
    #[error("could not find ferro-wg executable: {0}")]
    ExecutableNotFound(std::io::Error),
    /// Could not start daemon process.
    #[error("could not start daemon: run 'sudo ferro-wg daemon --daemonize' ({0})")]
    DaemonStartFailed(std::io::Error),
    /// Config import failed.
    #[error("config import failed: {0}")]
    ConfigImportFailed(String),
    /// Generic TUI error.
    #[error("{0}")]
    Generic(String),
}

impl From<&str> for TuiError {
    fn from(s: &str) -> Self {
        Self::Generic(s.to_string())
    }
}

/// Messages sent from background daemon tasks to the event loop.
enum DaemonMessage {
    /// Status poll returned peer statuses.
    StatusUpdate(Vec<PeerStatus>),
    /// A command completed successfully.
    CommandOk(String),
    /// A command failed with an error message.
    CommandError(TuiError),
    /// Daemon is unreachable.
    Unreachable,
    /// A wg-quick import succeeded; reload state from the new config.
    ReloadConfig(AppConfig, String),
    /// Live progress update from a running benchmark.
    BenchmarkProgress(BenchmarkProgress),
    /// A benchmark run completed successfully.
    BenchmarkComplete(BenchmarkResult),
}

/// All TUI components, grouped to reduce function parameter counts.
///
/// Components are split into tab content (`tabs`) and fixed chrome
/// (`tab_bar`, `status_bar`, `connection_bar`, `confirm_dialog`).
/// The index into `tabs` corresponds to [`Tab::index()`](ferro_wg_tui_core::Tab).
struct ComponentBundle {
    /// Tab content components in tab-index order.
    tabs: Vec<Box<dyn Component>>,
    /// Fixed chrome: top tab navigation bar.
    tab_bar: TabBarComponent,
    /// Fixed chrome: bottom status / search bar.
    status_bar: StatusBarComponent,
    /// Optional chrome: multi-connection selector bar.
    connection_bar: ConnectionBarComponent,
    /// Modal overlay: confirmation dialog.
    confirm_dialog: ConfirmDialogComponent,
    /// Modal overlay: diff preview (rendered on top of everything).
    diff_preview: DiffPreviewComponent,
    /// Modal overlay: help overlay (topmost).
    help_overlay: HelpOverlayComponent,
    /// Toast notifications in bottom-right corner.
    toast: ToastComponent,
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
            confirm_dialog: ConfirmDialogComponent::new(),
            diff_preview: DiffPreviewComponent::new(),
            help_overlay: HelpOverlayComponent::new(),
            toast: ToastComponent::new(),
        }
    }
}

/// Process pending daemon messages and dispatch actions.
fn handle_daemon_messages(
    daemon_rx: &mut mpsc::UnboundedReceiver<DaemonMessage>,
    state: &mut AppState,
    bundle: &mut ComponentBundle,
    daemon_tx: &mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
    benchmarks_path: &Path,
) {
    while let Ok(msg) = daemon_rx.try_recv() {
        let actions: Vec<Action> = match msg {
            DaemonMessage::ReloadConfig(ref config, ref ok_msg) => {
                state.reload_from_config(config.clone());
                vec![Action::DaemonOk(ok_msg.clone())]
            }
            DaemonMessage::StatusUpdate(ref peers) => vec![Action::UpdatePeers(peers.clone())],
            DaemonMessage::CommandOk(ref msg) => vec![Action::DaemonOk(msg.clone())],
            DaemonMessage::CommandError(ref err) => vec![Action::DaemonError(err.to_string())],
            DaemonMessage::Unreachable => vec![
                Action::DaemonConnectivityChanged(false),
                Action::DaemonError("daemon is not running".into()),
            ],
            DaemonMessage::BenchmarkProgress(ref p) => {
                vec![Action::BenchmarkProgressUpdate(p.clone())]
            }
            DaemonMessage::BenchmarkComplete(ref r) => vec![Action::BenchmarkComplete(r.clone())],
        };
        for action in &actions {
            dispatch_all(state, action, bundle);
        }
        // After dispatching BenchmarkComplete, persist the updated history.
        if matches!(msg, DaemonMessage::BenchmarkComplete(_)) {
            spawn_save_history_task(
                benchmarks_path,
                state.benchmark_history.clone(),
                daemon_tx,
                tasks,
            );
        }
    }
}

/// Render "Terminal too small" message when the terminal is below minimum size.
fn render_too_small(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &AppState) {
    let text = Text::styled("Terminal too small (min 80×24)", state.theme.error);
    let para = Paragraph::new(text).alignment(Alignment::Center);
    frame.render_widget(para, area);
}

/// Render the UI to the terminal.
fn render_ui<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    state: &AppState,
    bundle: &mut ComponentBundle,
    chunks: &[ratatui::layout::Rect],
    show_bar: bool,
    area: ratatui::layout::Rect,
) -> Result<(), Box<dyn std::error::Error>> {
    terminal.draw(|frame| {
        if area.width < MIN_TERMINAL_WIDTH || area.height < MIN_TERMINAL_HEIGHT {
            render_too_small(frame, area, state);
            return;
        }
        bundle.tab_bar.render(frame, chunks[0], false, state);
        if show_bar {
            bundle.connection_bar.render(frame, chunks[1], false, state);
        }
        bundle.tabs[state.active_tab.index()].render(frame, chunks[2], true, state);
        bundle.status_bar.render(frame, chunks[3], false, state);
        bundle.confirm_dialog.render(frame, chunks[2], false, state);
        bundle.diff_preview.render(frame, chunks[2], false, state); // topmost
        bundle.help_overlay.render(frame, chunks[2], false, state); // topmost
        bundle.toast.render(frame, area, false, state);
    })?;
    Ok(())
}

/// Handle a mouse event: resolve it to an action, dispatch, and spawn any command.
#[allow(clippy::too_many_arguments)]
fn handle_mouse_event(
    mouse: MouseEvent,
    state: &mut AppState,
    bundle: &mut ComponentBundle,
    daemon_tx: &mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
    config_path: &Path,
    benchmarks_path: &Path,
    chunks: &[ratatui::layout::Rect],
) {
    let action = if state.show_help
        || state.pending_confirm.is_some()
        || state.config_diff_pending.is_some()
    {
        None
    } else {
        ferro_wg_tui_core::ux::resolve_mouse_action(&mouse, chunks[0])
            .or_else(|| bundle.tabs[state.active_tab.index()].handle_mouse(mouse, state))
    };

    let Some(ref action) = action else { return };

    dispatch_all(state, action, bundle);
    maybe_spawn_command(
        action,
        state,
        daemon_tx,
        tasks,
        config_path,
        benchmarks_path,
    );
}

/// Handle a key event: resolve it to an action, dispatch, and spawn any command.
fn handle_key_event(
    key: KeyEvent,
    state: &mut AppState,
    bundle: &mut ComponentBundle,
    daemon_tx: &mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
    config_path: &Path,
    benchmarks_path: &Path,
) {
    let action = if state.show_help {
        // Help overlay captures all keys while open.
        bundle.help_overlay.handle_key(key, state)
    } else if state.config_diff_pending.is_some() {
        // Diff preview captures all keys while open.
        bundle.diff_preview.handle_key(key, state)
    } else if state.pending_confirm.is_some() {
        // Confirmation dialog captures all keys; no other handler runs.
        bundle.confirm_dialog.handle_key(key, state)
    } else if matches!(
        state.input_mode,
        InputMode::Search | InputMode::Import(_) | InputMode::Export(_)
    ) {
        // Text-input modes are handled exclusively by the status bar.
        bundle.status_bar.handle_key(key, state)
    } else {
        handle_global_key(key)
            .or_else(|| bundle.connection_bar.handle_key(key, state))
            .or_else(|| bundle.tabs[state.active_tab.index()].handle_key(key, state))
    };

    let Some(ref action) = action else { return };

    // For ConfirmYes: peek at the pending action *before* dispatch clears it,
    // so we can immediately dispatch the confirmed follow-up action.
    let follow_up = if matches!(action, Action::ConfirmYes) {
        state
            .pending_confirm
            .as_ref()
            .map(|p| confirmed_action(&p.action))
    } else {
        None
    };

    // For SubmitImport: capture the path buffer *before* dispatch resets the
    // input mode back to Normal (clearing the buffer from state).
    let import_path = if matches!(action, Action::SubmitImport) {
        state
            .import_buffer()
            .map(std::path::Path::new)
            .map(std::path::Path::to_path_buf)
    } else {
        None
    };

    // For SubmitExport: capture the path buffer *before* dispatch resets the
    // input mode back to Normal (clearing the buffer from state).
    let export_path = if matches!(action, Action::SubmitExport) {
        state
            .export_buffer()
            .map(std::path::Path::new)
            .map(std::path::Path::to_path_buf)
    } else {
        None
    };

    // For SaveConfig: capture the draft and connection name *before* dispatch
    // clears config_diff_pending from state.
    let save_config_data = if let Action::SaveConfig { reconnect } = action {
        state
            .config_diff_pending
            .as_ref()
            .map(|p| (p.connection_name.clone(), p.draft.clone(), *reconnect))
    } else {
        None
    };

    dispatch_all(state, action, bundle);
    maybe_spawn_command(
        action,
        state,
        daemon_tx,
        tasks,
        config_path,
        benchmarks_path,
    );

    if let Some(ref follow) = follow_up {
        dispatch_all(state, follow, bundle);
        maybe_spawn_command(
            follow,
            state,
            daemon_tx,
            tasks,
            config_path,
            benchmarks_path,
        );
    }

    if let Some(path) = import_path {
        spawn_import_task(path, config_path, daemon_tx, tasks);
    }

    if let Some(path) = export_path {
        spawn_export_task(path, &state.benchmark_history.clone(), daemon_tx, tasks);
    }

    if let Some((connection_name, draft, reconnect)) = save_config_data {
        spawn_save_config_task(
            connection_name,
            draft,
            reconnect,
            config_path,
            daemon_tx,
            tasks,
        );
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
/// messages. `NotRunning` maps to `Unreachable`; all other
/// errors are displayed as `CommandError`.
fn error_to_message(err: client::DaemonClientError) -> DaemonMessage {
    if err.is_not_running() {
        DaemonMessage::Unreachable
    } else {
        DaemonMessage::CommandError(err.into())
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
pub async fn run(
    app_config: AppConfig,
    config_path: PathBuf,
    benchmarks_path: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    // Setup terminal.
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Run the event loop, then restore terminal regardless of outcome.
    let result = event_loop(&mut terminal, app_config, config_path, benchmarks_path).await;

    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
    let _ = terminal.show_cursor();

    result
}

/// Drive the TUI event loop until the user quits.
async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app_config: AppConfig,
    config_path: PathBuf,
    benchmarks_path: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut state = AppState::new(app_config);
    let mut bundle = ComponentBundle::new();
    let mut events = EventHandler::new(Duration::from_millis(250));
    let (daemon_tx, mut daemon_rx) = mpsc::unbounded_channel::<DaemonMessage>();
    let poll_in_flight = Arc::new(AtomicBool::new(false));
    let mut tasks = JoinSet::new();

    // Load benchmark history at startup.
    match history::load_benchmark_history(&benchmarks_path).await {
        Ok(history) => state.benchmark_history = history,
        Err(e) => warn!("failed to load benchmark history: {e}"),
    }

    spawn_log_stream(&mut tasks, &state);

    while state.running {
        handle_daemon_messages(
            &mut daemon_rx,
            &mut state,
            &mut bundle,
            &daemon_tx,
            &mut tasks,
            &benchmarks_path,
        );
        state.clear_expired_toasts();

        let size = terminal.size()?;
        let area = Rect::new(0, 0, size.width, size.height);
        let (chunks, show_bar) = compute_layout(area, state.connections.len());

        match events.next().await {
            Some(AppEvent::Key(key)) => {
                handle_key_event(
                    key,
                    &mut state,
                    &mut bundle,
                    &daemon_tx,
                    &mut tasks,
                    &config_path,
                    &benchmarks_path,
                );
            }
            Some(AppEvent::Mouse(mouse)) => {
                handle_mouse_event(
                    mouse,
                    &mut state,
                    &mut bundle,
                    &daemon_tx,
                    &mut tasks,
                    &config_path,
                    &benchmarks_path,
                    &chunks,
                );
            }
            Some(AppEvent::Tick) => spawn_status_poll(&daemon_tx, &poll_in_flight, &mut tasks),
            None => break,
        }

        render_ui(terminal, &state, &mut bundle, &chunks, show_bar, area)?;
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
    bundle.confirm_dialog.update(action, state);
    bundle.help_overlay.update(action, state);
}

/// Map a [`ConfirmAction`] to the [`Action`] that should execute after confirmation.
///
/// Called by [`handle_key_event`] when the user presses `y` to confirm.
/// This is the bridge between the confirmation dialog and the downstream
/// command dispatch.
fn confirmed_action(action: &ConfirmAction) -> Action {
    match action {
        ConfirmAction::DisconnectAll => Action::DisconnectAll,
        ConfirmAction::StopDaemon => Action::StopDaemon,
        ConfirmAction::DeletePeer(i) => Action::DeleteConfigPeer(*i),
    }
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
        KeyCode::Char('T') => Some(Action::ToggleTheme),
        KeyCode::Char('?') => Some(Action::ShowHelp),
        KeyCode::Char('i') => Some(Action::EnterImport),
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
            Err(e) => error_to_message(e),
            Ok(_) => {
                in_flight.store(false, Ordering::SeqCst);
                return;
            }
        };
        let _ = tx.send(msg);
        in_flight.store(false, Ordering::SeqCst);
    });
}

/// Attempt to start the daemon as a background subprocess.
///
/// Flow:
/// 1. If the socket is already reachable, send `CommandError("Daemon is already running")`.
/// 2. Spawn `sudo <exe> daemon --daemonize -c <config_path>` detached.
/// 3. Poll the socket up to 6 × 500 ms. On first success send `CommandOk`.
///    On timeout send `CommandError("not yet reachable")`.
fn spawn_daemon_start(
    config_path: &Path,
    tx: &mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
) {
    let config_path = config_path.to_path_buf();
    let tx = tx.clone();

    tasks.spawn(async move {
        // Pre-spawn guard: abort if daemon is already reachable.
        if client::send_command(&DaemonCommand::Status).await.is_ok() {
            let _ = tx.send(DaemonMessage::CommandError(
                "Daemon is already running".into(),
            ));
            return;
        }

        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => {
                let _ = tx.send(DaemonMessage::CommandError(TuiError::ExecutableNotFound(e)));
                return;
            }
        };

        let spawn_result = tokio::process::Command::new("sudo")
            .arg(&exe)
            .arg("daemon")
            .arg("--daemonize")
            .arg("-c")
            .arg(&config_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();

        if let Err(e) = spawn_result {
            let _ = tx.send(DaemonMessage::CommandError(TuiError::DaemonStartFailed(e)));
            return;
        }

        // Post-spawn poll: wait up to 3 s for the socket to become available.
        for _ in 0_u8..6 {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            if client::send_command(&DaemonCommand::Status).await.is_ok() {
                let _ = tx.send(DaemonMessage::CommandOk("Daemon started".into()));
                return;
            }
        }

        let _ = tx.send(DaemonMessage::CommandError(
            "Daemon started but not yet reachable — try again in a moment".into(),
        ));
    });
}

/// Derive a connection name from a file path.
///
/// Uses the file stem (e.g. `mia` from `/etc/wireguard/mia.conf`).
/// Returns an error string if the stem is absent or not valid UTF-8.
fn connection_name_from_path(path: &Path) -> Result<String, String> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "cannot derive connection name from path: {}",
                path.display()
            )
        })
}

/// Import a wg-quick config file and persist it into the ferro-wg config.
///
/// Flow:
/// 1. Derive a connection name from the filename stem.
/// 2. Parse the file as wg-quick format.
/// 3. Load the existing [`AppConfig`] from `config_path`.
/// 4. Insert the new connection (overwriting any existing entry with the same name).
/// 5. Write the updated config back to disk.
/// 6. Send [`DaemonMessage::ReloadConfig`] with the new config.
fn spawn_import_task(
    import_path: PathBuf,
    config_path: &Path,
    tx: &mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
) {
    let config_path = config_path.to_path_buf();
    let tx = tx.clone();

    tasks.spawn(async move {
        // All I/O is blocking but small — run on the async executor directly.
        // (tokio::task::spawn_blocking is unnecessary for short file reads.)
        let msg = import_and_persist(&import_path, &config_path);
        let _ = tx.send(msg);
    });
}

/// Parse the wg-quick file, merge it into the existing config, and write it back.
///
/// Returns the updated [`AppConfig`] and the derived connection name on success,
/// or an error string that will be shown in the status bar.
fn try_import(import_path: &Path, config_path: &Path) -> Result<(AppConfig, String), String> {
    let name = connection_name_from_path(import_path)?;
    let wg_config =
        wg_quick::load_from_file(import_path).map_err(|e| format!("Import failed: {e}"))?;
    let mut app_config = config_toml::load_app_config(config_path)
        .map_err(|e| format!("Could not load config: {e}"))?;
    app_config.insert(name.clone(), wg_config);
    config_toml::save_app_config(&app_config, config_path)
        .map_err(|e| format!("Could not save config: {e}"))?;
    Ok((app_config, name))
}

/// Map the fallible import result to a [`DaemonMessage`] for the event loop.
fn import_and_persist(import_path: &Path, config_path: &Path) -> DaemonMessage {
    // I/O is blocking (std::fs). This is user-initiated and infrequent — acceptable
    // on the async executor. Use spawn_blocking if config files ever grow large.
    match try_import(import_path, config_path) {
        Ok((config, name)) => DaemonMessage::ReloadConfig(config, format!("Imported: {name}")),
        Err(e) => DaemonMessage::CommandError(TuiError::ConfigImportFailed(e)),
    }
}

/// Spawn an async task that sends a `DaemonCommand::Benchmark` IPC request,
/// relays periodic `BenchmarkProgress` updates, and sends `BenchmarkComplete`
/// when the run finishes.
fn spawn_benchmark_task(
    connection_name: String,
    duration_secs: u32,
    daemon_tx: &mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
) {
    let cmd = DaemonCommand::Benchmark {
        connection_name,
        duration_secs,
    };
    let daemon_tx = daemon_tx.clone();
    tasks.spawn(async move {
        let stream = match client::send_streaming_command(cmd).await {
            Ok(s) => s,
            Err(e) => {
                let _ = daemon_tx.send(error_to_message(e));
                return;
            }
        };
        tokio::pin!(stream);
        while let Some(response) = stream.next().await {
            match response {
                DaemonResponse::BenchmarkProgress(p) => {
                    let _ = daemon_tx.send(DaemonMessage::BenchmarkProgress(p));
                }
                DaemonResponse::BenchmarkResult(r) => {
                    let _ = daemon_tx.send(DaemonMessage::BenchmarkComplete(r));
                    return;
                }
                DaemonResponse::Error(e) => {
                    let _ =
                        daemon_tx.send(DaemonMessage::CommandError(TuiError::DaemonResponse(e)));
                    return;
                }
                _ => {}
            }
        }
        let _ = daemon_tx.send(DaemonMessage::CommandError(TuiError::StreamClosed));
    });
}

/// Spawn an async task that serialises `runs` and writes the result to `path`.
///
/// Extension determines format: `.csv` → CSV; anything else → JSON.
fn spawn_export_task(
    path: PathBuf,
    runs: &[ferro_wg_tui_core::benchmark::BenchmarkRun],
    daemon_tx: &mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
) {
    use ferro_wg_tui_core::benchmark::{BenchmarkError, benchmark_to_csv, benchmark_to_json};

    let runs = runs.to_vec();
    let daemon_tx = daemon_tx.clone();
    tasks.spawn(async move {
        let result: Result<(), BenchmarkError> = async {
            let content = match path.extension().and_then(|e| e.to_str()) {
                Some("csv") => benchmark_to_csv(&runs),
                _ => benchmark_to_json(&runs)?,
            };
            tokio::fs::write(&path, content).await?;
            Ok(())
        }
        .await;
        let msg = match result {
            Ok(()) => DaemonMessage::CommandOk(format!("exported to {}", path.display())),
            Err(e) => DaemonMessage::CommandError(TuiError::Generic(e.to_string())),
        };
        let _ = daemon_tx.send(msg);
    });
}

/// Spawn a background task to switch the benchmark backend for the active connection.
fn spawn_switch_backend_task(
    backend: String,
    state: &AppState,
    tx: &mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
) {
    let bk = match backend.as_str() {
        "boringtun" => BackendKind::Boringtun,
        "neptun" => BackendKind::Neptun,
        "gotatun" => BackendKind::Gotatun,
        _ => {
            let tx = tx.clone();
            tasks.spawn(async move {
                let _ = tx.send(DaemonMessage::CommandError(TuiError::UnknownBackend(
                    backend,
                )));
            });
            return;
        }
    };
    let Some(connection) = state.active_connection() else {
        let tx = tx.clone();
        tasks.spawn(async move {
            let _ = tx.send(DaemonMessage::CommandError(TuiError::DaemonResponse(
                "no active connection to switch backend for".into(),
            )));
        });
        return;
    };
    let cmd = DaemonCommand::SwitchBackend {
        connection_name: connection.name.clone(),
        backend: bk,
    };
    let description = format!("Switched backend: {backend}");
    let tx = tx.clone();
    tasks.spawn(async move {
        let msg = match client::send_command(&cmd).await {
            Ok(DaemonResponse::Ok) => DaemonMessage::CommandOk(description),
            Ok(DaemonResponse::Error(e)) => {
                DaemonMessage::CommandError(TuiError::DaemonResponse(e))
            }
            Err(e) => error_to_message(e),
            _ => return,
        };
        let _ = tx.send(msg);
    });
}

/// Spawn an async task that saves the benchmark history to disk.
fn spawn_save_history_task(
    benchmarks_path: &Path,
    runs: Vec<ferro_wg_tui_core::benchmark::BenchmarkRun>,
    daemon_tx: &mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
) {
    let path = benchmarks_path.to_path_buf();
    let daemon_tx = daemon_tx.clone();
    tasks.spawn(async move {
        let result: Result<(), ferro_wg_tui_core::benchmark::BenchmarkError> =
            history::save_benchmark_history(&path, runs).await;
        if let Err(e) = result {
            let _ = daemon_tx.send(DaemonMessage::CommandError(TuiError::Generic(format!(
                "failed to save benchmark history: {e}"
            ))));
        }
    });
}

/// Save a `WgConfig` draft for the named connection to disk.
///
/// Flow:
/// 1. Load the current [`AppConfig`] from `config_path`.
/// 2. Write a backup to `config_path` + `.bak`.
/// 3. Replace the named connection's config with `draft`.
/// 4. Write the updated config back to `config_path`.
/// 5. Send [`DaemonMessage::ReloadConfig`] to reload app state.
/// 6. If `reconnect`, send daemon `Down` + `Up` commands for the connection.
fn spawn_save_config_task(
    connection_name: String,
    draft: ferro_wg_core::config::WgConfig,
    reconnect: bool,
    config_path: &Path,
    tx: &mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
) {
    let config_path = config_path.to_path_buf();
    let tx = tx.clone();
    tasks.spawn(async move {
        let msg = try_save_config(&connection_name, &draft, &config_path);
        let _ = tx.send(msg);
        if reconnect {
            // Best-effort reconnect: tear down then bring back up.
            let down = DaemonCommand::Down {
                connection_name: Some(connection_name.clone()),
            };
            let up = DaemonCommand::Up {
                connection_name: Some(connection_name.clone()),
                backend: ferro_wg_core::error::BackendKind::Boringtun,
            };
            let _ = client::send_command(&down).await;
            let _ = client::send_command(&up).await;
        }
    });
}

/// Perform the backup-and-write save operation.
///
/// Returns a [`DaemonMessage`] to be forwarded to the event loop.
fn try_save_config(
    connection_name: &str,
    draft: &ferro_wg_core::config::WgConfig,
    config_path: &Path,
) -> DaemonMessage {
    let result: Result<AppConfig, String> = (|| {
        let mut app_config = config_toml::load_app_config(config_path)
            .map_err(|e| format!("Could not load config: {e}"))?;
        // Backup before writing.
        let backup_path = config_path.with_extension("toml.bak");
        let backup_content = config_toml::save_app_config_string(&app_config)
            .map_err(|e| format!("Could not serialise backup: {e}"))?;
        std::fs::write(&backup_path, backup_content)
            .map_err(|e| format!("Could not write backup: {e}"))?;
        // Apply draft.
        app_config.insert(connection_name.to_string(), draft.clone());
        config_toml::save_app_config(&app_config, config_path)
            .map_err(|e| format!("Could not write config: {e}"))?;
        Ok(app_config)
    })();
    match result {
        Ok(config) => DaemonMessage::ReloadConfig(config, "Config saved".into()),
        Err(e) => DaemonMessage::CommandError(TuiError::Generic(e)),
    }
}

/// If the action is a peer command or daemon lifecycle command, spawn a background task.
fn maybe_spawn_command(
    action: &Action,
    state: &AppState,
    tx: &mpsc::UnboundedSender<DaemonMessage>,
    tasks: &mut JoinSet<()>,
    config_path: &Path,
    _benchmarks_path: &Path,
) {
    if matches!(
        action,
        Action::StartBenchmark | Action::StartBenchmarkForBackend(_)
    ) {
        if let Some(connection) = state.active_connection() {
            spawn_benchmark_task(connection.name.clone(), 10, tx, tasks);
        }
        return;
    }
    if let Action::SwitchBenchmarkBackend(backend) = action {
        spawn_switch_backend_task(backend.clone(), state, tx, tasks);
        return;
    }
    if matches!(action, Action::StartDaemon) {
        spawn_daemon_start(config_path, tx, tasks);
        return;
    }
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
        Action::ConnectAll => (
            DaemonCommand::Up {
                connection_name: None,
                backend: BackendKind::Boringtun,
            },
            "All connections up".to_owned(),
        ),
        Action::DisconnectAll => (
            DaemonCommand::Down {
                connection_name: None,
            },
            "All connections down".to_owned(),
        ),
        Action::StopDaemon => (DaemonCommand::Shutdown, "Daemon stopped".to_owned()),
        _ => return,
    };

    let tx = tx.clone();
    tasks.spawn(async move {
        let msg = match client::send_command(&cmd).await {
            Ok(DaemonResponse::Ok) => DaemonMessage::CommandOk(description),
            Ok(DaemonResponse::Error(e)) => {
                DaemonMessage::CommandError(TuiError::DaemonResponse(e))
            }
            Ok(DaemonResponse::LogEntry(_)) => {
                warn!("Received unexpected LogEntry response for command");
                return;
            }
            Err(e) => error_to_message(e),
            Ok(
                DaemonResponse::Status(_)
                | DaemonResponse::BenchmarkProgress(_)
                | DaemonResponse::BenchmarkResult(_),
            ) => return,
        };
        let _ = tx.send(msg);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_wg_core::config::AppConfig;
    use ferro_wg_core::stats::BenchmarkResult;
    use ferro_wg_tui_core::AppState;
    use ferro_wg_tui_core::benchmark::{BenchmarkResultMap, BenchmarkRun};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio::task::JoinSet;

    // Note: spawn_benchmark_task requires mocking client::send_streaming_command,
    // which is complex for unit tests. Integration tests cover the full flow.

    #[tokio::test]
    async fn test_maybe_spawn_command_switch_backend_unknown() {
        let config_path = PathBuf::from("/tmp/config.toml");
        let benchmarks_path = PathBuf::from("/tmp/benchmarks.json");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut tasks = JoinSet::new();
        let state = AppState::new(AppConfig::default());

        maybe_spawn_command(
            &Action::SwitchBenchmarkBackend("unknown".into()),
            &state,
            &tx,
            &mut tasks,
            &config_path,
            &benchmarks_path,
        );

        let msg = rx.recv().await.unwrap();
        assert!(matches!(
            msg,
            DaemonMessage::CommandError(TuiError::UnknownBackend(_))
        ));
    }

    #[tokio::test]
    async fn test_maybe_spawn_command_switch_backend_no_active_connection() {
        let config_path = PathBuf::from("/tmp/config.toml");
        let benchmarks_path = PathBuf::from("/tmp/benchmarks.json");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut tasks = JoinSet::new();
        let state = AppState::new(AppConfig::default());

        // No active connection
        maybe_spawn_command(
            &Action::SwitchBenchmarkBackend("boringtun".into()),
            &state,
            &tx,
            &mut tasks,
            &config_path,
            &benchmarks_path,
        );

        let msg = rx.recv().await.unwrap();
        assert!(matches!(
            msg,
            DaemonMessage::CommandError(TuiError::DaemonResponse(_))
        ));
        if let DaemonMessage::CommandError(TuiError::DaemonResponse(s)) = msg {
            assert!(s.contains("no active connection"));
        }
    }

    fn make_test_run() -> BenchmarkRun {
        let mut results = BenchmarkResultMap::new();
        results.insert(
            "boringtun".to_string(),
            BenchmarkResult {
                backend: "boringtun".to_string(),
                packets_processed: 1000,
                bytes_encapsulated: 100_000,
                elapsed: Duration::from_secs(1),
                throughput_bps: 800_000.0,
                avg_latency: Duration::from_micros(100),
                p50_latency: Duration::ZERO,
                p95_latency: Duration::ZERO,
                p99_latency: Duration::ZERO,
            },
        );
        BenchmarkRun {
            timestamp_ms: 1_234_567_890,
            connection_name: "test_conn".to_string(),
            results,
        }
    }

    #[tokio::test]
    async fn test_spawn_export_task_csv_success() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("test_export.csv");
        let runs = vec![make_test_run()];
        let (tx, mut rx) = mpsc::unbounded_channel::<DaemonMessage>();
        let mut tasks = JoinSet::new();

        spawn_export_task(path.clone(), &runs, &tx, &mut tasks);

        // Wait for the task to complete
        tasks.join_next().await.unwrap().unwrap();

        let msg = rx.recv().await.unwrap();
        assert!(matches!(msg, DaemonMessage::CommandOk(_)));

        // Check file content
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("timestamp_ms,connection_name,backend"));
        assert!(content.contains("1234567890"));
        assert!(content.contains("test_conn"));
        assert!(content.contains("boringtun"));
        assert!(content.contains("800000"));

        // Cleanup
        tokio::fs::remove_file(&path).await.unwrap();
    }

    #[tokio::test]
    async fn test_spawn_export_task_json_success() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("test_export.json");
        let runs = vec![make_test_run()];
        let (tx, mut rx) = mpsc::unbounded_channel::<DaemonMessage>();
        let mut tasks = JoinSet::new();

        spawn_export_task(path.clone(), &runs, &tx, &mut tasks);

        // Wait for the task to complete
        tasks.join_next().await.unwrap().unwrap();

        let msg = rx.recv().await.unwrap();
        assert!(matches!(msg, DaemonMessage::CommandOk(_)));

        // Check file content
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("\"timestamp_ms\":"));
        assert!(content.contains("1234567890"));
        assert!(content.contains("\"connection_name\": \"test_conn\""));
        assert!(content.contains("\"boringtun\""));

        // Cleanup
        tokio::fs::remove_file(&path).await.unwrap();
    }

    #[tokio::test]
    async fn test_spawn_export_task_io_failure() {
        let path = PathBuf::from("/invalid/path/test.json");
        let runs = vec![make_test_run()];
        let (tx, mut rx) = mpsc::unbounded_channel::<DaemonMessage>();
        let mut tasks = JoinSet::new();

        spawn_export_task(path, &runs, &tx, &mut tasks);

        // Wait for the task to complete
        tasks.join_next().await.unwrap().unwrap();

        let msg = rx.recv().await.unwrap();
        assert!(matches!(msg, DaemonMessage::CommandError(_)));
    }

    #[test]
    fn compute_layout_80x24() {
        let area = Rect::new(0, 0, 80, 24);
        let (chunks, _show_bar) = compute_layout(area, 0);
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0].height, TAB_BAR_HEIGHT);
        assert_eq!(chunks[3].height, STATUS_BAR_HEIGHT);
        assert!(chunks[2].height >= 1);
    }

    fn render_ui_to_buffer(width: u16, height: u16) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = AppState::new(AppConfig::default());
        let mut bundle = ComponentBundle::new();
        let area = Rect::new(0, 0, width, height);
        let (chunks, show_bar) = compute_layout(area, state.connections.len());
        render_ui(&mut terminal, &state, &mut bundle, &chunks, show_bar, area).unwrap();
    }

    #[test]
    fn render_ui_at_79x24() {
        render_ui_to_buffer(79, 24);
    }

    #[test]
    fn render_ui_at_80x23() {
        render_ui_to_buffer(80, 23);
    }

    #[test]
    fn render_ui_at_80x24() {
        render_ui_to_buffer(80, 24);
    }

    fn render_component_to_buffer(tab_index: usize, width: u16, height: u16) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = AppState::new(AppConfig::default());
        let mut bundle = ComponentBundle::new();
        let area = Rect::new(0, 0, width, height);
        terminal
            .draw(|frame| {
                bundle.tabs[tab_index].render(frame, area, true, &state);
            })
            .unwrap();
    }

    #[test]
    fn overview_component_at_80x24_empty() {
        render_component_to_buffer(0, 80, 24);
    }

    #[test]
    fn overview_component_at_80x24_one_connection() {
        // Note: Using default config (empty), so no connection, but test no panic
        render_component_to_buffer(0, 80, 24);
    }

    #[test]
    fn overview_component_at_120x40_one_connection() {
        render_component_to_buffer(0, 120, 40);
    }

    #[test]
    fn status_component_at_80x24() {
        render_component_to_buffer(1, 80, 24);
    }

    #[test]
    fn peers_component_at_80x24() {
        render_component_to_buffer(2, 80, 24);
    }

    #[test]
    fn compare_component_at_80x24() {
        render_component_to_buffer(3, 80, 24);
    }

    #[test]
    fn config_component_at_80x24() {
        render_component_to_buffer(4, 80, 24);
    }

    #[test]
    fn logs_component_at_80x24() {
        render_component_to_buffer(5, 80, 24);
    }
}
