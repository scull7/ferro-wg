//! ferro-wg: `WireGuard` TUI and CLI entry point.

mod cli;
mod client;
#[cfg(feature = "tui")]
mod tui;

use std::path::{Path, PathBuf};
use std::process;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Command};
use ferro_wg_core::config;
use ferro_wg_core::config::AppConfig;
use ferro_wg_core::error::BackendKind;
use ferro_wg_core::ipc::{DaemonCommand, DaemonResponse, SOCKET_PATH};
use ferro_wg_core::key::PrivateKey;

fn main() {
    let cli = Cli::parse();

    let filter = match cli.verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .init();

    let config_path = cli.config.unwrap_or_else(default_config_path);

    let result = match cli.command {
        None | Some(Command::Tui) => run_tui(&config_path),
        Some(Command::Up { peer }) => cmd_up(peer.as_deref()),
        Some(Command::Down { peer }) => cmd_down(peer.as_deref()),
        Some(Command::Status) => cmd_status(),
        Some(Command::Daemon { daemonize, stop }) => cmd_daemon(&config_path, daemonize, stop),
        Some(Command::Import { path }) => cmd_import(&path, &config_path),
        Some(Command::Genkey) => {
            cmd_genkey();
            Ok(())
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}

/// Default config file location.
fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ferro-wg")
        .join("config.toml")
}

// -- Daemon interaction helpers --

/// Send a command to the daemon, printing a clean error if it's not running.
fn daemon_command(cmd: &DaemonCommand) -> Result<DaemonResponse, Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client::send_command(cmd)).map_err(|e| {
        if client::is_not_running(&e) {
            "daemon is not running.\n\n\
             Start it with:\n  \
             sudo ferro-wg daemon\n\n\
             Or in the background:\n  \
             sudo ferro-wg daemon --daemonize"
                .into()
        } else {
            e.into()
        }
    })
}

// -- Subcommand handlers --

/// Launch the interactive TUI.
fn run_tui(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "tui")]
    {
        let app_config = load_app_config(config_path)?;
        // For the TUI, create a combined view from the first connection.
        // TODO: update TUI to support multiple connections natively.
        let first = app_config
            .connections
            .values()
            .next()
            .ok_or("no connections configured")?
            .clone();
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(tui::run(first))?;
        Ok(())
    }
    #[cfg(not(feature = "tui"))]
    {
        let _ = config_path;
        Err("TUI feature not enabled. Rebuild with --features tui".into())
    }
}

/// Bring tunnel(s) up via the daemon.
fn cmd_up(peer: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let cmd = DaemonCommand::Up {
        peer_name: peer.map(ToOwned::to_owned),
        backend: BackendKind::Boringtun,
    };
    let response = daemon_command(&cmd)?;

    match response {
        DaemonResponse::Ok => {
            let target = peer.unwrap_or("all connections");
            println!("Brought up: {target}");
            Ok(())
        }
        DaemonResponse::Error(e) => Err(e.into()),
        DaemonResponse::Status(_) => Err("unexpected response from daemon".into()),
    }
}

/// Tear down tunnel(s) via the daemon.
fn cmd_down(peer: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let cmd = DaemonCommand::Down {
        peer_name: peer.map(ToOwned::to_owned),
    };
    let response = daemon_command(&cmd)?;

    match response {
        DaemonResponse::Ok => {
            let target = peer.unwrap_or("all connections");
            println!("Tore down: {target}");
            Ok(())
        }
        DaemonResponse::Error(e) => Err(e.into()),
        DaemonResponse::Status(_) => Err("unexpected response from daemon".into()),
    }
}

/// Print connection status from the daemon.
fn cmd_status() -> Result<(), Box<dyn std::error::Error>> {
    let response = daemon_command(&DaemonCommand::Status)?;

    match response {
        DaemonResponse::Status(peers) => {
            if peers.is_empty() {
                println!("No connections configured.");
                return Ok(());
            }
            for peer in &peers {
                let status = if peer.connected {
                    "connected"
                } else {
                    "disconnected"
                };
                let iface = peer.interface.as_deref().unwrap_or("-");
                let endpoint = peer.endpoint.as_deref().unwrap_or("-");

                println!("connection: {}", peer.name);
                println!("  status: {status}");
                println!("  backend: {}", peer.backend);
                println!("  endpoint: {endpoint}");
                println!("  interface: {iface}");
                if peer.connected {
                    println!("  tx: {} bytes", peer.stats.tx_bytes);
                    println!("  rx: {} bytes", peer.stats.rx_bytes);
                    if let Some(hs) = peer.stats.last_handshake {
                        println!("  last handshake: {}s ago", hs.as_secs());
                    }
                }
                println!();
            }
            Ok(())
        }
        DaemonResponse::Error(e) => Err(e.into()),
        DaemonResponse::Ok => Err("unexpected response from daemon".into()),
    }
}

