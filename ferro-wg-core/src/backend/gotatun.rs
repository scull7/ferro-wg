//! `gotatun` backend adapter.
//!
//! Wraps Mullvad's [`gotatun::noise::Tunn`] behind the [`WgBackend`](super::WgBackend)
//! trait. Unlike `boringtun`/`neptun`, `gotatun` uses owned `Packet` types
//! (backed by the `bytes` crate) rather than borrowed buffer slices, so
//! this adapter copies between the caller's buffers and owned packets.

use std::net::SocketAddr;
use std::sync::Arc;

use ::gotatun::noise::index_table::IndexTable;
use ::gotatun::noise::rate_limiter::RateLimiter;
use ::gotatun::noise::{Tunn, TunnResult};
use ::gotatun::packet::{Packet, WgKind};
use ::gotatun::x25519;
use bytes::BytesMut;

use super::{PacketAction, TunnelConfig, WgBackend};
use crate::error::{BackendKind, WgError};
use crate::stats::TunnelStats;

/// Adapter wrapping a `gotatun::noise::Tunn` instance.
pub struct GotatunBackend {
    tunn: Tunn,
    config: TunnelConfig,
    index_table: IndexTable,
}

impl GotatunBackend {
    /// Construct a new `gotatun` tunnel from the given config.
    ///
    /// # Errors
    ///
    /// Returns [`WgError::Tunnel`] if the tunnel cannot be created.
    pub fn new(config: &TunnelConfig) -> Result<Self, WgError> {
        let static_private = x25519::StaticSecret::from(*config.private_key.as_bytes());
        let peer_public = x25519::PublicKey::from(*config.peer_public_key.as_bytes());

        let our_public = x25519::PublicKey::from(&static_private);
        let index_table = IndexTable::from_os_rng();
        let rate_limiter = Arc::new(RateLimiter::new(&our_public, 100));

        let tunn = Tunn::new(
            static_private,
            peer_public,
            config.preshared_key.as_ref().map(|k| *k.as_bytes()),
            config.persistent_keepalive,
            index_table.clone(),
            rate_limiter,
        );

        Ok(Self {
            tunn,
            config: config.clone(),
            index_table,
        })
    }
}

/// Copy bytes from a `WgKind` into a caller-provided buffer.
///
/// Converts the owned `WgKind` to a `Packet<[u8]>`, then copies bytes
/// into `dst`, returning the number of bytes written.
fn wg_kind_to_dst(wg_kind: WgKind, dst: &mut [u8]) -> usize {
    let packet: Packet = wg_kind.into();
    let raw = packet.into_bytes();
    let data: &[u8] = &raw;
    let len = data.len().min(dst.len());
    dst[..len].copy_from_slice(&data[..len]);
    len
}

/// Copy bytes from a `Packet` into a caller-provided buffer.
fn packet_to_dst(packet: Packet, dst: &mut [u8]) -> usize {
    let raw = packet.into_bytes();
    let data: &[u8] = &raw;
    let len = data.len().min(dst.len());
    dst[..len].copy_from_slice(&data[..len]);
    len
}

impl WgBackend for GotatunBackend {
    fn encapsulate(&mut self, src: &[u8], dst: &mut [u8]) -> PacketAction {
        let packet = Packet::from_bytes(BytesMut::from(src));
        match self.tunn.handle_outgoing_packet(packet, None) {
            Some(wg_kind) => PacketAction::WriteToNetwork(wg_kind_to_dst(wg_kind, dst)),
            None => PacketAction::Done,
        }
    }

