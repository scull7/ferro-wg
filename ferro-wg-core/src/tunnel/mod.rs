//! Tunnel manager — owns TUN devices, UDP sockets, and `WireGuard` backends.
//!
//! The [`TunnelManager`] runs as part of the privileged daemon process,
//! creating per-peer packet loops that encrypt/decrypt traffic through
//! the configured [`WgBackend`](crate::backend::WgBackend).

pub mod route;
pub mod tun_device;
pub mod udp;

use std::collections::HashMap;
use std::net::SocketAddr;

use tokio::sync::{oneshot, watch};
use tokio::time::{self, Duration};
use tracing::{debug, error, info, warn};

use crate::backend::{self, PacketAction, TunnelConfig, WgBackend};
use crate::config::WgConfig;
use crate::error::{BackendKind, WgError};
use crate::ipc::PeerStatus;
use crate::stats::TunnelStats;

/// Size of packet buffers (MTU + `WireGuard` overhead).
const BUF_SIZE: usize = 65536;

/// Timer tick interval for keepalives and rekey.
const TICK_INTERVAL: Duration = Duration::from_millis(250);

/// macOS utun packet information header length.
/// When `packet_information(true)` is set, every TUN read/write has
/// a 4-byte prefix: `[0, 0, 0, AF]` where AF is `AF_INET` (2) or
/// `AF_INET6` (30).
#[cfg(target_os = "macos")]
const TUN_PI_HEADER_LEN: usize = 4;
#[cfg(not(target_os = "macos"))]
const TUN_PI_HEADER_LEN: usize = 0;

/// `AF_INET` for the macOS utun packet information header.
#[cfg(target_os = "macos")]
const AF_INET: u8 = 2;
/// `AF_INET6` for the macOS utun packet information header.
#[cfg(target_os = "macos")]
const AF_INET6: u8 = 30;

/// Manages active `WireGuard` tunnels for all configured peers.
pub struct TunnelManager {
    config: WgConfig,
    peers: HashMap<String, PeerTunnel>,
}

/// A single active peer tunnel.
struct PeerTunnel {
    /// Signal to shut down the packet loop task.
    shutdown_tx: oneshot::Sender<()>,
    /// Receive stats updates from the packet loop.
    stats_rx: watch::Receiver<TunnelStats>,
    /// Which backend is running.
    backend_kind: BackendKind,
    /// The TUN interface name (e.g. `utun4`).
    tun_name: String,
    /// The peer's endpoint string (stored for future re-resolution).
    _endpoint: Option<String>,
}

impl TunnelManager {
    /// Create a new tunnel manager from the given config.
    #[must_use]
    pub fn new(config: WgConfig) -> Self {
        Self {
            config,
            peers: HashMap::new(),
        }
    }

    /// Bring up a peer's tunnel.
    ///
    /// Creates a TUN device, resolves the endpoint, starts the packet loop.
    ///
    /// # Errors
    ///
    /// Returns an error if TUN creation, endpoint resolution, or backend
    /// construction fails.
    pub async fn up(&mut self, peer_name: &str, backend_kind: BackendKind) -> Result<(), WgError> {
        if self.peers.contains_key(peer_name) {
            return Err(WgError::Tunnel(format!("peer {peer_name} is already up")));
        }

        let peer_config = self
            .config
            .peers
            .iter()
            .find(|p| p.name == peer_name)
            .ok_or_else(|| WgError::Tunnel(format!("peer {peer_name} not found in config")))?
            .clone();

        let endpoint_str = peer_config
            .endpoint
            .as_deref()
            .ok_or_else(|| WgError::Tunnel(format!("peer {peer_name} has no endpoint")))?;

        // Resolve hostname to SocketAddr.
        let endpoint = udp::resolve_endpoint(endpoint_str).await?;
        info!("Resolved {endpoint_str} -> {endpoint}");

        // Create TUN device.
        let tun = tun_device::create_tun()?;
        let tun_name = tun_device::get_tun_name(&tun)?;
        info!("Created TUN device: {tun_name}");

        // Configure interface address and routes.
        for addr in &self.config.interface.addresses {
            route::set_interface_addr(&tun_name, addr)?;
        }
        for cidr in &peer_config.allowed_ips {
            route::add_route(cidr, &tun_name)?;
        }

        // Create UDP socket.
        let udp_socket = udp::create_udp_socket(self.config.interface.listen_port).await?;
        info!(
            "Bound UDP socket on port {}",
            self.config.interface.listen_port
        );

        // Create WireGuard backend.
        let tunnel_config = TunnelConfig {
            private_key: self.config.interface.private_key.clone(),
            peer_public_key: peer_config.public_key.clone(),
            preshared_key: peer_config.preshared_key.clone(),
            persistent_keepalive: if peer_config.persistent_keepalive == 0 {
                None
            } else {
                Some(peer_config.persistent_keepalive)
            },
            index: 0,
        };
        let backend = backend::create_backend(backend_kind, &tunnel_config)?;

        // Set up channels.
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (stats_tx, stats_rx) = watch::channel(TunnelStats::default());

        // Spawn the packet loop.
        let peer_name_owned = peer_name.to_owned();
        tokio::spawn(async move {
            if let Err(e) =
                packet_loop(tun, udp_socket, backend, endpoint, stats_tx, shutdown_rx).await
            {
                error!("Packet loop for {peer_name_owned} exited with error: {e}");
            } else {
                info!("Packet loop for {peer_name_owned} shut down cleanly");
            }
        });

        self.peers.insert(
            peer_name.to_owned(),
            PeerTunnel {
                shutdown_tx,
                stats_rx,
                backend_kind,
                tun_name,
                _endpoint: peer_config.endpoint.clone(),
            },
        );

        info!("Peer {peer_name} is up via {backend_kind}");
        Ok(())
    }

