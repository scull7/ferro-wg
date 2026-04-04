//! Tunnel manager — owns TUN devices, UDP sockets, and `WireGuard` backends.
//!
//! The [`TunnelManager`] runs as part of the privileged daemon process,
//! creating per-connection packet loops that encrypt/decrypt traffic through
//! the configured [`WgBackend`](crate::backend::WgBackend).

mod dns;
pub mod route;
pub mod tun_device;
pub mod udp;

use std::collections::HashMap;
use std::net::SocketAddr;

use tokio::sync::{oneshot, watch};
use tokio::time::{self, Duration};
use tracing::{debug, error, info, warn};

use crate::backend::{self, PacketAction, TunnelConfig, WgBackend};
use crate::config::AppConfig;
use crate::error::{BackendKind, WgError};
use crate::ipc::PeerStatus;
use crate::stats::TunnelStats;

/// Size of packet buffers (MTU + `WireGuard` overhead).
const BUF_SIZE: usize = 65536;

/// Timer tick interval for keepalives and rekey.
const TICK_INTERVAL: Duration = Duration::from_millis(250);

/// Manages active `WireGuard` tunnels for all configured connections.
pub struct TunnelManager {
    config: AppConfig,
    connections: HashMap<String, ActiveConnection>,
}

/// A single active connection tunnel.
struct ActiveConnection {
    /// Signal to shut down the packet loop task.
    shutdown_tx: oneshot::Sender<()>,
    /// Receive stats updates from the packet loop.
    stats_rx: watch::Receiver<TunnelStats>,
    /// Which backend is running.
    backend_kind: BackendKind,
    /// The TUN interface name (e.g. `utun4`).
    tun_name: String,
    /// The peer's endpoint string.
    endpoint: Option<String>,
    /// DNS state applied during `up()`; used to revert on `down()`.
    dns_state: Option<dns::DnsState>,
}