    fn decapsulate(
        &mut self,
        _src_addr: Option<SocketAddr>,
        datagram: &[u8],
        dst: &mut [u8],
    ) -> PacketAction {
        let packet = Packet::from_bytes(BytesMut::from(datagram));
        let wg_kind = match packet.try_into_wg() {
            Ok(kind) => kind,
            Err(e) => return PacketAction::Err(format!("parse WG packet: {e}")),
        };

        match self.tunn.handle_incoming_packet(wg_kind) {
            TunnResult::Done => PacketAction::Done,
            TunnResult::Err(e) => PacketAction::Err(format!("{e:?}")),
            TunnResult::WriteToNetwork(kind) => {
                PacketAction::WriteToNetwork(wg_kind_to_dst(kind, dst))
            }
            TunnResult::WriteToTunnel(pkt) => PacketAction::WriteToTun(packet_to_dst(pkt, dst)),
        }
    }

    fn initiate_handshake(&mut self, dst: &mut [u8], force: bool) -> PacketAction {
        match self.tunn.format_handshake_initiation(force) {
            Some(hs_packet) => {
                // Handshake init packet: convert from Packet<WgHandshakeInit> to raw bytes.
                let raw = hs_packet.into_bytes();
                let data: &[u8] = &raw;
                let len = data.len().min(dst.len());
                dst[..len].copy_from_slice(&data[..len]);
                PacketAction::WriteToNetwork(len)
            }
            None => PacketAction::Done,
        }
    }

    fn tick(&mut self, dst: &mut [u8]) -> PacketAction {
        match self.tunn.update_timers() {
            Ok(Some(wg_kind)) => PacketAction::WriteToNetwork(wg_kind_to_dst(wg_kind, dst)),
            Ok(None) => PacketAction::Done,
            Err(e) => PacketAction::Err(format!("{e:?}")),
        }
    }

    fn stats(&self) -> TunnelStats {
        let (last_handshake, tx, rx, loss, session) = self.tunn.stats();
        #[allow(clippy::cast_possible_truncation)]
        TunnelStats {
            tx_bytes: tx as u64,
            rx_bytes: rx as u64,
            last_handshake,
            packet_loss: loss,
            session_index: session,
        }
    }

    fn reset(&mut self) {
        let static_private = x25519::StaticSecret::from(*self.config.private_key.as_bytes());
        let peer_public = x25519::PublicKey::from(*self.config.peer_public_key.as_bytes());
        let our_public = x25519::PublicKey::from(&static_private);
        let rate_limiter = Arc::new(RateLimiter::new(&our_public, 100));

        self.tunn = Tunn::new(
            static_private,
            peer_public,
            self.config.preshared_key.as_ref().map(|k| *k.as_bytes()),
            self.config.persistent_keepalive,
            self.index_table.clone(),
            rate_limiter,
        );
    }

    fn backend_name(&self) -> BackendKind {
        BackendKind::Gotatun
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
        assert!(GotatunBackend::new(&config).is_ok());
    }

    #[test]
    fn backend_name() {
        let config = make_config();
        let backend = GotatunBackend::new(&config).expect("create");
        assert_eq!(backend.backend_name(), BackendKind::Gotatun);
    }

    #[test]
    fn initial_stats_are_zero() {
        let config = make_config();
        let backend = GotatunBackend::new(&config).expect("create");
        let stats = backend.stats();
        assert_eq!(stats.tx_bytes, 0);
        assert_eq!(stats.rx_bytes, 0);
        assert!(stats.last_handshake.is_none());
    }

    #[test]
    fn encapsulate_triggers_handshake() {
        let config = make_config();
        let mut backend = GotatunBackend::new(&config).expect("create");
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
        let mut backend = GotatunBackend::new(&config).expect("create");
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
        let mut backend = GotatunBackend::new(&config).expect("create");
        let mut dst = [0u8; 256];
        let _ = backend.initiate_handshake(&mut dst, true);
        backend.reset();
        let stats = backend.stats();
        assert_eq!(stats.tx_bytes, 0);
    }

    #[test]
    fn decapsulate_garbage() {
        let config = make_config();
        let mut backend = GotatunBackend::new(&config).expect("create");
        let garbage = [0xFF; 100];
        let mut dst = [0u8; 256];
        let action = backend.decapsulate(None, &garbage, &mut dst);
        assert!(matches!(action, PacketAction::Err(_) | PacketAction::Done));
    }
}
