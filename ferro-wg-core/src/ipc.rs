//! IPC protocol types for communication between the CLI/TUI and the daemon.
//!
//! Messages are serialized as newline-delimited JSON over a Unix domain socket.

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::error::BackendKind;
use crate::stats::{BenchmarkResult, TunnelStats};

/// Default Unix socket path for the daemon.
pub const SOCKET_PATH: &str = "/tmp/ferro-wg.sock";

/// Severity level of a daemon log entry.
///
/// Variants are declared in ascending severity order so that `PartialOrd`/`Ord`
/// derived impls satisfy `Trace < Debug < Info < Warn < Error`.  The TUI filter
/// threshold cycles through `Debug → Info → Warn → Error → Debug` (Trace is
/// deliberately excluded from the cycle — it always passes the filter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LogLevel {
    /// Most verbose; always shown regardless of the filter threshold.
    Trace,
    /// Diagnostic detail; the default filter threshold.
    Debug,
    /// Informational messages about normal operation.
    Info,
    /// Potentially unexpected conditions that do not stop operation.
    Warn,
    /// Error conditions.
    Error,
}

impl LogLevel {
    /// Advance to the next stricter filter threshold, wrapping `Error` back to
    /// `Debug`.  `Trace` is not part of the cycle.
    ///
    /// Cycle order: `Debug → Info → Warn → Error → Debug`.
    #[must_use]
    pub fn cycle(self) -> Self {
        match self {
            Self::Trace | Self::Debug => Self::Info,
            Self::Info => Self::Warn,
            Self::Warn => Self::Error,
            Self::Error => Self::Debug,
        }
    }

    /// Short label used in the Logs block title, e.g. `"INFO+"`.
    #[must_use]
    pub fn title_label(self) -> &'static str {
        match self {
            Self::Trace => "TRACE+",
            Self::Debug => "DEBUG+",
            Self::Info => "INFO+",
            Self::Warn => "WARN+",
            Self::Error => "ERROR",
        }
    }

    /// Short badge label used in log line rendering, e.g. `"INFO"`.
    #[must_use]
    pub fn badge(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// A single structured log event emitted by the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Milliseconds since the Unix epoch (UTC).
    ///
    /// Using `i64` avoids a `chrono` dependency at the serde boundary and keeps
    /// the wire format simple.  The TUI converts to local time for display.
    pub timestamp_ms: i64,
    /// Severity level.
    pub level: LogLevel,
    /// Connection this event belongs to, if any.  `None` means a global daemon
    /// event not tied to a specific tunnel — global events always pass
    /// connection filters.
    pub connection_name: Option<String>,
    /// Formatted log message, typically `"target: text"`.
    pub message: String,
}

impl LogEntry {
    /// Construct a [`LogEntry`] timestamped at the current wall-clock time.
    #[must_use]
    pub fn now(level: LogLevel, connection_name: Option<String>, message: String) -> Self {
        Self {
            timestamp_ms: Local::now().timestamp_millis(),
            level,
            connection_name,
            message,
        }
    }

    /// Format `timestamp_ms` as `"HH:MM:SS"` in the local timezone.
    ///
    /// Returns `"??:??:??"` when `timestamp_ms` is outside the range that
    /// [`chrono`] can represent, rather than panicking.
    #[must_use]
    pub fn time_label(&self) -> String {
        DateTime::from_timestamp_millis(self.timestamp_ms).map_or_else(
            || "??:??:??".to_owned(),
            |utc| {
                let local: DateTime<Local> = utc.with_timezone(&Local);
                local.format("%H:%M:%S").to_string()
            },
        )
    }
}

/// Commands sent from the CLI/TUI to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonCommand {
    /// Bring up tunnel(s). `None` means all configured connections.
    Up {
        /// Which connection to bring up (by name). `None` = all.
        connection_name: Option<String>,
        /// Which backend to use.
        backend: BackendKind,
    },
    /// Tear down tunnel(s). `None` means all active connections.
    Down {
        /// Which connection to tear down (by name). `None` = all.
        connection_name: Option<String>,
    },
    /// Request current status of all connections.
    Status,
    /// Switch a connection's backend (disconnects and reconnects).
    SwitchBackend {
        /// Connection name.
        connection_name: String,
        /// New backend to use.
        backend: BackendKind,
    },
    /// Ask the daemon to shut down cleanly.
    Shutdown,
    /// Request to stream daemon logs in real-time.
    StreamLogs,
    /// Run a performance benchmark against the named connection.
    ///
    /// The daemon streams [`DaemonResponse::BenchmarkProgress`] updates
    /// approximately once per second, then sends a final
    /// [`DaemonResponse::BenchmarkResult`] when the run completes.
    Benchmark {
        /// Which connection to benchmark.
        connection_name: String,
        /// Benchmark duration in seconds (default: 10).
        duration_secs: u32,
    },
}