    /// Tear down a peer's tunnel.
    ///
    /// # Errors
    ///
    /// Returns [`WgError::Tunnel`] if the peer is not currently up.
    pub fn down(&mut self, peer_name: &str) -> Result<(), WgError> {
        let tunnel = self
            .peers
            .remove(peer_name)
            .ok_or_else(|| WgError::Tunnel(format!("peer {peer_name} is not up")))?;

        // Signal the packet loop to stop.
        let _ = tunnel.shutdown_tx.send(());

        // Remove routes.
        if let Some(peer_config) = self.config.peers.iter().find(|p| p.name == peer_name) {
            for cidr in &peer_config.allowed_ips {
                if let Err(e) = route::remove_route(cidr) {
                    warn!("Failed to remove route {cidr}: {e}");
                }
            }
        }

        info!("Peer {peer_name} is down");
        Ok(())
    }

    /// Bring up all configured peers.
    ///
    /// # Errors
    ///
    /// Returns the first error encountered; peers that were already up are skipped.
    pub async fn up_all(&mut self, backend_kind: BackendKind) -> Result<(), WgError> {
        let names: Vec<String> = self.config.peers.iter().map(|p| p.name.clone()).collect();
        for name in &names {
            if !self.peers.contains_key(name) {
                self.up(name, backend_kind).await?;
            }
        }
        Ok(())
    }

    /// Tear down all active peers.
    pub fn down_all(&mut self) {
        let names: Vec<String> = self.peers.keys().cloned().collect();
        for name in &names {
            if let Err(e) = self.down(name) {
                warn!("Failed to bring down {name}: {e}");
            }
        }
    }

    /// Get the current status of all configured peers.
    #[must_use]
    pub fn status(&self) -> Vec<PeerStatus> {
        self.config
            .peers
            .iter()
            .map(|pc| {
                let active = self.peers.get(&pc.name);
                PeerStatus {
                    name: pc.name.clone(),
                    connected: active.is_some(),
                    backend: active.map_or(BackendKind::Boringtun, |t| t.backend_kind),
                    stats: active
                        .map_or_else(TunnelStats::default, |t| t.stats_rx.borrow().clone()),
                    endpoint: pc.endpoint.clone(),
                    interface: active.map(|t| t.tun_name.clone()),
                }
            })
            .collect()
    }
}

impl Drop for TunnelManager {
    fn drop(&mut self) {
        self.down_all();
    }
}

