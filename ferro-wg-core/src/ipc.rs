//! IPC protocol types for communication between the CLI/TUI and the daemon.
//!
//! Messages are serialized as newline-delimited JSON over a Unix domain socket.

use serde::{Deserialize, Serialize};

use crate::error::BackendKind;
use crate::stats::TunnelStats;

/// Default Unix socket path for the daemon.
pub const SOCKET_PATH: &str = "/tmp/ferro-wg.sock";

/// Commands sent from the CLI/TUI to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonCommand {
    /// Bring up tunnel(s). `None` means all configured peers.
    Up {
        /// Which peer to connect (by name). `None` = all.
        peer_name: Option<String>,
        /// Which backend to use.
        backend: BackendKind,
    },
    /// Tear down tunnel(s). `None` means all active peers.
    Down {
        /// Which peer to disconnect (by name). `None` = all.
        peer_name: Option<String>,
    },
    /// Request current status of all peers.
    Status,
    /// Switch a peer's backend (disconnects and reconnects).
    SwitchBackend {
        /// Peer name.
        peer_name: String,
        /// New backend to use.
        backend: BackendKind,
    },
    /// Ask the daemon to shut down cleanly.
    Shutdown,
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
}

/// Runtime status of a single peer, reported by the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerStatus {
    /// The peer's configured name.
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
            peer_name: Some("dc-mia".into()),
            backend: BackendKind::Boringtun,
        };
        let encoded = encode_message(&cmd).expect("encode");
        assert!(encoded.ends_with('\n'));
        let decoded: DaemonCommand = decode_message(&encoded).expect("decode");
        assert!(matches!(
            decoded,
            DaemonCommand::Up {
                peer_name: Some(ref n),
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
    fn all_commands_serialize() {
        let commands = vec![
            DaemonCommand::Up {
                peer_name: None,
                backend: BackendKind::Gotatun,
            },
            DaemonCommand::Down {
                peer_name: Some("test".into()),
            },
            DaemonCommand::Status,
            DaemonCommand::SwitchBackend {
                peer_name: "test".into(),
                backend: BackendKind::Neptun,
            },
            DaemonCommand::Shutdown,
        ];
        for cmd in &commands {
            let encoded = encode_message(cmd).expect("encode");
            let _: DaemonCommand = decode_message(&encoded).expect("decode");
        }
    }
}
