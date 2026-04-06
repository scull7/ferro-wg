//! Unix socket IPC server for the `ferro-wg` daemon.
//!
//! Accepts connections from the CLI/TUI, dispatches [`DaemonCommand`]s to the
//! [`TunnelManager`](crate::tunnel::TunnelManager), and sends back
//! [`DaemonResponse`]s. Shared between the standalone `ferro-wg-daemon` binary
//! and the `ferro-wg daemon` subcommand.
//!
//! The daemon automatically reloads the config file from disk before
//! handling `Up` and `Status` commands, so newly imported connections
//! are picked up without restarting.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;
use tracing::{info, warn};
use tracing_subscriber::Layer;

use crate::config::AppConfig;
use crate::config::toml::load_app_config;
use crate::ipc::{self, DaemonCommand, DaemonResponse, LogEntry, LogLevel};
use crate::tunnel::TunnelManager;

/// Bounded ring buffer for structured daemon log entries.
///
/// Shared between the [`LogLayer`] (writer) and the broadcaster task (reader).
/// `Clone` produces a second handle to the same underlying buffer.
#[derive(Clone)]
pub struct LogBuffer {
    buffer: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl LogBuffer {
    /// Create a new buffer that holds at most `capacity` entries, evicting the
    /// oldest when full.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
        }
    }

    /// Append an entry, evicting the oldest when at capacity.
    fn add_entry(&self, entry: LogEntry) {
        match self.buffer.lock() {
            Ok(mut buf) => {
                if buf.len() == buf.capacity() {
                    buf.pop_front();
                }
                buf.push_back(entry);
            }
            Err(_) => {
                warn!("LogBuffer mutex poisoned, skipping log entry");
            }
        }
    }

    /// Return a snapshot of all entries currently in the buffer.
    ///
    /// Returns an empty `Vec` when the mutex is poisoned.
    #[must_use]
    pub fn get_buffer(&self) -> Vec<LogEntry> {
        if let Ok(buf) = self.buffer.lock() {
            buf.iter().cloned().collect()
        } else {
            warn!("LogBuffer mutex poisoned, returning empty buffer");
            Vec::new()
        }
    }

    /// Drain and return all entries, leaving the buffer empty.
    ///
    /// Returns an empty `Vec` when the mutex is poisoned.
    #[must_use]
    pub fn drain_logs(&self) -> Vec<LogEntry> {
        if let Ok(mut buf) = self.buffer.lock() {
            buf.drain(..).collect()
        } else {
            warn!("LogBuffer mutex poisoned, cannot drain logs");
            Vec::new()
        }
    }
}

/// Tracing layer that routes log events into a [`LogBuffer`] as structured
/// [`LogEntry`] values.
///
/// This is the sole edge between the tracing subsystem (calculation) and the
/// log buffer (data). All I/O — broadcasting, streaming — happens elsewhere
/// at the application boundary.
pub struct LogLayer {
    buffer: LogBuffer,
}

impl LogLayer {
    /// Wrap a [`LogBuffer`] as a tracing subscriber layer.
    #[must_use]
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

/// Visitor that extracts `message` and optional `connection` fields from a
/// tracing event.
///
/// `record_str` is the primary path: tracing macros pass literal string
/// messages as `&str`, arriving here without any Debug overhead.
/// `record_debug` is the fallback for values that implement only `Debug`;
/// the representation is used verbatim.
#[derive(Default)]
struct LogVisitor {
    message: String,
    connection: Option<String>,
}

impl tracing::field::Visit for LogVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "message" => value.clone_into(&mut self.message),
            "connection" => self.connection = Some(value.to_owned()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => self.message = format!("{value:?}"),
            "connection" => self.connection = Some(format!("{value:?}")),
            _ => {}
        }
    }
}

/// Map a [`tracing::Level`] to the IPC [`LogLevel`] enum.
fn map_tracing_level(level: tracing::Level) -> LogLevel {
    match level {
        tracing::Level::TRACE => LogLevel::Trace,
        tracing::Level::DEBUG => LogLevel::Debug,
        tracing::Level::INFO => LogLevel::Info,
        tracing::Level::WARN => LogLevel::Warn,
        tracing::Level::ERROR => LogLevel::Error,
    }
}

impl<S> Layer<S> for LogLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = LogVisitor::default();
        event.record(&mut visitor);
        let level = map_tracing_level(*event.metadata().level());
        let message = format!("{}: {}", event.metadata().target(), visitor.message);
        let entry = LogEntry::now(level, visitor.connection, message);
        self.buffer.add_entry(entry);
    }
}

