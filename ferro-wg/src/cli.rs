//! CLI argument definitions.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// `WireGuard` TUI and CLI with swappable backends for performance comparison.
#[derive(Debug, Parser)]
#[command(name = "ferro-wg", version, about)]
pub struct Cli {
    /// Path to the configuration file.
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    /// Increase log verbosity (-v, -vv, -vvv).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Subcommand to run (defaults to TUI if omitted).
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Available subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Launch the interactive TUI.
    Tui,

    /// Bring tunnel(s) up.
    Up {
        /// Peer name to connect (omit for all peers).
        peer: Option<String>,
    },

    /// Tear down tunnel(s).
    Down {
        /// Peer name to disconnect (omit for all peers).
        peer: Option<String>,
    },

    /// Print connection status.
    Status,

    /// Start the privileged tunnel daemon.
    Daemon {
        /// Run in the background (detach from terminal).
        #[arg(short, long)]
        daemonize: bool,

        /// Stop a running daemon.
        #[arg(short, long)]
        stop: bool,
    },

    /// Import a `wg-quick` configuration file.
    Import {
        /// Path to the `.conf` file.
        path: PathBuf,
    },

    /// Generate an X25519 keypair.
    Genkey,
}
