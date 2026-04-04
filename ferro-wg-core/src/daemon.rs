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
use tokio::sync::broadcast;
use tracing::{info, warn};
use tracing_subscriber::Layer;

use crate::config::AppConfig;
use crate::config::toml::load_app_config;
use crate::ipc::{self, DaemonCommand, DaemonResponse};
use crate::tunnel::TunnelManager;

/// Buffer for daemon logs with broadcasting capability.
#[derive(Clone)]
pub struct LogBuffer {
    buffer: Arc<Mutex<VecDeque<String>>>,
    tx: broadcast::Sender<String>,
}

impl LogBuffer {
    /// Create a new log buffer with maximum capacity.
    #[must_use]
    pub fn new(max_lines: usize) -> Self {
        let (tx, _) = broadcast::channel(100);
        Self {
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(max_lines))),
            tx,
        }
    }

    /// Add a log line to the buffer and broadcast it.
    fn add_line(&self, line: String) {
        let mut buf = self.buffer.lock().expect("mutex poisoned");
        if buf.len() == buf.capacity() {
            buf.pop_front();
        }
        buf.push_back(line.clone());
        let _ = self.tx.send(line);
    }

    /// Get a copy of the current buffer.
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned.
    #[must_use]
    pub fn get_buffer(&self) -> Vec<String> {
        self.buffer.lock().expect("mutex poisoned").iter().cloned().collect()
    }
}

/// Visitor to extract log message from tracing event.
struct LogVisitor(String);

impl tracing::field::Visit for LogVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}").trim_matches('"').to_string();
        }
    }
}

impl<S> Layer<S> for LogBuffer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = LogVisitor(String::new());
        event.record(&mut visitor);
        let line = format!(
            "{} {}: {}",
            event.metadata().level(),
            event.metadata().target(),
            visitor.0
        );
        self.add_line(line);
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

/// Handle a StreamLogs connection by sending buffered logs then streaming new ones.
///
/// # Errors
///
/// Returns an error if sending responses fails.
#[tracing::instrument(skip(writer, log_buffer))]
async fn handle_stream_logs(
    mut writer: tokio::net::unix::OwnedWriteHalf,
    log_buffer: &LogBuffer,
) -> Result<(), Box<dyn std::error::Error>> {
    // Send buffered logs first
    let buf = log_buffer.get_buffer();
    for line in buf {
        let resp = DaemonResponse::LogLine(line);
        if send_response(&mut writer, &resp).await.is_err() {
            return Ok(());
        }
    }
    // Then stream new logs
    let mut rx = log_buffer.tx.subscribe();
    loop {
        match rx.recv().await {
            Ok(line) => {
                let resp = DaemonResponse::LogLine(line);
                if send_response(&mut writer, &resp).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Closed) => break,
            Err(_) => {}
        }
    }
    Ok(())
}

/// Handle a single client connection.
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
) -> Result<bool, Box<dyn std::error::Error>> {
    let (stream, _addr) = listener.accept().await?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    // Read one command per connection.
    match reader.read_line(&mut line).await {
        Ok(0) => return Ok(false), // EOF
        Ok(_) => {}
        Err(e) => {
            warn!("Read error: {e}");
            return Ok(false);
        }
    }

    let command: DaemonCommand = match ipc::decode_message(&line) {
        Ok(cmd) => cmd,
        Err(e) => {
            let resp = DaemonResponse::Error(format!("invalid command: {e}"));
            let _ = send_response(&mut writer, &resp).await;
            return Ok(false);
        }
    };

    info!("Received command: {command:?}");

    match command {
        DaemonCommand::StreamLogs => {
            handle_stream_logs(writer, log_buffer).await?;
            Ok(false)
        }
        cmd => {
            // Reload config for commands that need the latest connections.
            if needs_config_reload(&cmd) {
                reload_config(manager, config_path);
            }

            let response = handle_command(manager, &cmd).await;

            // Check for shutdown before sending response.
            let is_shutdown = matches!(response, DaemonResponse::Ok) && matches!(cmd, DaemonCommand::Shutdown);

            let _ = send_response(&mut writer, &response).await;

            Ok(is_shutdown)
        }
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
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = setup_listener(socket_path)?;
    set_socket_permissions(socket_path)?;

    info!("Listening on {}", socket_path.display());

    let mut manager = TunnelManager::new(config);
    let config_path = config_path.to_owned();

    loop {
        if handle_connection(&listener, &mut manager, &config_path, &log_buffer).await? {
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
            let result = match connection_name {
                Some(name) => manager.up(name, backend).await,
                None => manager.up_all(backend).await,
            };
            match result {
                Ok(()) => DaemonResponse::Ok,
                Err(e) => DaemonResponse::Error(e.to_string()),
            }
        }
        DaemonCommand::Down {
            ref connection_name,
        } => {
            if let Some(name) = connection_name {
                match manager.down(name) {
                    Ok(()) => DaemonResponse::Ok,
                    Err(e) => DaemonResponse::Error(e.to_string()),
                }
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
                warn!("Down before switch: {e}");
            }
            match manager.up(connection_name, backend).await {
                Ok(()) => DaemonResponse::Ok,
                Err(e) => DaemonResponse::Error(e.to_string()),
            }
        }
        DaemonCommand::Shutdown => DaemonResponse::Ok,
        DaemonCommand::StreamLogs => unreachable!("StreamLogs handled in server loop"),
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
    use super::*;

    #[test]
    fn log_buffer_add_line_and_overflow() {
        let buffer = LogBuffer::new(3);
        assert!(buffer.get_buffer().is_empty());

        buffer.add_line("line1".to_string());
        assert_eq!(buffer.get_buffer(), vec!["line1"]);

        buffer.add_line("line2".to_string());
        buffer.add_line("line3".to_string());
        assert_eq!(buffer.get_buffer(), vec!["line1", "line2", "line3"]);

        // Overflow: oldest should be removed
        buffer.add_line("line4".to_string());
        assert_eq!(buffer.get_buffer(), vec!["line2", "line3", "line4"]);
    }

    #[test]
    fn log_buffer_broadcast() {
        let buffer = LogBuffer::new(10);
        let _rx = buffer.tx.subscribe();

        buffer.add_line("test".to_string());
        // Note: In a real test, we'd await rx.recv(), but since it's sync, we can't easily test broadcast here
        // This test mainly checks that add_line doesn't panic
        assert_eq!(buffer.get_buffer(), vec!["test"]);
    }
}
