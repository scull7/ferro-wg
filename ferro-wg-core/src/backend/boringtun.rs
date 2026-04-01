//! `boringtun` backend adapter.
//!
//! Wraps Cloudflare's [`boringtun::noise::Tunn`] behind the [`WgBackend`](super::WgBackend)
//! trait. The `boringtun` API uses borrowed output buffers (`&'a mut [u8]`)
//! returning [`TunnResult`] variants that reference the caller's buffer.

use std::net::SocketAddr;
use std::sync::Arc;

use ::boringtun::noise::rate_limiter::RateLimiter;
use ::boringtun::noise::{Tunn, TunnResult};

use super::{PacketAction, TunnelConfig, WgBackend};
use crate::error::{BackendKind, WgError};
use crate::stats::TunnelStats;

/// Adapter wrapping a `boringtun::noise::Tunn` instance.
pub struct BoringtunBackend {
    tunn: Tunn,
    config: TunnelConfig,
}

impl BoringtunBackend {
    /// Construct a new `boringtun` tunnel from the given config.
    ///
    /// # Errors
    ///
    /// Returns [`WgError::Tunnel`] if the tunnel cannot be created.
    pub fn new(config: &TunnelConfig) -> Result<Self, WgError> {
        let static_private = config.private_key.to_static_secret();
        let peer_public = config.peer_public_key.to_x25519();
        let preshared_key = config.preshared_key.as_ref().map(|k| *k.as_bytes());

        let rate_limiter = Arc::new(RateLimiter::new(&peer_public, 100));

        let tunn = Tunn::new(
            static_private,
            peer_public,
            preshared_key,
            config.persistent_keepalive,
            config.index,
            Some(rate_limiter),
        );

        Ok(Self {
            tunn,
            config: config.clone(),
        })
    }
}

/// Convert a `boringtun` `TunnResult` into our unified `PacketAction`.
///
/// This helper is also used by the `neptun` adapter since they share similar
/// return types.
pub(crate) fn tunn_result_to_action(result: TunnResult<'_>) -> PacketAction {
    match result {
        TunnResult::Done => PacketAction::Done,
        TunnResult::Err(e) => PacketAction::Err(format!("{e:?}")),
        TunnResult::WriteToNetwork(buf) => PacketAction::WriteToNetwork(buf.len()),
        TunnResult::WriteToTunnelV4(buf, _) | TunnResult::WriteToTunnelV6(buf, _) => {
            PacketAction::WriteToTun(buf.len())
        }
    }
}

impl WgBackend for BoringtunBackend {
    fn encapsulate(&mut self, src: &[u8], dst: &mut [u8]) -> PacketAction {
        tunn_result_to_action(self.tunn.encapsulate(src, dst))
    }

    fn decapsulate(
        &mut self,
        src_addr: Option<SocketAddr>,
        datagram: &[u8],
        dst: &mut [u8],
    ) -> PacketAction {
        let ip_addr = src_addr.map(|s| s.ip());
        tunn_result_to_action(self.tunn.decapsulate(ip_addr, datagram, dst))
    }

    fn initiate_handshake(&mut self, dst: &mut [u8], force: bool) -> PacketAction {
        tunn_result_to_action(self.tunn.format_handshake_initiation(dst, force))
    }

    fn tick(&mut self, dst: &mut [u8]) -> PacketAction {
        tunn_result_to_action(self.tunn.update_timers(dst))
    }

    fn stats(&self) -> TunnelStats {
        let (last_handshake, tx, rx, loss, session) = self.tunn.stats();
        TunnelStats {
            tx_bytes: tx as u64,
            rx_bytes: rx as u64,
            last_handshake,
            packet_loss: loss,
            session_index: session,
        }
    }

    fn reset(&mut self) {
        // Recreate the tunnel with the same config to reset sessions.
        let static_private = self.config.private_key.to_static_secret();
        let peer_public = self.config.peer_public_key.to_x25519();
        let preshared_key = self.config.preshared_key.as_ref().map(|k| *k.as_bytes());
        let rate_limiter = Arc::new(RateLimiter::new(&peer_public, 100));

        self.tunn = Tunn::new(
            static_private,
            peer_public,
            preshared_key,
            self.config.persistent_keepalive,
            self.config.index,
            Some(rate_limiter),
        );
    }

    fn backend_name(&self) -> BackendKind {
        BackendKind::Boringtun
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::PrivateKey;

    fn make_config() -> TunnelConfig {
        TunnelConfig {
            private_key: PrivateKey::generate(),
            peer_public_key: PrivateKey::generate().public_key(),
            preshared_key: None,
            persistent_keepalive: Some(25),
            index: 0,
        }
    }

    #[test]
    fn create_tunnel() {
        let config = make_config();
        let backend = BoringtunBackend::new(&config);
        assert!(backend.is_ok());
    }

    #[test]
    fn backend_name() {
        let config = make_config();
        let backend = BoringtunBackend::new(&config).expect("create");
        assert_eq!(backend.backend_name(), BackendKind::Boringtun);
    }

    #[test]
    fn initial_stats_are_zero() {
        let config = make_config();
        let backend = BoringtunBackend::new(&config).expect("create");
        let stats = backend.stats();
        assert_eq!(stats.tx_bytes, 0);
        assert_eq!(stats.rx_bytes, 0);
        assert!(stats.last_handshake.is_none());
    }

    #[test]
    fn encapsulate_without_handshake_returns_handshake_init() {
        let config = make_config();
        let mut backend = BoringtunBackend::new(&config).expect("create");

        // An IP packet before handshake should trigger a handshake initiation.
        let src = [0u8; 64]; // dummy IP packet
        let mut dst = [0u8; 256];
        let action = backend.encapsulate(&src, &mut dst);

        // Before a handshake is complete, boringtun queues the packet and
        // sends a handshake init instead.
        assert!(
            matches!(action, PacketAction::WriteToNetwork(_) | PacketAction::Done),
            "expected WriteToNetwork or Done, got {action:?}"
        );
    }

    #[test]
    fn initiate_handshake_produces_output() {
        let config = make_config();
        let mut backend = BoringtunBackend::new(&config).expect("create");
        let mut dst = [0u8; 256];
        let action = backend.initiate_handshake(&mut dst, true);
        assert!(
            matches!(action, PacketAction::WriteToNetwork(_)),
            "expected WriteToNetwork, got {action:?}"
        );
    }

    #[test]
    fn reset_clears_state() {
        let config = make_config();
        let mut backend = BoringtunBackend::new(&config).expect("create");

        // Force a handshake to change state.
        let mut dst = [0u8; 256];
        let _ = backend.initiate_handshake(&mut dst, true);

        backend.reset();
        let stats = backend.stats();
        assert_eq!(stats.tx_bytes, 0);
    }

    #[test]
    fn tick_without_handshake() {
        let config = make_config();
        let mut backend = BoringtunBackend::new(&config).expect("create");
        let mut dst = [0u8; 256];
        let action = backend.tick(&mut dst);
        // Before any handshake, tick may produce a handshake init or Done.
        assert!(matches!(
            action,
            PacketAction::Done | PacketAction::WriteToNetwork(_)
        ));
    }

    #[test]
    fn decapsulate_garbage_returns_error() {
        let config = make_config();
        let mut backend = BoringtunBackend::new(&config).expect("create");
        let garbage = [0xFF; 100];
        let mut dst = [0u8; 256];
        let action = backend.decapsulate(None, &garbage, &mut dst);
        // Garbage input should produce an error or Done (invalid packet type).
        assert!(matches!(action, PacketAction::Err(_) | PacketAction::Done));
    }
}
