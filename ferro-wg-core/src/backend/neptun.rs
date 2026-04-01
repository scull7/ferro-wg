//! `neptun` backend adapter.
//!
//! Wraps `NordSecurity`'s [`neptun::noise::Tunn`] behind the [`WgBackend`](super::WgBackend)
//! trait. `neptun` is a fork of `boringtun` with a unified `WriteToTunnel` variant
//! (instead of separate V4/V6) and a fallible constructor.

use std::net::SocketAddr;
use std::sync::Arc;

use ::neptun::noise::rate_limiter::RateLimiter;
use ::neptun::noise::{Tunn, TunnResult};

use super::{PacketAction, TunnelConfig, WgBackend};
use crate::error::{BackendKind, WgError};
use crate::stats::TunnelStats;

/// Adapter wrapping a `neptun::noise::Tunn` instance.
pub struct NeptunBackend {
    tunn: Tunn,
    config: TunnelConfig,
}

impl NeptunBackend {
    /// Construct a new `neptun` tunnel from the given config.
    ///
    /// # Errors
    ///
    /// Returns [`WgError::Tunnel`] if the tunnel cannot be created.
    pub fn new(config: &TunnelConfig) -> Result<Self, WgError> {
        let static_private = config.private_key.to_static_secret();
        let peer_public = config.peer_public_key.to_x25519();
        let preshared_key = config.preshared_key.as_ref().map(|k| *k.as_bytes());

        let our_public = x25519_dalek::PublicKey::from(&static_private);
        let rate_limiter = Arc::new(RateLimiter::new(&our_public, 100));

        let tunn = Tunn::new(
            static_private,
            peer_public,
            preshared_key,
            config.persistent_keepalive,
            config.index,
            Some(rate_limiter),
        )
        .map_err(|e| WgError::Tunnel(e.to_string()))?;

        Ok(Self {
            tunn,
            config: config.clone(),
        })
    }
}

/// Convert a `neptun` `TunnResult` into our unified `PacketAction`.
///
/// Unlike `boringtun`, `neptun` uses a single `WriteToTunnel(buf, IpAddr)`
/// variant instead of separate V4/V6 variants.
fn tunn_result_to_action(result: TunnResult<'_>) -> PacketAction {
    match result {
        TunnResult::Done => PacketAction::Done,
        TunnResult::Err(e) => PacketAction::Err(format!("{e:?}")),
        TunnResult::WriteToNetwork(buf) => PacketAction::WriteToNetwork(buf.len()),
        TunnResult::WriteToTunnel(buf, _addr) => PacketAction::WriteToTun(buf.len()),
    }
}

impl WgBackend for NeptunBackend {
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
            tx_bytes: tx,
            rx_bytes: rx,
            last_handshake,
            packet_loss: loss,
            session_index: session,
        }
    }

    fn reset(&mut self) {
        let static_private = self.config.private_key.to_static_secret();
        let peer_public = self.config.peer_public_key.to_x25519();
        let preshared_key = self.config.preshared_key.as_ref().map(|k| *k.as_bytes());
        let our_public = x25519_dalek::PublicKey::from(&static_private);
        let rate_limiter = Arc::new(RateLimiter::new(&our_public, 100));

        if let Ok(new_tunn) = Tunn::new(
            static_private,
            peer_public,
            preshared_key,
            self.config.persistent_keepalive,
            self.config.index,
            Some(rate_limiter),
        ) {
            self.tunn = new_tunn;
        }
    }

    fn backend_name(&self) -> BackendKind {
        BackendKind::Neptun
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
        let backend = NeptunBackend::new(&config);
        assert!(backend.is_ok());
    }

    #[test]
    fn backend_name() {
        let config = make_config();
        let backend = NeptunBackend::new(&config).expect("create");
        assert_eq!(backend.backend_name(), BackendKind::Neptun);
    }

    #[test]
    fn initial_stats_are_zero() {
        let config = make_config();
        let backend = NeptunBackend::new(&config).expect("create");
        let stats = backend.stats();
        assert_eq!(stats.tx_bytes, 0);
        assert_eq!(stats.rx_bytes, 0);
        assert!(stats.last_handshake.is_none());
    }

    #[test]
    fn encapsulate_triggers_handshake() {
        let config = make_config();
        let mut backend = NeptunBackend::new(&config).expect("create");
        let src = [0u8; 64];
        let mut dst = [0u8; 256];
        let action = backend.encapsulate(&src, &mut dst);
        assert!(
            matches!(action, PacketAction::WriteToNetwork(_) | PacketAction::Done),
            "expected WriteToNetwork or Done, got {action:?}"
        );
    }

    #[test]
    fn initiate_handshake_produces_output() {
        let config = make_config();
        let mut backend = NeptunBackend::new(&config).expect("create");
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
        let mut backend = NeptunBackend::new(&config).expect("create");
        let mut dst = [0u8; 256];
        let _ = backend.initiate_handshake(&mut dst, true);
        backend.reset();
        let stats = backend.stats();
        assert_eq!(stats.tx_bytes, 0);
    }

    #[test]
    fn decapsulate_garbage() {
        let config = make_config();
        let mut backend = NeptunBackend::new(&config).expect("create");
        let garbage = [0xFF; 100];
        let mut dst = [0u8; 256];
        let action = backend.decapsulate(None, &garbage, &mut dst);
        assert!(matches!(action, PacketAction::Err(_) | PacketAction::Done));
    }
}
