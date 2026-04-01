//! Async UDP socket with hostname resolution.
//!
//! `WireGuard` endpoints can be hostnames (e.g. `wireguard.vpn.mia.tensorwaveops.com:51821`).
//! This module resolves them to `SocketAddr` at connection time.

use std::net::SocketAddr;

use tokio::net::UdpSocket;

use crate::error::WgError;

/// Resolve an endpoint string (`host:port`) to a `SocketAddr`.
///
/// Supports both IP addresses and hostnames. For hostnames, uses
/// the system DNS resolver via `tokio::net::lookup_host`.
///
/// # Errors
///
/// Returns [`WgError::Tunnel`] if DNS resolution fails or no addresses
/// are returned.
pub async fn resolve_endpoint(endpoint: &str) -> Result<SocketAddr, WgError> {
    let mut addrs = tokio::net::lookup_host(endpoint)
        .await
        .map_err(|e| WgError::Tunnel(format!("failed to resolve endpoint {endpoint}: {e}")))?;

    addrs
        .next()
        .ok_or_else(|| WgError::Tunnel(format!("no addresses found for {endpoint}")))
}

/// Create an async UDP socket bound to the given port.
///
/// Port 0 lets the OS assign a random ephemeral port.
///
/// # Errors
///
/// Returns [`WgError::Tunnel`] if the socket cannot be bound.
pub async fn create_udp_socket(port: u16) -> Result<UdpSocket, WgError> {
    let bind_addr: SocketAddr = format!("0.0.0.0:{port}")
        .parse()
        .map_err(|e| WgError::Tunnel(format!("invalid bind address: {e}")))?;

    UdpSocket::bind(bind_addr)
        .await
        .map_err(|e| WgError::Tunnel(format!("failed to bind UDP socket on port {port}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolve_ip_endpoint() {
        let addr = resolve_endpoint("127.0.0.1:51820").await.expect("resolve");
        assert_eq!(addr.port(), 51820);
        assert!(addr.ip().is_loopback());
    }

    #[tokio::test]
    async fn resolve_hostname_endpoint() {
        let addr = resolve_endpoint("localhost:51820").await.expect("resolve");
        assert_eq!(addr.port(), 51820);
    }

    #[tokio::test]
    async fn resolve_invalid_endpoint() {
        let result = resolve_endpoint("not-a-real-host-xyz.invalid:51820").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn create_ephemeral_socket() {
        let sock = create_udp_socket(0).await.expect("bind");
        let local = sock.local_addr().expect("local addr");
        assert_ne!(local.port(), 0); // OS assigned a port
    }
}