impl TunnelManager {
    /// Create a new tunnel manager from the given config.
    #[must_use]
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            connections: HashMap::new(),
        }
    }

    /// Replace the config with a new one (e.g. after re-reading from disk).
    ///
    /// Active connections are not affected — only future `up` calls and
    /// `status` queries will use the new config.
    pub fn reload_config(&mut self, config: AppConfig) {
        info!(
            "Reloaded config: {} connection(s)",
            config.connections.len()
        );
        self.config = config;
    }

    /// Bring up a named connection.
    ///
    /// Creates a TUN device, resolves the endpoint, starts the packet loop.
    ///
    /// # Errors
    ///
    /// Returns an error if TUN creation, endpoint resolution, or backend
    /// construction fails.
    pub async fn up(&mut self, conn_name: &str, backend_kind: BackendKind) -> Result<(), WgError> {
        if self.connections.contains_key(conn_name) {
            return Err(WgError::Tunnel(format!(
                "connection {conn_name} is already up"
            )));
        }

        let wg_config = self
            .config
            .get(conn_name)
            .ok_or_else(|| WgError::Tunnel(format!("connection {conn_name} not found in config")))?
            .clone();

        // Use the first peer (each wg-quick import has one peer).
        let peer_config = wg_config
            .peers
            .first()
            .ok_or_else(|| WgError::Tunnel(format!("connection {conn_name} has no peers")))?
            .clone();

        let endpoint_str = peer_config
            .endpoint
            .as_deref()
            .ok_or_else(|| WgError::Tunnel(format!("connection {conn_name} has no endpoint")))?;

        // Resolve hostname to SocketAddr.
        let endpoint = udp::resolve_endpoint(endpoint_str).await?;
        info!("Resolved {endpoint_str} -> {endpoint}");

        // Create TUN device.
        let tun = tun_device::create_tun()?;
        let tun_name = tun_device::get_tun_name(&tun)?;
        info!("Created TUN device: {tun_name}");

        // Configure interface address and routes.
        for addr in &wg_config.interface.addresses {
            route::set_interface_addr(&tun_name, addr)?;
        }
        for cidr in &peer_config.allowed_ips {
            route::add_route(cidr, &tun_name)?;
        }

        // Apply DNS configuration (non-fatal: a failure logs a warning but
        // does not abort the tunnel bring-up).
        let dns_state = dns::apply_dns(
            &tun_name,
            &wg_config.interface.dns,
            &wg_config.interface.dns_search,
        )
        .unwrap_or_else(|e| {
            warn!("Failed to apply DNS for {conn_name}: {e}");
            None
        });

        // Create UDP socket.
        let udp_socket = udp::create_udp_socket(wg_config.interface.listen_port).await?;
        info!(
            "Bound UDP socket on port {}",
            wg_config.interface.listen_port
        );

        // Create WireGuard backend.
        let tunnel_config = TunnelConfig {
            private_key: wg_config.interface.private_key.clone(),
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
        let name_owned = conn_name.to_owned();
        tokio::spawn(async move {
            if let Err(e) =
                packet_loop(tun, udp_socket, backend, endpoint, stats_tx, shutdown_rx).await
            {
                error!("Packet loop for {name_owned} exited with error: {e}");
            } else {
                info!("Packet loop for {name_owned} shut down cleanly");
            }
        });

        self.connections.insert(
            conn_name.to_owned(),
            ActiveConnection {
                shutdown_tx,
                stats_rx,
                backend_kind,
                tun_name,
                endpoint: peer_config.endpoint.clone(),
                dns_state,
            },
        );

        info!("Connection {conn_name} is up via {backend_kind}");
        Ok(())
    }

    /// Tear down a named connection.
    ///
    /// # Errors
    ///
    /// Returns [`WgError::Tunnel`] if the connection is not currently up.
    pub fn down(&mut self, conn_name: &str) -> Result<(), WgError> {
        let conn = self
            .connections
            .remove(conn_name)
            .ok_or_else(|| WgError::Tunnel(format!("connection {conn_name} is not up")))?;

        // Signal the packet loop to stop.
        let _ = conn.shutdown_tx.send(());

        // Remove routes.
        if let Some(wg_config) = self.config.get(conn_name) {
            for peer in &wg_config.peers {
                for cidr in &peer.allowed_ips {
                    if let Err(e) = route::remove_route(cidr) {
                        warn!("Failed to remove route {cidr}: {e}");
                    }
                }
            }
        }

        // Revert DNS configuration — but only when no other active connection
        // has DNS applied.  DNS is system-wide on macOS (bound to the primary
        // network service) and global on Linux (/etc/resolv.conf fallback), so
        // reverting while another tunnel is still up would break its DNS.
        if let Some(state) = conn.dns_state {
            let other_dns_active = self.connections.values().any(|c| c.dns_state.is_some());
            if other_dns_active {
                warn!(
                    "Skipping DNS revert for {conn_name}: \
                     another active connection has DNS applied"
                );
            } else if let Err(e) = dns::remove_dns(state) {
                warn!("Failed to remove DNS for {conn_name}: {e}");
            }
        }

        info!("Connection {conn_name} is down");
        Ok(())
    }

    /// Bring up all configured connections.
    ///
    /// # Errors
    ///
    /// Returns the first error encountered; connections already up are skipped.
    pub async fn up_all(&mut self, backend_kind: BackendKind) -> Result<(), WgError> {
        let names: Vec<String> = self
            .config
            .connection_names()
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        for name in &names {
            if !self.connections.contains_key(name) {
                self.up(name, backend_kind).await?;
            }
        }
        Ok(())
    }

    /// Tear down all active connections.
    pub fn down_all(&mut self) {
        let names: Vec<String> = self.connections.keys().cloned().collect();
        for name in &names {
            if let Err(e) = self.down(name) {
                warn!("Failed to bring down {name}: {e}");
            }
        }
    }

    /// Get the current status of all configured connections.
    #[must_use]
    pub fn status(&self) -> Vec<PeerStatus> {
        self.config
            .connections
            .iter()
            .map(|(name, wg_config)| {
                let active = self.connections.get(name);
                let first_peer = wg_config.peers.first();
                PeerStatus {
                    name: name.clone(),
                    connected: active.is_some(),
                    backend: active.map_or(BackendKind::Boringtun, |c| c.backend_kind),
                    stats: active
                        .map_or_else(TunnelStats::default, |c| c.stats_rx.borrow().clone()),
                    endpoint: active
                        .and_then(|c| c.endpoint.clone())
                        .or_else(|| first_peer.and_then(|p| p.endpoint.clone())),
                    interface: active.map(|c| c.tun_name.clone()),
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

/// The per-connection packet loop: TUN <-> `WgBackend` <-> UDP.
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
            // Note: the `tun` crate handles PI header stripping internally
            // when `packet_information(true)` is set.
            result = tun::AsyncDevice::recv(&tun, &mut tun_buf) => {
                match result {
                    Ok(n) if n > 0 => {
                        handle_outgoing(
                            &mut backend, &tun_buf[..n], &mut wg_buf, &udp, endpoint
                        ).await;
                    }
                    Ok(_) => {}
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
    // The `tun` crate handles PI header addition internally on send,
    // so we pass raw IP packets directly.
    match backend.decapsulate(Some(src_addr), datagram, tun_buf) {
        PacketAction::WriteToTun(len) => {
            if let Err(e) = tun::AsyncDevice::send(tun, &tun_buf[..len]).await {
                debug!("TUN write error: {e}");
            }
        }
        PacketAction::WriteToNetwork(len) => {
            // Handshake response — send back via UDP.
            let _ = udp.send_to(&tun_buf[..len], endpoint).await;
        }
        PacketAction::Err(e) => debug!("Decapsulate error: {e}"),
        PacketAction::Done => {}
    }
}
