//! Unix socket IPC client for communicating with the `ferro-wg` daemon.
//!
//! Shared by both the CLI binary and the TUI crate. Opens a fresh
//! connection for each command (one-shot protocol).

use std::path::Path;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::ipc::{self, DaemonCommand, DaemonResponse, SOCKET_PATH};

/// Errors that can occur when communicating with the daemon.
///
/// This is the shared IPC error type used by both the CLI binary and
/// the TUI crate. Each variant represents a distinct failure mode in
/// the one-shot Unix socket protocol (connect → send → read → close).
/// At the TUI layer, these errors are converted into the UI's own
/// message type via a centralized boundary helper.
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
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
                DaemonClientError::NotRunning
            }
            _ => DaemonClientError::Io(e),
        })?;

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

/// Stream daemon logs from the default socket path.
///
/// Returns a receiver that yields log lines as they are emitted by the daemon.
/// The stream continues until the connection is closed or an error occurs.
///
/// # Errors
///
/// Returns a [`DaemonClientError`] if the daemon is unreachable or the command cannot be encoded.
pub async fn stream_logs() -> Result<tokio::sync::mpsc::Receiver<String>, DaemonClientError> {
    stream_logs_from(Path::new(SOCKET_PATH)).await
}

/// Stream daemon logs from a specific socket path.
///
/// # Errors
///
/// Returns a [`DaemonClientError`] describing the failure.
pub async fn stream_logs_from(
    socket_path: &Path,
) -> Result<tokio::sync::mpsc::Receiver<String>, DaemonClientError> {
    let stream = UnixStream::connect(socket_path)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
                DaemonClientError::NotRunning
            }
            _ => DaemonClientError::Io(e),
        })?;

    let (reader, mut writer) = stream.into_split();

    // Send StreamLogs command.
    let json = ipc::encode_message(&DaemonCommand::StreamLogs)?;
    writer.write_all(json.as_bytes()).await?;
    writer.flush().await?;
    // Shut down write half.
    drop(writer);

    // Read log lines.
    let mut reader = BufReader::new(reader);
    let (tx, rx) = tokio::sync::mpsc::channel(100);

    tokio::spawn(async move {
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if let Ok(DaemonResponse::LogLine(log)) =
                        ipc::decode_message::<DaemonResponse>(&line)
                    {
                        if tx.send(log).await.is_err() {
                            break;
                        }
                    } else {
                        break; // unexpected response
                    }
                }
            }
        }
    });

    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_running_is_not_running() {
        assert!(DaemonClientError::NotRunning.is_not_running());
    }

    #[test]
    fn other_errors_are_not_not_running() {
        assert!(!DaemonClientError::NoResponse.is_not_running());
        let io_err = DaemonClientError::Io(std::io::Error::other("test"));
        assert!(!io_err.is_not_running());
    }

    #[test]
    fn error_display_messages() {
        assert_eq!(
            DaemonClientError::NotRunning.to_string(),
            "daemon is not running"
        );
        assert_eq!(
            DaemonClientError::NoResponse.to_string(),
            "daemon closed connection without response"
        );
    }

    #[tokio::test]
    async fn send_to_nonexistent_socket_returns_not_running() {
        let result =
            send_command_to(&DaemonCommand::Status, Path::new("/tmp/nonexistent.sock")).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().is_not_running());
    }
}
