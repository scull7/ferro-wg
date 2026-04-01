//! ferro-wg: `WireGuard` TUI and CLI entry point.

mod cli;
#[cfg(feature = "tui")]
mod tui;

use std::path::{Path, PathBuf};

use clap::Parser;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Command};
use ferro_wg_core::config;
use ferro_wg_core::key::PrivateKey;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Configure logging based on verbosity.
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

    match cli.command {
        // Default to TUI when no subcommand given.
        None | Some(Command::Tui) => run_tui(&config_path),
        Some(Command::Up { peer }) => cmd_up(&config_path, peer.as_deref()),
        Some(Command::Down { peer }) => cmd_down(&config_path, peer.as_deref()),
        Some(Command::Status) => cmd_status(&config_path),
        Some(Command::Import { path }) => cmd_import(&path, &config_path),
        Some(Command::Genkey) => {
            cmd_genkey();
            Ok(())
        }
    }
}

/// Default config file location: `~/.config/ferro-wg/config.toml`.
fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ferro-wg")
        .join("config.toml")
}

/// Launch the interactive TUI.
fn run_tui(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "tui")]
    {
        let wg_config = load_config(config_path)?;
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(tui::run(wg_config))?;
        Ok(())
    }
    #[cfg(not(feature = "tui"))]
    {
        let _ = config_path;
        Err("TUI feature not enabled. Rebuild with --features tui".into())
    }
}

/// Bring tunnel(s) up (stub -- requires `TunnelManager`).
fn cmd_up(config_path: &Path, peer: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let wg_config = load_config(config_path)?;
    let target = peer.unwrap_or("all peers");
    println!("Bringing up: {target}");
    println!(
        "Configured peers: {}",
        wg_config
            .peers
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("(tunnel manager not yet implemented)");
    Ok(())
}

/// Tear down tunnel(s) (stub).
fn cmd_down(config_path: &Path, peer: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let _wg_config = load_config(config_path)?;
    let target = peer.unwrap_or("all peers");
    println!("Tearing down: {target}");
    println!("(tunnel manager not yet implemented)");
    Ok(())
}

/// Print connection status (stub).
fn cmd_status(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let wg_config = load_config(config_path)?;
    println!("interface: ferro-wg");
    println!(
        "  public key: {}",
        wg_config.interface.private_key.public_key().to_base64()
    );
    println!("  listening port: {}", wg_config.interface.listen_port);
    println!();
    for peer in &wg_config.peers {
        let name = if peer.name.is_empty() {
            "(unnamed)"
        } else {
            &peer.name
        };
        println!("peer: {name}");
        println!("  public key: {}", peer.public_key.to_base64());
        if let Some(ep) = &peer.endpoint {
            println!("  endpoint: {ep}");
        }
        println!("  allowed ips: {}", peer.allowed_ips.join(", "));
        println!("  status: disconnected");
        println!();
    }
    Ok(())
}

/// Import a `wg-quick` config into native TOML format.
///
/// Derives peer names from the source filename when the `wg-quick`
/// format doesn't include one (which is always — `wg-quick` has no name field).
fn cmd_import(src_path: &Path, dst_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut wg_config = config::wg_quick::load_from_file(src_path)?;

    // Derive a base name from the filename (e.g. "MIA_nathan_tensorwave_com").
    let base_name = src_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("peer");

    // Assign names to unnamed peers.
    let peer_count = wg_config.peers.len();
    for (i, peer) in wg_config.peers.iter_mut().enumerate() {
        if peer.name.is_empty() {
            peer.name = if peer_count == 1 {
                base_name.to_owned()
            } else {
                format!("{base_name}-{i}")
            };
        }
    }

    println!(
        "Imported {} peer(s) from {}",
        wg_config.peers.len(),
        src_path.display()
    );
    for peer in &wg_config.peers {
        let endpoint = peer.endpoint.as_deref().unwrap_or("-");
        println!("  {} -> {}", peer.name, endpoint);
    }
    config::toml::save_to_file(&wg_config, dst_path)?;
    println!("Saved to {}", dst_path.display());
    Ok(())
}

/// Generate and print an X25519 keypair.
fn cmd_genkey() {
    let private = PrivateKey::generate();
    let public = private.public_key();
    println!("private key: {}", private.to_base64());
    println!("public key:  {}", public.to_base64());
}

/// Load the `WireGuard` configuration from disk.
fn load_config(path: &Path) -> Result<ferro_wg_core::config::WgConfig, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Err(format!(
            "config file not found: {}\nRun `ferro-wg import <wg-quick.conf>` or create one manually.",
            path.display()
        )
        .into());
    }
    let cfg = config::toml::load_from_file(path)?;
    Ok(cfg)
}
