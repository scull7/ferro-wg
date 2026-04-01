//! Unix socket IPC server for the `ferro-wg` daemon.
//!
//! Accepts connections from the CLI/TUI, dispatches [`DaemonCommand`]s to the
//! [`TunnelManager`](crate::tunnel::TunnelManager), and sends back
//! [`DaemonResponse`]s. Shared between the standalone `ferro-wg-daemon` binary
//! and the `ferro-wg daemon` subcommand.

use std::path::Path;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tracing::{info, warn};

use crate::config::WgConfig;
use crate::ipc::{self, DaemonCommand, DaemonResponse};
use crate::tunnel::TunnelManager;

/// Run the IPC server loop.
///
/// Listens on a Unix socket and handles commands from clients.
/// Runs until a `Shutdown` command is received.
///
/// # Errors
///
/// Returns an error if the socket cannot be bound.
pub async fn run(config: WgConfig, socket_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // Remove stale socket file.
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }

    let listener = UnixListener::bind(socket_path)?;

    // Allow unprivileged users to connect to the daemon socket.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o666))?;
    }

    info!("Listening on {}", socket_path.display());

    let mut manager = TunnelManager::new(config);

    loop {
        let (stream, _addr) = listener.accept().await?;
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        // Read one command per connection.
        match reader.read_line(&mut line).await {
            Ok(0) => continue, // EOF
            Ok(_) => {}
            Err(e) => {
                warn!("Read error: {e}");
                continue;
            }
        }

        let command: DaemonCommand = match ipc::decode_message(&line) {
            Ok(cmd) => cmd,
            Err(e) => {
                let resp = DaemonResponse::Error(format!("invalid command: {e}"));
                let _ = send_response(&mut writer, &resp).await;
                continue;
            }
        };

        info!("Received command: {command:?}");

        let response = handle_command(&mut manager, command).await;

        // Check for shutdown before sending response.
        let is_shutdown = matches!(response, DaemonResponse::Ok)
            && matches!(
                ipc::decode_message::<DaemonCommand>(&line),
                Ok(DaemonCommand::Shutdown)
            );

        let _ = send_response(&mut writer, &response).await;

        if is_shutdown {
            info!("Shutdown requested, exiting");
            manager.down_all();
            let _ = std::fs::remove_file(socket_path);
            break;
        }
    }

    Ok(())
}

/// Dispatch a command to the tunnel manager and produce a response.
async fn handle_command(manager: &mut TunnelManager, command: DaemonCommand) -> DaemonResponse {
    match command {
        DaemonCommand::Up { peer_name, backend } => {
            let result = match peer_name {
                Some(name) => manager.up(&name, backend).await,
                None => manager.up_all(backend).await,
            };
            match result {
                Ok(()) => DaemonResponse::Ok,
                Err(e) => DaemonResponse::Error(e.to_string()),
            }
        }
        DaemonCommand::Down { peer_name } => {
            if let Some(name) = peer_name {
                match manager.down(&name) {
                    Ok(()) => DaemonResponse::Ok,
                    Err(e) => DaemonResponse::Error(e.to_string()),
                }
            } else {
                manager.down_all();
                DaemonResponse::Ok
            }
        }
        DaemonCommand::Status => DaemonResponse::Status(manager.status()),
        DaemonCommand::SwitchBackend { peer_name, backend } => {
            if let Err(e) = manager.down(&peer_name) {
                warn!("Down before switch: {e}");
            }
            match manager.up(&peer_name, backend).await {
                Ok(()) => DaemonResponse::Ok,
                Err(e) => DaemonResponse::Error(e.to_string()),
            }
        }
        DaemonCommand::Shutdown => DaemonResponse::Ok,
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