/// Set socket permissions to allow unprivileged connections.
///
/// # Errors
///
/// Returns an error if permissions cannot be set.
#[cfg(unix)]
fn set_socket_permissions(socket_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o666))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_socket_permissions(_socket_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

/// Set up the Unix listener for IPC connections.
///
/// # Errors
///
/// Returns an error if the socket file cannot be removed or bound.
fn setup_listener(socket_path: &Path) -> Result<UnixListener, Box<dyn std::error::Error>> {
    // Remove stale socket file.
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }

    let listener = UnixListener::bind(socket_path)?;
    Ok(listener)
}

/// Handle a `StreamLogs` connection: replay history, then stream live entries.
///
/// The live-stream channel is bounded `mpsc` so the sender blocks under
/// backpressure rather than dropping messages.  The upstream ring buffer
/// ([`LogBuffer`]) can still evict old entries under sustained load before
/// they reach the channel.
///
/// # Errors
///
/// Returns an error if sending responses fails.
#[tracing::instrument(skip(writer, log_buffer))]
async fn handle_stream_logs(
    mut reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    mut writer: tokio::net::unix::OwnedWriteHalf,
    log_buffer: &LogBuffer,
    log_rx: &mut mpsc::Receiver<LogEntry>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Replay historical buffer first so the client sees context on connect.
    for entry in log_buffer.get_buffer() {
        let resp = DaemonResponse::LogEntry(entry);
        if let Err(e) = send_response(&mut writer, &resp).await {
            warn!("Failed to send buffered log entry: {e}");
            return Ok(());
        }
    }

    // Stream live entries, breaking immediately when the client disconnects
    // (EOF on the read half) so the daemon event loop is not blocked waiting
    // for the next log entry.
    let mut discard = String::new();
    loop {
        discard.clear();
        tokio::select! {
            entry = log_rx.recv() => match entry {
                Some(entry) => {
                    let resp = DaemonResponse::LogEntry(entry);
                    if let Err(e) = send_response(&mut writer, &resp).await {
                        warn!("Failed to send streamed log entry: {e}");
                        break;
                    }
                }
                None => break,
            },
            // EOF (Ok(0)) or error both signal the client has disconnected.
            _ = reader.read_line(&mut discard) => break,
        }
    }
    Ok(())
}

/// Read and decode a command from the client stream.
///
/// # Errors
///
/// Returns an error if reading or decoding fails.
async fn read_command(
    reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: &mut tokio::net::unix::OwnedWriteHalf,
) -> Result<Option<DaemonCommand>, Box<dyn std::error::Error>> {
    let mut line = String::new();

    // Read one command per connection.
    match reader.read_line(&mut line).await {
        Ok(0) => return Ok(None), // EOF
        Ok(_) => {}
        Err(e) => {
            warn!("Read error: {e}");
            return Ok(None);
        }
    }

    match ipc::decode_message(&line) {
        Ok(cmd) => Ok(Some(cmd)),
        Err(e) => {
            let resp = DaemonResponse::Error(format!("invalid command: {e}"));
            let _ = send_response(writer, &resp).await;
            Ok(None)
        }
    }
}

/// Process a non-streaming command.
///
/// # Errors
///
/// Returns an error if sending response fails.
async fn process_command(
    cmd: &DaemonCommand,
    manager: &mut TunnelManager,
    config_path: &Path,
    writer: &mut tokio::net::unix::OwnedWriteHalf,
) -> Result<bool, Box<dyn std::error::Error>> {
    // Reload config for commands that need the latest connections.
    if needs_config_reload(cmd) {
        reload_config(manager, config_path);
    }

    let response = handle_command(manager, cmd).await;

    // Check for shutdown before sending response.
    let is_shutdown =
        matches!(response, DaemonResponse::Ok) && matches!(cmd, DaemonCommand::Shutdown);

    let _ = send_response(writer, &response).await;

    Ok(is_shutdown)
}

/// Accept the next connection from the listener and decode its command.
///
/// Returns `None` if the connection closed before a valid command was received.
/// The read half is returned alongside the write half so callers that stream
/// responses back (e.g. `handle_stream_logs`) can detect client disconnect via
/// EOF on the read side.
///
/// # Errors
///
/// Returns an error on socket I/O or protocol decode failure.
async fn accept_command(
    listener: &UnixListener,
) -> Result<
    Option<(
        BufReader<tokio::net::unix::OwnedReadHalf>,
        tokio::net::unix::OwnedWriteHalf,
        DaemonCommand,
    )>,
    Box<dyn std::error::Error>,
