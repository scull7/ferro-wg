//! Unix socket IPC client for communicating with the `ferro-wg` daemon.

use std::path::Path;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use ferro_wg_core::ipc::{self, DaemonCommand, DaemonResponse, SOCKET_PATH};

/// Send a command to the daemon and return the response.
///
/// # Errors
///
/// Returns an error if the daemon is not running, the connection fails,
/// or the response cannot be parsed.
pub async fn send_command(cmd: &DaemonCommand) -> Result<DaemonResponse, String> {
    send_command_to(cmd, Path::new(SOCKET_PATH)).await
}

/// Send a command to the daemon at a specific socket path.
///
/// # Errors
///
/// Returns a descriptive error string if the daemon is unreachable.
pub async fn send_command_to(
    cmd: &DaemonCommand,
    socket_path: &Path,
) -> Result<DaemonResponse, String> {
    let stream = UnixStream::connect(socket_path)
        .await
        .map_err(|_| "NOT_RUNNING".to_owned())?;

    let (reader, mut writer) = stream.into_split();

    // Send command.
    let json = ipc::encode_message(cmd).map_err(|e| format!("encode command: {e}"))?;
    writer
        .write_all(json.as_bytes())
        .await
        .map_err(|e| format!("send command: {e}"))?;
    writer.flush().await.map_err(|e| format!("flush: {e}"))?;
    // Shut down write half so daemon knows we're done sending.
    drop(writer);

    // Read response.
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .map_err(|e| format!("read response: {e}"))?;

    if line.is_empty() {
        return Err("daemon closed connection without response".into());
    }

    ipc::decode_message(&line).map_err(|e| format!("decode response: {e}"))
}
