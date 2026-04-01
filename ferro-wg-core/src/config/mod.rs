//! `WireGuard` configuration types.
//!
//! These types represent the unified configuration model shared across
//! all config sources (native TOML, `wg-quick` import, API fetch). Parsers
//! in submodules convert from their respective formats into these types.

pub mod toml;
pub mod wg_quick;

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::key::{PresharedKey, PrivateKey, PublicKey};

/// Complete `WireGuard` interface configuration (our side of the tunnel).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceConfig {
    /// Our private key.
    pub private_key: PrivateKey,
    /// UDP listen port (0 = random).
    #[serde(default)]
    pub listen_port: u16,
    /// Local tunnel addresses (e.g. `10.0.0.2/24`).
    #[serde(default)]
    pub addresses: Vec<String>,
    /// DNS servers to use when the tunnel is active.
    #[serde(default)]
    pub dns: Vec<IpAddr>,
    /// Maximum transmission unit (0 = auto).
    #[serde(default)]
    pub mtu: u16,
    /// Firewall mark for outgoing packets (Linux only).
    #[serde(default)]
    pub fwmark: u32,
    /// Commands to run before bringing the interface up.
    #[serde(default)]
    pub pre_up: Vec<String>,
    /// Commands to run after bringing the interface up.
    #[serde(default)]
    pub post_up: Vec<String>,
    /// Commands to run before tearing the interface down.
    #[serde(default)]
    pub pre_down: Vec<String>,
    /// Commands to run after tearing the interface down.
    #[serde(default)]
    pub post_down: Vec<String>,
}

/// A single `WireGuard` peer configuration (remote side).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerConfig {
    /// Human-readable name for this peer (not part of the WG protocol).
    #[serde(default)]
    pub name: String,
    /// The peer's public key.
    pub public_key: PublicKey,
    /// Optional preshared key for additional symmetric encryption.
    #[serde(default)]
    pub preshared_key: Option<PresharedKey>,
    /// The peer's endpoint (`host:port`). Supports both IP addresses and
    /// hostnames (resolved at connection time). `None` for receive-only peers.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// IP ranges to route through this peer (CIDR notation).
    #[serde(default)]
    pub allowed_ips: Vec<String>,
    /// Send keepalive packets every N seconds (0 = disabled).
    #[serde(default)]
    pub persistent_keepalive: u16,
}

/// A full `WireGuard` configuration file (interface + peers).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WgConfig {
    /// Interface (our side) configuration.
    pub interface: InterfaceConfig,
    /// Peer configurations.
    #[serde(default)]
    pub peers: Vec<PeerConfig>,
}

impl WgConfig {
    /// Validate the configuration, returning the first error found.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::MissingField`] if no peers are configured,
    /// or [`ConfigError::InvalidValue`] if a peer has no allowed IPs.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.peers.is_empty() {
            return Err(ConfigError::MissingField("peers"));
        }
        for (i, peer) in self.peers.iter().enumerate() {
            if peer.allowed_ips.is_empty() {
                return Err(ConfigError::InvalidValue {
                    field: "allowed_ips",
                    reason: format!("peer {i} has no allowed IPs"),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> WgConfig {
        WgConfig {
            interface: InterfaceConfig {
                private_key: PrivateKey::generate(),
                listen_port: 51820,
                addresses: vec!["10.0.0.2/24".into()],
                dns: vec!["1.1.1.1".parse().expect("dns")],
                mtu: 1420,
                fwmark: 0,
                pre_up: Vec::new(),
                post_up: Vec::new(),
                pre_down: Vec::new(),
                post_down: Vec::new(),
            },
            peers: vec![PeerConfig {
                name: "tw-dc-sjc01".into(),
                public_key: PrivateKey::generate().public_key(),
                preshared_key: None,
                endpoint: Some("198.51.100.1:51820".into()),
                allowed_ips: vec!["10.100.0.0/16".into()],
                persistent_keepalive: 25,
            }],
        }
    }

    #[test]
    fn valid_config_passes_validation() {
        let cfg = sample_config();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn empty_peers_fails_validation() {
        let mut cfg = sample_config();
        cfg.peers.clear();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, ConfigError::MissingField("peers")));
    }

    #[test]
    fn peer_without_allowed_ips_fails() {
        let mut cfg = sample_config();
        cfg.peers[0].allowed_ips.clear();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidValue {
                field: "allowed_ips",
                ..
            }
        ));
    }

    #[test]
    fn config_toml_roundtrip() {
        let cfg = sample_config();
        let toml_str = self::toml::save_to_string(&cfg).expect("serialize");
        let back = self::toml::load_from_str(&toml_str).expect("deserialize");
        assert_eq!(back.interface.listen_port, 51820);
        assert_eq!(back.peers.len(), 1);
        assert_eq!(back.peers[0].name, "tw-dc-sjc01");
        assert_eq!(back.peers[0].persistent_keepalive, 25);
    }

    #[test]
    fn peer_config_optional_fields() {
        let peer = PeerConfig {
            name: String::new(),
            public_key: PrivateKey::generate().public_key(),
            preshared_key: None,
            endpoint: None,
            allowed_ips: vec!["0.0.0.0/0".into()],
            persistent_keepalive: 0,
        };
        assert!(peer.endpoint.is_none());
        assert!(peer.preshared_key.is_none());
    }
}
