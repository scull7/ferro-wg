//! Async TUN device wrapper using the `tun` crate.
//!
//! Creates a macOS `utun` interface for routing IP packets through the tunnel.
//! Requires root privileges.

use tun::AbstractDevice;

use crate::error::WgError;

/// Create a new async TUN device.
///
/// On macOS, this creates a `utun` device with an auto-assigned name
/// (e.g. `utun4`, `utun5`). Requires root privileges.
///
/// # Errors
///
/// Returns [`WgError::Tunnel`] if the device cannot be created
/// (usually due to insufficient privileges).
pub fn create_tun() -> Result<tun::AsyncDevice, WgError> {
    // On macOS, don't set a name — the kernel auto-assigns utunN.
    // Setting "utun" without a number causes a parse error in the tun crate.
    #[cfg(target_os = "macos")]
    let config = {
        let mut c = tun::Configuration::default();
        c.platform_config(|p| {
            p.packet_information(true);
        });
        c
    };

    #[cfg(not(target_os = "macos"))]
    let config = tun::Configuration::default();

    tun::create_as_async(&config)
        .map_err(|e| WgError::Tunnel(format!("failed to create TUN device: {e}")))
}

/// Get the kernel-assigned name of a TUN device (e.g. `utun4`).
///
/// # Errors
///
/// Returns [`WgError::Tunnel`] if the name cannot be retrieved.
pub fn get_tun_name(device: &tun::AsyncDevice) -> Result<String, WgError> {
    device
        .tun_name()
        .map_err(|e| WgError::Tunnel(format!("failed to get TUN name: {e}")))
}
