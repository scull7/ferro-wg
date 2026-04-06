//! CLI argument definitions.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use ferro_wg_core::ipc::LogLevel;

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

    /// Show network routes for active tunnels.
    Routes,

    /// Import a `wg-quick` configuration file.
    Import {
        /// Path to the `.conf` file.
        path: PathBuf,
    },

    /// Generate an X25519 keypair.
    Genkey,

    /// Display logs from the daemon.
    Logs {
        /// Minimum log level to display.
        #[arg(long, default_value = "debug")]
        level: LogLevel,

        /// Filter by connection name.
        #[arg(long)]
        connection: Option<String>,

        /// Filter logs by search string.
        #[arg(long, default_value = "")]
        search: String,

        /// Number of lines to display.
        #[arg(long, default_value = "50")]
        lines: Option<usize>,

        /// Continuously watch for new logs.
        #[arg(long)]
        watch: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logs_command_default() {
        let cli = Cli::try_parse_from(["ferro-wg", "logs"]).unwrap();
        match cli.command.unwrap() {
            Command::Logs {
                level,
                connection,
                search,
                lines,
                watch,
            } => {
                assert_eq!(level, LogLevel::Debug);
                assert_eq!(connection, None);
                assert_eq!(search, "");
                assert_eq!(lines, Some(50));
                assert!(!watch);
            }
            _ => panic!("Expected Logs command"),
        }
    }

    #[test]
    fn test_logs_command_with_options() {
        let cli = Cli::try_parse_from([
            "ferro-wg",
            "logs",
            "--level",
            "info",
            "--connection",
            "myconn",
            "--search",
            "error",
            "--lines",
            "100",
            "--watch",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Command::Logs {
                level,
                connection,
                search,
                lines,
                watch,
            } => {
                assert_eq!(level, LogLevel::Info);
                assert_eq!(connection, Some("myconn".to_string()));
                assert_eq!(search, "error");
                assert_eq!(lines, Some(100));
                assert!(watch);
            }
            _ => panic!("Expected Logs command"),
        }
    }
}