/// The per-peer packet loop: TUN <-> `WgBackend` <-> UDP.
async fn packet_loop(
    tun: tun::AsyncDevice,
    udp: tokio::net::UdpSocket,
    mut backend: Box<dyn WgBackend>,
    endpoint: SocketAddr,
    stats_tx: watch::Sender<TunnelStats>,
    mut shutdown_rx: oneshot::Receiver<()>,
) -> Result<(), WgError> {
    let mut tun_buf = vec![0u8; BUF_SIZE];
    let mut udp_buf = vec![0u8; BUF_SIZE];
    let mut wg_buf = vec![0u8; BUF_SIZE];

    let mut tick_interval = time::interval(TICK_INTERVAL);
    tick_interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    // Initiate the first handshake.
    match backend.initiate_handshake(&mut wg_buf, true) {
        PacketAction::WriteToNetwork(len) => {
            if let Err(e) = udp.send_to(&wg_buf[..len], endpoint).await {
                warn!("Failed to send handshake init: {e}");
            } else {
                debug!("Sent handshake initiation to {endpoint}");
            }
        }
        PacketAction::Err(e) => warn!("Handshake initiation error: {e}"),
        _ => {}
    }

    loop {
        tokio::select! {
            // TUN -> encrypt -> UDP (outgoing user traffic)
            result = tun::AsyncDevice::recv(&tun, &mut tun_buf) => {
                match result {
                    Ok(n) if n > TUN_PI_HEADER_LEN => {
                        // Strip the macOS 4-byte packet info header;
                        // WgBackend expects a raw IP packet.
                        let ip_packet = &tun_buf[TUN_PI_HEADER_LEN..n];
                        handle_outgoing(
                            &mut backend, ip_packet, &mut wg_buf, &udp, endpoint
                        ).await;
                    }
                    Ok(_) => {} // too short or zero-length, ignore
                    Err(e) => {
                        debug!("TUN read error: {e}");
                    }
                }
            }

            // UDP -> decrypt -> TUN (incoming encrypted traffic)
            result = udp.recv_from(&mut udp_buf) => {
                match result {
                    Ok((n, addr)) => {
                        handle_incoming(
                            &mut backend, &udp_buf[..n], addr, &mut tun_buf,
                            &tun, &udp, endpoint,
                        ).await;
                    }
                    Err(e) => {
                        debug!("UDP recv error: {e}");
                    }
                }
            }

            // Timer tick: keepalives, rekey, stats broadcast
            _ = tick_interval.tick() => {
                // Run WireGuard timers.
                match backend.tick(&mut wg_buf) {
                    PacketAction::WriteToNetwork(len) => {
                        let _ = udp.send_to(&wg_buf[..len], endpoint).await;
                    }
                    PacketAction::Err(e) => debug!("Tick error: {e}"),
                    _ => {}
                }

                // Broadcast stats.
                let _ = stats_tx.send(backend.stats());
            }

            // Shutdown signal
            _ = &mut shutdown_rx => {
                info!("Packet loop received shutdown signal");
                break;
            }
        }
    }

    Ok(())
}

/// Handle an outgoing IP packet from TUN: encrypt and send via UDP.
async fn handle_outgoing(
    backend: &mut Box<dyn WgBackend>,
    ip_packet: &[u8],
    wg_buf: &mut [u8],
    udp: &tokio::net::UdpSocket,
    endpoint: SocketAddr,
) {
    match backend.encapsulate(ip_packet, wg_buf) {
        PacketAction::WriteToNetwork(len) => {
            if let Err(e) = udp.send_to(&wg_buf[..len], endpoint).await {
                debug!("UDP send error: {e}");
            }
        }
        PacketAction::Err(e) => debug!("Encapsulate error: {e}"),
        _ => {}
    }
}

/// Handle an incoming UDP datagram: decrypt and write to TUN.
async fn handle_incoming(
    backend: &mut Box<dyn WgBackend>,
    datagram: &[u8],
    src_addr: SocketAddr,
    tun_buf: &mut [u8],
    tun: &tun::AsyncDevice,
    udp: &tokio::net::UdpSocket,
    endpoint: SocketAddr,
) {
    // Reserve space for the macOS PI header at the start of tun_buf.
    // Decapsulate into tun_buf[PI_LEN..] so we can prepend the header.
    let dst = &mut tun_buf[TUN_PI_HEADER_LEN..];
    match backend.decapsulate(Some(src_addr), datagram, dst) {
        PacketAction::WriteToTun(len) => {
            // Prepend macOS utun packet information header.
            #[cfg(target_os = "macos")]
            {
                let ip_version = dst[0] >> 4;
                let af = if ip_version == 6 { AF_INET6 } else { AF_INET };
                tun_buf[0] = 0;
                tun_buf[1] = 0;
                tun_buf[2] = 0;
                tun_buf[3] = af;
            }
            let total = TUN_PI_HEADER_LEN + len;
            if let Err(e) = tun::AsyncDevice::send(tun, &tun_buf[..total]).await {
                debug!("TUN write error: {e}");
            }
        }
        PacketAction::WriteToNetwork(len) => {
            // Handshake response — decapsulate wrote to dst (tun_buf[PI_LEN..]).
            let start = TUN_PI_HEADER_LEN;
            let _ = udp.send_to(&tun_buf[start..start + len], endpoint).await;
        }
        PacketAction::Err(e) => debug!("Decapsulate error: {e}"),
        PacketAction::Done => {}
    }
}
