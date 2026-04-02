//! Unix socket IPC client for communicating with the `ferro-wg` daemon.
//!
//! Shared by both the CLI binary and the TUI crate. Opens a fresh
//! connection for each command (one-shot protocol).

use std::path::Path;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::ipc::{self, DaemonCommand, DaemonResponse, SOCKET_PATH};

/// Errors that can occur when communicating with the daemon.
#[derive(Debug, thiserror::Error)]
pub enum DaemonClientError {
    /// The daemon is not running (socket connection refused).
    #[error("daemon is not running")]
    NotRunning,
    /// Failed to encode the command as JSON.
    #[error("encode command: {0}")]
    Encode(#[from] serde_json::Error),
    /// I/O error during socket communication.
    #[error("socket I/O: {0}")]
    Io(#[from] std::io::Error),
    /// Daemon closed the connection without sending a response.
    #[error("daemon closed connection without response")]
    NoResponse,
    /// Failed to decode the daemon's JSON response.
    #[error("decode response: {0}")]
    Decode(serde_json::Error),
}

impl DaemonClientError {
    /// Whether this error indicates the daemon is not running.
    #[must_use]
    pub fn is_not_running(&self) -> bool {
        matches!(self, Self::NotRunning)
    }
}

/// Send a command to the daemon at the default socket path.
///
/// # Errors
///
/// Returns a [`DaemonClientError`] if the daemon is unreachable,
/// the command cannot be encoded, or the response cannot be decoded.
pub async fn send_command(cmd: &DaemonCommand) -> Result<DaemonResponse, DaemonClientError> {
    send_command_to(cmd, Path::new(SOCKET_PATH)).await
}

/// Send a command to the daemon at a specific socket path.
///
/// # Errors
///
/// Returns a [`DaemonClientError`] describing the failure.
pub async fn send_command_to(
    cmd: &DaemonCommand,
    socket_path: &Path,
) -> Result<DaemonResponse, DaemonClientError> {
    let stream = UnixStream::connect(socket_path)
        .await
        .map_err(|_| DaemonClientError::NotRunning)?;

    let (reader, mut writer) = stream.into_split();

    // Send command.
    let json = ipc::encode_message(cmd)?;
    writer.write_all(json.as_bytes()).await?;
    writer.flush().await?;
    // Shut down write half so daemon knows we're done sending.
    drop(writer);

    // Read response.
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;

    if line.is_empty() {
        return Err(DaemonClientError::NoResponse);
    }

    ipc::decode_message(&line).map_err(DaemonClientError::Decode)
}