> {
    let (stream, _addr) = listener.accept().await?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let Some(command) = read_command(&mut reader, &mut writer).await? else {
        return Ok(None);
    };
    info!("Received command: {command:?}");
    Ok(Some((reader, writer, command)))
}

/// Route a decoded command to its handler.
///
/// Returns `true` if shutdown was requested.
///
/// # Errors
///
/// Returns an error on socket I/O issues.
#[tracing::instrument(skip(manager, log_buffer))]
async fn handle_connection(
    listener: &UnixListener,
    manager: &mut TunnelManager,
    config_path: &Path,
    log_buffer: &LogBuffer,
    log_rx: &mut mpsc::Receiver<LogEntry>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let Some((reader, mut writer, command)) = accept_command(listener).await? else {
        return Ok(false);
    };
    match command {
        DaemonCommand::StreamLogs => {
            handle_stream_logs(reader, writer, log_buffer, log_rx).await?;
            Ok(false)
        }
        cmd => process_command(&cmd, manager, config_path, &mut writer).await,
    }
}

/// Run the IPC server loop.
///
/// Listens on a Unix socket and handles commands from clients.
/// Runs until a `Shutdown` command is received. Automatically reloads
/// the config file before `Up` and `Status` commands.
///
/// # Errors
///
/// Returns an error if the socket cannot be bound.
#[tracing::instrument(skip(config, log_buffer))]
pub async fn run(
    config: AppConfig,
    config_path: &Path,
    socket_path: &Path,
    log_buffer: LogBuffer,
    mut log_rx: mpsc::Receiver<LogEntry>,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = setup_listener(socket_path)?;
    set_socket_permissions(socket_path)?;

    info!("Listening on {}", socket_path.display());

    let mut manager = TunnelManager::new(config);
    let config_path = config_path.to_owned();

    loop {
        if handle_connection(
            &listener,
            &mut manager,
            &config_path,
            &log_buffer,
            &mut log_rx,
        )
        .await?
        {
            break; // Shutdown
        }
    }

    Ok(())
}
/// Check if a command should trigger a config reload.
fn needs_config_reload(command: &DaemonCommand) -> bool {
    matches!(command, DaemonCommand::Up { .. } | DaemonCommand::Status)
}

/// Reload config from disk, logging any errors without failing.
fn reload_config(manager: &mut TunnelManager, config_path: &Path) {
    match load_app_config(config_path) {
        Ok(new_config) => manager.reload_config(new_config),
        Err(e) => warn!(
            "Failed to reload config from {}: {e}",
            config_path.display()
        ),
    }
}

/// Dispatch a command to the tunnel manager and produce a response.
#[tracing::instrument(skip(manager))]
async fn handle_command(manager: &mut TunnelManager, command: &DaemonCommand) -> DaemonResponse {
    match *command {
        DaemonCommand::Up {
            ref connection_name,
            backend,
        } => {
            let result = if let Some(name) = connection_name {
                info!(connection = %name, "bringing up tunnel");
                manager.up(name, backend).await
            } else {
                manager.up_all(backend).await
            };
            result.map_or_else(
                |e| DaemonResponse::Error(e.to_string()),
                |()| DaemonResponse::Ok,
            )
        }
        DaemonCommand::Down {
            ref connection_name,
        } => {
            if let Some(name) = connection_name {
                info!(connection = %name, "tearing down tunnel");
                manager.down(name).map_or_else(
                    |e| DaemonResponse::Error(e.to_string()),
                    |()| DaemonResponse::Ok,
                )
            } else {
                manager.down_all();
                DaemonResponse::Ok
            }
        }
        DaemonCommand::Status => DaemonResponse::Status(manager.status()),
        DaemonCommand::SwitchBackend {
            ref connection_name,
            backend,
        } => {
            if let Err(e) = manager.down(connection_name) {
                warn!(connection = %connection_name, "down before backend switch failed: {e}");
            }
            info!(connection = %connection_name, "switching backend to {backend:?}");
            manager.up(connection_name, backend).await.map_or_else(
                |e| DaemonResponse::Error(e.to_string()),
                |()| DaemonResponse::Ok,
            )
        }
        DaemonCommand::Shutdown => DaemonResponse::Ok,
        DaemonCommand::Benchmark { .. } => {
            // TODO: Implement benchmark in later commit
            DaemonResponse::Error("Benchmark not implemented yet".to_string())
        }
        DaemonCommand::StreamLogs => {
            warn!("StreamLogs command received in handle_command, should be handled separately");
            DaemonResponse::Error("StreamLogs not supported here".to_string())
        }
    }
}

