//! macOS network configuration: interface addresses and routes.
//!
//! These functions shell out to `ifconfig` and `route` to configure the
//! network stack. They require root privileges.
//!
//! DNS configuration is handled by [`super::dns`].

use std::process::Command;

use tracing::{debug, warn};

use crate::error::WgError;

/// Set the IPv4 address on a TUN interface.
///
/// Runs: `ifconfig {iface} inet {addr} {addr} up`
///
/// The address should be in CIDR notation (e.g. `172.31.250.32/32`).
/// The netmask is stripped and the address is used as both local and
/// destination (point-to-point).
///
/// # Errors
///
/// Returns [`WgError::Tunnel`] if the command fails.
pub fn set_interface_addr(iface: &str, addr_cidr: &str) -> Result<(), WgError> {
    let addr = addr_cidr.split('/').next().unwrap_or(addr_cidr);

    let output = Command::new("ifconfig")
        .args([iface, "inet", addr, addr, "up"])
        .output()
        .map_err(|e| WgError::Tunnel(format!("ifconfig exec failed: {e}")))?;

    if output.status.success() {
        debug!("Set {iface} address to {addr}");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(WgError::Tunnel(format!(
            "ifconfig {iface} inet {addr} failed: {stderr}"
        )))
    }
}

/// Add a route through the TUN interface.
///
/// Runs: `route -n add -net {cidr} -interface {iface}`
///
/// # Errors
///
/// Returns [`WgError::Tunnel`] if the command fails.
pub fn add_route(cidr: &str, iface: &str) -> Result<(), WgError> {
    let output = Command::new("route")
        .args(["-n", "add", "-net", cidr, "-interface", iface])
        .output()
        .map_err(|e| WgError::Tunnel(format!("route exec failed: {e}")))?;

    if output.status.success() {
        debug!("Added route {cidr} via {iface}");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Route already exists is not fatal.
        if stderr.contains("File exists") {
            warn!("Route {cidr} already exists, skipping");
            Ok(())
        } else {
            Err(WgError::Tunnel(format!(
                "route add {cidr} via {iface} failed: {stderr}"
            )))
        }
    }
}

/// Remove a route.
///
/// Runs: `route -n delete -net {cidr}`
///
/// # Errors
///
/// Returns [`WgError::Tunnel`] if the command fails.
pub fn remove_route(cidr: &str) -> Result<(), WgError> {
    let output = Command::new("route")
        .args(["-n", "delete", "-net", cidr])
        .output()
        .map_err(|e| WgError::Tunnel(format!("route exec failed: {e}")))?;

    if output.status.success() {
        debug!("Removed route {cidr}");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("not in table") {
            warn!("Route {cidr} not in table, skipping removal");
            Ok(())
        } else {
            Err(WgError::Tunnel(format!(
                "route delete {cidr} failed: {stderr}"
            )))
        }
    }
}