/// Responses sent from the daemon to the CLI/TUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonResponse {
    /// Command succeeded with no additional data.
    Ok,
    /// Command failed.
    Error(String),
    /// Current status of all peers.
    Status(Vec<PeerStatus>),
    /// A single structured log entry pushed from an active [`StreamLogs`] subscription.
    ///
    /// [`StreamLogs`]: DaemonCommand::StreamLogs
    LogEntry(LogEntry),
    /// Periodic progress update during an active benchmark run.
    BenchmarkProgress(BenchmarkProgress),
    /// Final aggregated result after a benchmark run completes.
    BenchmarkResult(BenchmarkResult),
}

/// Runtime status of a single connection, reported by the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerStatus {
    /// The connection's configured name (matches `AppConfig` connection keys).
    pub name: String,
    /// Whether the tunnel is connected.
    pub connected: bool,
    /// Which backend is active.
    pub backend: BackendKind,
    /// Current tunnel statistics.
    pub stats: TunnelStats,
    /// The peer's endpoint (hostname:port or ip:port).
    pub endpoint: Option<String>,
    /// The local TUN interface name (e.g. `utun4`).
    pub interface: Option<String>,
}

/// Periodic benchmark progress update streamed from the daemon.
///
/// `BenchmarkProgress` is a wire-format type: it travels over the IPC socket
/// from daemon to TUI as a `DaemonResponse::BenchmarkProgress` variant.
/// It lives in `ipc.rs` alongside the other wire types, **not** in `stats.rs`
/// (which holds post-processed aggregation results).
///
/// Sent roughly once per second during an active `DaemonCommand::Benchmark`
/// run. The TUI drives the `Sparkline` and progress bar from these updates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkProgress {
    /// Backend under test.
    pub backend: String,
    /// Seconds elapsed since the benchmark started.
    pub elapsed_secs: u32,
    /// Configured total duration.
    pub total_secs: u32,
    /// Instantaneous throughput at this sample point.
    pub current_throughput_bps: f64,
    /// Cumulative packets processed so far.
    pub packets_processed: u64,
}

/// Encode a message as a newline-terminated JSON string.
///
/// # Errors
///
/// Returns a serialization error if the value cannot be encoded.
pub fn encode_message<T: Serialize>(msg: &T) -> Result<String, serde_json::Error> {
    let mut json = serde_json::to_string(msg)?;
    json.push('\n');
    Ok(json)
}