/// Start, stop, or daemonize the tunnel daemon.
fn cmd_daemon(
    config_path: &Path,
    daemonize: bool,
    stop: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if stop {
        return cmd_daemon_stop();
    }

    if daemonize {
        return cmd_daemon_background(config_path);
    }

    // Foreground mode.
    let app_config = load_app_config(config_path)?;
    let socket_path = PathBuf::from(SOCKET_PATH);

    let conn_names = app_config.connection_names();
    println!("Starting ferro-wg daemon (foreground)...");
    println!("Config: {}", config_path.display());
    println!("Socket: {}", socket_path.display());
    println!("Connections: {}", conn_names.join(", "));
    println!("Press Ctrl+C to stop.\n");

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(ferro_wg_core::daemon::run(app_config, &socket_path))?;
    Ok(())
}

/// Send a shutdown command to a running daemon.
fn cmd_daemon_stop() -> Result<(), Box<dyn std::error::Error>> {
    let response = daemon_command(&DaemonCommand::Shutdown)?;

    match response {
        DaemonResponse::Ok => {
            println!("Daemon stopped.");
            Ok(())
        }
        DaemonResponse::Error(e) => Err(e.into()),
        DaemonResponse::Status(_) => Err("unexpected response from daemon".into()),
    }
}

/// Spawn the daemon as a detached background process.
fn cmd_daemon_background(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;

    let child = std::process::Command::new("sudo")
        .arg(&exe)
        .arg("daemon")
        .arg("-c")
        .arg(config_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    println!("Daemon started in background (pid {}).", child.id());
    println!("Stop it with: ferro-wg daemon --stop");
    Ok(())
}

/// Import a `wg-quick` config as a named connection.
///
/// Each import creates a named connection with its own interface (private key)
/// and peer(s). The name is derived from the filename.
fn cmd_import(src_path: &Path, dst_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut wg_config = config::wg_quick::load_from_file(src_path)?;

    // Derive a connection name from the filename.
    let conn_name = src_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("default")
        .to_owned();

    // Name the peers within this connection.
    let peer_count = wg_config.peers.len();
    for (i, peer) in wg_config.peers.iter_mut().enumerate() {
        if peer.name.is_empty() {
            peer.name = if peer_count == 1 {
                conn_name.clone()
            } else {
                format!("{conn_name}-{i}")
            };
        }
    }

    // Load or create the AppConfig.
    let mut app_config = if dst_path.exists() {
        config::toml::load_app_config(dst_path)?
    } else {
        AppConfig::default()
    };

    // Check for existing connection with same name.
    if app_config.connections.contains_key(&conn_name) {
        eprintln!("Replacing existing connection: {conn_name}");
    }

    // Insert the connection.
    let endpoint = wg_config
        .peers
        .first()
        .and_then(|p| p.endpoint.as_deref())
        .unwrap_or("-");
    println!("  {conn_name} -> {endpoint}");

    app_config.insert(conn_name, wg_config);

    config::toml::save_app_config(&app_config, dst_path)?;
    println!("Saved to {}", dst_path.display());
    println!("Total connections: {}", app_config.connections.len());
    Ok(())
}

/// Generate and print an X25519 keypair.
fn cmd_genkey() {
    let private = PrivateKey::generate();
    let public = private.public_key();
    println!("private key: {}", private.to_base64());
    println!("public key:  {}", public.to_base64());
}

/// Load the [`AppConfig`] from disk.
fn load_app_config(path: &Path) -> Result<AppConfig, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Err(format!(
            "config not found: {}\n\n\
             Import a WireGuard config first:\n  \
             ferro-wg import <wg-quick.conf>",
            path.display()
        )
        .into());
    }
    let cfg = config::toml::load_app_config(path)?;
    Ok(cfg)
}
