//! `ferro-wg-daemon`: Privileged daemon for `WireGuard` tunnel management.
//!
//! Runs as root, listens on a Unix socket, and manages TUN devices,
//! UDP sockets, and `WireGuard` packet loops on behalf of the unprivileged
//! CLI/TUI.

mod server;

use std::path::PathBuf;

use clap::Parser;
use ferro_wg_core::daemon::{LogBuffer, LogLayer};
use ferro_wg_core::ipc::LogEntry;
use tokio::sync::broadcast;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, Layer};

/// Privileged `WireGuard` tunnel daemon.
#[derive(Debug, Parser)]
#[command(name = "ferro-wg-daemon", version, about)]
struct Cli {
    /// Path to the configuration file.
    #[arg(short, long)]
    config: PathBuf,

    /// Unix socket path for IPC.
    #[arg(short, long, default_value = ferro_wg_core::ipc::SOCKET_PATH)]
    socket: PathBuf,

    /// Increase log verbosity (-v, -vv, -vvv).
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let filter = match cli.verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };

    let log_buffer = LogBuffer::new(1000);

    // Broadcast channel: each StreamLogs client subscribes independently.
    let (log_tx, _) = broadcast::channel::<LogEntry>(1000);
    let log_tx_broadcast = log_tx.clone();

    // Spawn broadcaster task to drain the ring buffer into the broadcast channel.
    let log_buffer_for_broadcast = log_buffer.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new()
            .expect("failed to create Tokio runtime for log broadcaster");
        rt.block_on(async {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                let entries = log_buffer_for_broadcast.drain_logs();
                for entry in entries {
                    // Ignored if no subscribers are currently connected.
                    let _ = log_tx_broadcast.send(entry);
                }
            }
        });
    });

    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(EnvFilter::new(filter)))
        .with(LogLayer::new(log_buffer.clone()));
    tracing::subscriber::set_global_default(subscriber).expect("set global subscriber");

    // Load config.
    let config = ferro_wg_core::config::toml::load_app_config(&cli.config)?;
    tracing::info!(
        "Loaded config with {} connection(s) from {}",
        config.connections.len(),
        cli.config.display()
    );

    // Run the IPC server.
    server::run(config, &cli.config, &cli.socket, log_buffer, log_tx).await
}