/// Send a JSON response to the client.
async fn send_response(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    response: &DaemonResponse,
) -> Result<(), Box<dyn std::error::Error>> {
    let json = ipc::encode_message(response)?;
    writer.write_all(json.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use tokio::sync::mpsc;

    use super::*;

    /// Build a test [`LogEntry`] with the given message and `Debug` level.
    fn make_entry(msg: &str) -> LogEntry {
        LogEntry {
            timestamp_ms: 0,
            level: LogLevel::Debug,
            connection_name: None,
            message: msg.to_owned(),
        }
    }

    fn make_entry_for(connection: &str, msg: &str) -> LogEntry {
        LogEntry {
            timestamp_ms: 0,
            level: LogLevel::Info,
            connection_name: Some(connection.to_owned()),
            message: msg.to_owned(),
        }
    }

    #[test]
    fn log_buffer_capacity() {
        let buffer = LogBuffer::new(3);

        buffer.add_entry(make_entry("line1"));
        buffer.add_entry(make_entry("line2"));
        buffer.add_entry(make_entry("line3"));
        let snapshot = buffer.get_buffer();
        let msgs: Vec<&str> = snapshot.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(msgs, vec!["line1", "line2", "line3"]);

        // Overflow: should evict oldest.
        buffer.add_entry(make_entry("line4"));
        let snapshot = buffer.get_buffer();
        let msgs: Vec<&str> = snapshot.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(msgs, vec!["line2", "line3", "line4"]);

        assert_eq!(buffer.buffer.lock().unwrap().capacity(), 3);
    }

    #[test]
    fn log_buffer_overflow_eviction() {
        let buffer = LogBuffer::new(3);
        for i in 0..5 {
            buffer.add_entry(make_entry(&format!("line{i}")));
        }
        let msgs: Vec<String> = buffer.get_buffer().into_iter().map(|e| e.message).collect();
        assert_eq!(msgs, vec!["line2", "line3", "line4"]);
    }

    #[test]
    fn log_buffer_duplicates_are_stored() {
        let buffer = LogBuffer::new(5);
        for _ in 0..3 {
            buffer.add_entry(make_entry("dup"));
        }
        assert_eq!(buffer.get_buffer().len(), 3);
        assert!(buffer.get_buffer().iter().all(|e| e.message == "dup"));
    }

    #[test]
    fn log_buffer_drain_clears_buffer() {
        let buffer = LogBuffer::new(5);
        for i in 0..4 {
            buffer.add_entry(make_entry(&format!("line{i}")));
        }
        let drained = buffer.drain_logs();
        assert_eq!(drained.len(), 4);
        assert!(buffer.get_buffer().is_empty());
    }

    #[test]
    fn log_buffer_large_overflow_keeps_last_n() {
        let buffer = LogBuffer::new(5);
        for i in 0..20 {
            buffer.add_entry(make_entry(&format!("line{i}")));
        }
        let contents = buffer.get_buffer();
        assert_eq!(contents.len(), 5);
        let msgs: Vec<String> = contents.into_iter().map(|e| e.message).collect();
        assert_eq!(msgs, vec!["line15", "line16", "line17", "line18", "line19"]);
    }

    /// Poison the mutex by panicking while holding the lock, then verify that
    /// all `LogBuffer` methods handle the poisoned state without panicking.
    #[test]
    fn log_buffer_poisoned_mutex_does_not_panic() {
        let buffer = LogBuffer::new(3);
        let clone = buffer.clone();

        // Poison the mutex.
        let _ = thread::spawn(move || {
            let _guard = clone.buffer.lock().unwrap();
            panic!("intentional panic to poison mutex");
        })
        .join();

        // All methods must be safe to call on a poisoned buffer.
        buffer.add_entry(make_entry("after poison"));
        assert!(buffer.get_buffer().is_empty());
        assert!(buffer.drain_logs().is_empty());
    }

    #[test]
    fn log_buffer_concurrent_additions_no_deadlock() {
        let buffer = Arc::new(LogBuffer::new(100));
        let mut handles = Vec::new();

        for i in 0..10 {
            let b = Arc::clone(&buffer);
            handles.push(thread::spawn(move || {
                for j in 0..20 {
                    b.add_entry(make_entry(&format!("thread{i}-line{j}")));
                }
            }));
        }

        for h in handles {
            h.join().expect("thread panicked");
        }

        // 10 threads × 20 = 200 total additions, but capacity is 100.
        assert_eq!(buffer.get_buffer().len(), 100);
    }

    #[test]
    fn log_buffer_concurrent_add_and_drain_no_deadlock() {
        let buffer = Arc::new(LogBuffer::new(50));
        let mut handles = Vec::new();

        // Adder threads.
        for i in 0..4 {
            let b = Arc::clone(&buffer);
            handles.push(thread::spawn(move || {
                for j in 0..25 {
                    b.add_entry(make_entry(&format!("t{i}-{j}")));
                }
            }));
        }

        // Drain thread runs concurrently with adders.
        let drain_buf = Arc::clone(&buffer);
        handles.push(thread::spawn(move || {
            for _ in 0..10 {
                let _ = drain_buf.drain_logs();
                thread::yield_now();
            }
        }));

        for h in handles {
            h.join().expect("thread panicked");
        }
        // No assertion on final count — the goal is no deadlock / panic.
    }

    #[test]
    fn log_buffer_entries_carry_connection_name() {
        let buffer = LogBuffer::new(5);
        buffer.add_entry(make_entry("global"));
        buffer.add_entry(make_entry_for("mia", "conn-specific"));
        let entries = buffer.get_buffer();
        assert_eq!(entries[0].connection_name, None);
        assert_eq!(entries[1].connection_name.as_deref(), Some("mia"));
    }

    #[test]
    fn map_tracing_level_covers_all_variants() {
        assert_eq!(map_tracing_level(tracing::Level::TRACE), LogLevel::Trace);
        assert_eq!(map_tracing_level(tracing::Level::DEBUG), LogLevel::Debug);
        assert_eq!(map_tracing_level(tracing::Level::INFO), LogLevel::Info);
        assert_eq!(map_tracing_level(tracing::Level::WARN), LogLevel::Warn);
        assert_eq!(map_tracing_level(tracing::Level::ERROR), LogLevel::Error);
    }

    /// `handle_stream_logs` must return `Ok(())` when the writer half is
    /// closed before any data is sent (historical replay path).
    #[tokio::test]
    async fn handle_stream_logs_writer_closed_before_replay() {
        let buffer = LogBuffer::new(5);
        buffer.add_entry(make_entry("old entry"));

        let (client, server) = tokio::net::UnixStream::pair().unwrap();
        let (server_read, server_write) = server.into_split();
        let server_read = BufReader::new(server_read);

        // Drop the client immediately so writes to server_write fail.
        drop(client);

        let (_tx, mut rx) = mpsc::channel::<LogEntry>(1);
        let result = handle_stream_logs(server_read, server_write, &buffer, &mut rx).await;
        assert!(result.is_ok());
    }

    /// `handle_stream_logs` must return `Ok(())` when the writer fails
    /// mid-stream while forwarding live log entries.
    #[tokio::test]
    async fn handle_stream_logs_writer_closed_during_live_stream() {
        let buffer = LogBuffer::new(5);

        let (client, server) = tokio::net::UnixStream::pair().unwrap();
        let (server_read, server_write) = server.into_split();
        let server_read = BufReader::new(server_read);

        // Drop the client so the next write into server_write will fail.
        drop(client);

        let (tx, mut rx) = mpsc::channel::<LogEntry>(4);
        tx.send(make_entry("live entry")).await.unwrap();

        let result = handle_stream_logs(server_read, server_write, &buffer, &mut rx).await;
        assert!(result.is_ok());
    }

    /// `handle_stream_logs` replays the historical buffer then streams live entries.
    #[tokio::test]
    async fn handle_stream_logs_replays_then_streams() {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let buffer = LogBuffer::new(5);
        buffer.add_entry(make_entry("hist1"));
        buffer.add_entry(make_entry("hist2"));

        let (client, server) = tokio::net::UnixStream::pair().unwrap();
        let (client_read, _client_write) = client.into_split();
        let (server_read, server_write) = server.into_split();
        let server_read = BufReader::new(server_read);

        let (tx, mut rx) = mpsc::channel::<LogEntry>(4);
        tx.send(make_entry("live1")).await.unwrap();
        // Close the sender so handle_stream_logs terminates after draining.
        drop(tx);

        handle_stream_logs(server_read, server_write, &buffer, &mut rx)
            .await
            .unwrap();

        // Read all JSON lines the server wrote to the client.
        let mut reader = BufReader::new(client_read);
        let mut lines = Vec::new();
        let mut line = String::new();
        while reader.read_line(&mut line).await.unwrap() > 0 {
            lines.push(std::mem::take(&mut line));
        }

        // 2 historical + 1 live = 3 responses.
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("hist1"));
        assert!(lines[1].contains("hist2"));
        assert!(lines[2].contains("live1"));
    }
}