/// Decode a message from a JSON string (with or without trailing newline).
///
/// # Errors
///
/// Returns a deserialization error if the string is not valid JSON.
pub fn decode_message<T: for<'de> Deserialize<'de>>(json: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(json.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_roundtrip() {
        let cmd = DaemonCommand::Up {
            connection_name: Some("dc-mia".into()),
            backend: BackendKind::Boringtun,
        };
        let encoded = encode_message(&cmd).expect("encode");
        assert!(encoded.ends_with('\n'));
        let decoded: DaemonCommand = decode_message(&encoded).expect("decode");
        assert!(matches!(
            decoded,
            DaemonCommand::Up {
                connection_name: Some(ref n),
                backend: BackendKind::Boringtun,
            } if n == "dc-mia"
        ));
    }

    #[test]
    fn response_ok_roundtrip() {
        let resp = DaemonResponse::Ok;
        let encoded = encode_message(&resp).expect("encode");
        let decoded: DaemonResponse = decode_message(&encoded).expect("decode");
        assert!(matches!(decoded, DaemonResponse::Ok));
    }

    #[test]
    fn response_status_roundtrip() {
        let resp = DaemonResponse::Status(vec![PeerStatus {
            name: "mia".into(),
            connected: true,
            backend: BackendKind::Neptun,
            stats: TunnelStats::default(),
            endpoint: Some("vpn.example.com:51820".into()),
            interface: Some("utun4".into()),
        }]);
        let encoded = encode_message(&resp).expect("encode");
        let decoded: DaemonResponse = decode_message(&encoded).expect("decode");
        if let DaemonResponse::Status(peers) = decoded {
            assert_eq!(peers.len(), 1);
            assert_eq!(peers[0].name, "mia");
            assert!(peers[0].connected);
            assert_eq!(peers[0].interface.as_deref(), Some("utun4"));
        } else {
            panic!("expected Status response");
        }
    }

    #[test]
    fn response_error_roundtrip() {
        let resp = DaemonResponse::Error("no such peer".into());
        let encoded = encode_message(&resp).expect("encode");
        let decoded: DaemonResponse = decode_message(&encoded).expect("decode");
        assert!(matches!(decoded, DaemonResponse::Error(ref s) if s == "no such peer"));
    }

    #[test]
    fn benchmark_progress_roundtrip() {
        let progress = BenchmarkProgress {
            backend: "boringtun".into(),
            elapsed_secs: 5,
            total_secs: 10,
            current_throughput_bps: 1_234_567.0,
            packets_processed: 12345,
        };
        let resp = DaemonResponse::BenchmarkProgress(progress.clone());
        let encoded = encode_message(&resp).expect("encode");
        let decoded: DaemonResponse = decode_message(&encoded).expect("decode");
        assert!(matches!(decoded, DaemonResponse::BenchmarkProgress(ref p) if *p == progress));
    }

    #[test]
    fn benchmark_result_roundtrip() {
        let result = BenchmarkResult {
            backend: "boringtun".into(),
            packets_processed: 1000,
            bytes_encapsulated: 1_000_000,
            elapsed: std::time::Duration::from_secs(1),
            throughput_bps: 1_000_000.0,
            avg_latency: std::time::Duration::from_micros(10),
            ..BenchmarkResult::default()
        };
        let resp = DaemonResponse::BenchmarkResult(result.clone());
        let encoded = encode_message(&resp).expect("encode");
        let decoded: DaemonResponse = decode_message(&encoded).expect("decode");
        assert!(matches!(decoded, DaemonResponse::BenchmarkResult(ref r) if *r == result));
    }

    #[test]
    fn log_level_ordering() {
        assert!(LogLevel::Trace < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Error);
    }

    #[test]
    fn log_level_cycle_wraps() {
        assert_eq!(LogLevel::Debug.cycle(), LogLevel::Info);
        assert_eq!(LogLevel::Info.cycle(), LogLevel::Warn);
        assert_eq!(LogLevel::Warn.cycle(), LogLevel::Error);
        assert_eq!(LogLevel::Error.cycle(), LogLevel::Debug);
        // Trace is not part of the UI cycle — treated same as Debug
        assert_eq!(LogLevel::Trace.cycle(), LogLevel::Info);
    }

    #[test]
    fn logentry_roundtrip_no_connection() {
        let entry = LogEntry {
            timestamp_ms: 1_712_231_696_000,
            level: LogLevel::Info,
            connection_name: None,
            message: "ferro_wg_daemon::server: Listening on /tmp/ferro-wg.sock".into(),
        };
        let resp = DaemonResponse::LogEntry(entry.clone());
        let encoded = encode_message(&resp).expect("encode");
        let decoded: DaemonResponse = decode_message(&encoded).expect("decode");
        assert!(matches!(decoded, DaemonResponse::LogEntry(ref e) if *e == entry));
    }

    #[test]
    fn logentry_roundtrip_with_connection() {
        let entry = LogEntry {
            timestamp_ms: 1_712_231_696_123,
            level: LogLevel::Warn,
            connection_name: Some("mia".into()),
            message: "ferro_wg_core::tunnel: Handshake timeout".into(),
        };
        let resp = DaemonResponse::LogEntry(entry.clone());
        let encoded = encode_message(&resp).expect("encode");
        let decoded: DaemonResponse = decode_message(&encoded).expect("decode");
        assert!(matches!(decoded, DaemonResponse::LogEntry(ref e) if *e == entry));
    }

    #[test]
    fn logentry_time_label_invalid_timestamp() {
        // i64::MAX is outside chrono's representable range — should not panic.
        let entry = LogEntry {
            timestamp_ms: i64::MAX,
            level: LogLevel::Debug,
            connection_name: None,
            message: "test".into(),
        };
        assert_eq!(entry.time_label(), "??:??:??");
    }

    #[test]
    fn logentry_now_has_recent_timestamp() {
        let before = Local::now().timestamp_millis();
        let entry = LogEntry::now(LogLevel::Info, None, "msg".into());
        let after = Local::now().timestamp_millis();
        assert!(entry.timestamp_ms >= before);
        assert!(entry.timestamp_ms <= after);
    }

    #[test]
    fn all_commands_serialize() {
        let commands = vec![
            DaemonCommand::Up {
                connection_name: None,
                backend: BackendKind::Gotatun,
            },
            DaemonCommand::Down {
                connection_name: Some("test".into()),
            },
            DaemonCommand::Status,
            DaemonCommand::SwitchBackend {
                connection_name: "test".into(),
                backend: BackendKind::Neptun,
            },
            DaemonCommand::Shutdown,
            DaemonCommand::StreamLogs,
            DaemonCommand::Benchmark {
                connection_name: "test".into(),
                duration_secs: 10,
            },
        ];
        for cmd in &commands {
            let encoded = encode_message(cmd).expect("encode");
            let _: DaemonCommand = decode_message(&encoded).expect("decode");
        }
    }
}
