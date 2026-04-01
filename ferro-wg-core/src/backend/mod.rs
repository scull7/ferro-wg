//! `WireGuard` backend trait and packet action types.
//!
//! The [`WgBackend`] trait provides a synchronous, buffer-oriented interface
//! that normalizes the API differences between `boringtun`, `neptun`, and `gotatun`.
//! Async I/O (TUN device, UDP sockets) lives in the tunnel manager layer,
//! keeping the backend trait focused on pure packet cryptography.

#[cfg(feature = "boringtun")]
pub mod boringtun;
#[cfg(feature = "gotatun")]
pub mod gotatun;
#[cfg(feature = "neptun")]
pub mod neptun;

use std::net::SocketAddr;

use crate::error::{BackendKind, WgError};
use crate::key::{PresharedKey, PrivateKey, PublicKey};
use crate::stats::TunnelStats;

/// Configuration required to construct a tunnel via any backend.
#[derive(Debug, Clone)]
pub struct TunnelConfig {
    /// Our private key.
    pub private_key: PrivateKey,
    /// The peer's public key.
    pub peer_public_key: PublicKey,
    /// Optional preshared key for post-quantum resistance.
    pub preshared_key: Option<PresharedKey>,
    /// Send keepalive every N seconds (0 = disabled).
    pub persistent_keepalive: Option<u16>,
    /// Tunnel index (used by the protocol for session multiplexing).
    pub index: u32,
}

/// Result of a packet operation (encapsulate, decapsulate, tick).
///
/// The `usize` payload is the number of valid bytes written into the
/// caller-provided destination buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PacketAction {
    /// Decrypted IP packet written to `dst`; forward to the TUN device.
    WriteToTun(usize),
    /// Encrypted `WireGuard` packet written to `dst`; send over UDP.
    WriteToNetwork(usize),
    /// No output produced.
    Done,
    /// A protocol-level error occurred.
    Err(String),
}

/// Common interface for userspace `WireGuard` implementations.
///
/// All methods are synchronous and operate on caller-provided buffers to
/// avoid allocation in the packet processing hot path. The destination
/// buffer must be at least `src.len() + 32` bytes (minimum 148 bytes)
/// to accommodate `WireGuard` overhead.
pub trait WgBackend: Send {
    /// Encrypt an outgoing IP packet into a `WireGuard` UDP datagram.
    ///
    /// - `src`: plaintext IP packet from the TUN device
    /// - `dst`: buffer for the encrypted output
    fn encapsulate(&mut self, src: &[u8], dst: &mut [u8]) -> PacketAction;

    /// Decrypt an incoming `WireGuard` UDP datagram into an IP packet.
    ///
    /// - `src_addr`: the source address of the UDP datagram (for handshake validation)
    /// - `datagram`: the raw `WireGuard` packet from the network
    /// - `dst`: buffer for the decrypted output
    fn decapsulate(
        &mut self,
        src_addr: Option<SocketAddr>,
        datagram: &[u8],
        dst: &mut [u8],
    ) -> PacketAction;

    /// Generate a handshake initiation message.
    ///
    /// - `dst`: buffer for the handshake packet
    /// - `force`: if true, send even if a recent handshake exists
    fn initiate_handshake(&mut self, dst: &mut [u8], force: bool) -> PacketAction;

    /// Run timer-driven maintenance (retransmits, keepalives, expiry).
    ///
    /// Should be called on a regular interval (typically every 250ms).
    fn tick(&mut self, dst: &mut [u8]) -> PacketAction;

    /// Snapshot of current tunnel statistics.
    fn stats(&self) -> TunnelStats;

    /// Tear down active sessions but keep configuration.
    fn reset(&mut self);

    /// Human-readable backend identifier.
    fn backend_name(&self) -> BackendKind;
}

/// Create a backend by kind, dispatching to the appropriate feature-gated constructor.
///
/// # Errors
///
/// Returns [`WgError::BackendUnavailable`] if the requested backend's
/// cargo feature is not enabled, or a backend-specific error if construction fails.
pub fn create_backend(
    kind: BackendKind,
    config: &TunnelConfig,
) -> Result<Box<dyn WgBackend>, WgError> {
    // Suppress unused warning when no backend features are enabled.
    let _ = config;

    match kind {
        #[cfg(feature = "boringtun")]
        BackendKind::Boringtun => {
            let backend = self::boringtun::BoringtunBackend::new(config)?;
            Ok(Box::new(backend))
        }
        #[cfg(feature = "neptun")]
        BackendKind::Neptun => {
            let backend = self::neptun::NeptunBackend::new(config)?;
            Ok(Box::new(backend))
        }
        #[cfg(feature = "gotatun")]
        BackendKind::Gotatun => {
            let backend = self::gotatun::GotatunBackend::new(config)?;
            Ok(Box::new(backend))
        }
        #[allow(unreachable_patterns)]
        other => Err(WgError::BackendUnavailable(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> TunnelConfig {
        TunnelConfig {
            private_key: PrivateKey::generate(),
            peer_public_key: PrivateKey::generate().public_key(),
            preshared_key: None,
            persistent_keepalive: Some(25),
            index: 0,
        }
    }

    #[test]
    fn packet_action_variants() {
        let write_tun = PacketAction::WriteToTun(100);
        let write_net = PacketAction::WriteToNetwork(200);
        let done = PacketAction::Done;
        let err = PacketAction::Err("timeout".into());

        assert_eq!(write_tun, PacketAction::WriteToTun(100));
        assert_eq!(write_net, PacketAction::WriteToNetwork(200));
        assert_eq!(done, PacketAction::Done);
        assert!(matches!(err, PacketAction::Err(ref s) if s == "timeout"));
    }

    #[test]
    fn tunnel_config_debug_redacts_keys() {
        let cfg = sample_config();
        let debug = format!("{cfg:?}");
        // Private key Debug impl says [REDACTED].
        assert!(debug.contains("REDACTED"));
    }

    #[test]
    fn unavailable_backend_returns_error() {
        // When no features are enabled, all backends should be unavailable.
        // This test is meaningful when run without --features.
        #[cfg(not(any(feature = "boringtun", feature = "neptun", feature = "gotatun")))]
        {
            let cfg = sample_config();
            let result = create_backend(BackendKind::Boringtun, &cfg);
            assert!(matches!(result, Err(WgError::BackendUnavailable(_))));
        }
    }
}
