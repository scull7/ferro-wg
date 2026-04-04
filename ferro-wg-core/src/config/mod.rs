//! `WireGuard` configuration types.
//!
//! These types represent the unified configuration model shared across
//! all config sources (native TOML, `wg-quick` import, API fetch). Parsers
//! in submodules convert from their respective formats into these types.

pub mod toml;
pub mod wg_quick;

use std::collections::BTreeMap;
use std::collections::BTreeMap;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::key::{PresharedKey, PrivateKey, PublicKey};

/// Maximum allowed length for a connection or peer name.
const MAX_NAME_LEN: usize = 64;

/// Top-level application config: a map of named connections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    /// All configured connections, keyed by name.
    pub connections: BTreeMap<String, Connection>,

    /// Log display configuration.
    #[serde(default)]
    pub log_display: LogDisplayConfig,
}

impl AppConfig {
    /// Validate all connections.
    ///
    /// # Errors
    ///
    /// Returns the first validation error found, prefixed with the
    /// connection name.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.connections.is_empty() {
            return Err(ConfigError::MissingField("connections"));
        }
        for (name, conn) in &self.connections {
            validate_name("connection name", name).map_err(|e| ConfigError::InvalidValue {
                field: "connection name",
                reason: format!("{name}: {e}"),
            })?;
            conn.validate().map_err(|e| ConfigError::InvalidValue {
                field: "connections",
                reason: format!("{name}: {e}"),
            })?;
        }
        Ok(())
    }

    /// Get a connection by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&WgConfig> {
        self.connections.get(name)
    }

    /// List all connection names.
    #[must_use]
    pub fn connection_names(&self) -> Vec<&str> {
        self.connections.keys().map(String::as_str).collect()
    }

    /// Insert or replace a named connection.
    pub fn insert(&mut self, name: String, config: WgConfig) {
        self.connections.insert(name, config);
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
                dns_search: Vec::new(),
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

    // ── validate_name unit tests ──────────────────────────────────────────────

    #[test]
    fn validate_name_accepts_valid_names() {
        for name in &["mia", "ord01", "tus1", "my-vpn", "vpn_home", "A1-B2_C3"] {
            assert!(
                validate_name("test", name).is_ok(),
                "expected {name:?} to be valid"
            );
        }
    }

    #[test]
    fn validate_name_rejects_empty() {
        let err = validate_name("test", "").unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidValue { field: "test", .. }
        ));
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn validate_name_rejects_too_long() {
        let long = "a".repeat(MAX_NAME_LEN + 1);
        let err = validate_name("test", &long).unwrap_err();
        assert!(err.to_string().contains("maximum"));
    }

    #[test]
    fn validate_name_rejects_invalid_chars() {
        for name in &["has space", "has/slash", "has.dot", "has@at", "has!bang"] {
            let err = validate_name("test", name).unwrap_err();
            assert!(
                err.to_string().contains("invalid character"),
                "expected invalid-char error for {name:?}, got: {err}"
            );
        }
    }

    #[test]
    fn validate_name_rejects_unicode() {
        let err = validate_name("test", "café").unwrap_err();
        assert!(err.to_string().contains("invalid character"));
    }

    // ── AppConfig connection-name validation ──────────────────────────────────

    fn app_config_with_name(name: &str) -> AppConfig {
        let mut connections = std::collections::BTreeMap::new();
        connections.insert(name.to_string(), sample_config());
        AppConfig { connections }
    }

    #[test]
    fn app_config_valid_connection_name_passes() {
        assert!(app_config_with_name("mia").validate().is_ok());
    }

    #[test]
    fn app_config_invalid_connection_name_fails() {
        let err = app_config_with_name("bad name!").validate().unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidValue {
                field: "connection name",
                ..
            }
        ));
    }

    #[test]
    fn app_config_overlong_connection_name_fails() {
        let err = app_config_with_name(&"x".repeat(MAX_NAME_LEN + 1))
            .validate()
            .unwrap_err();
        assert!(err.to_string().contains("maximum"));
    }

    // ── WgConfig peer-name validation ─────────────────────────────────────────

    #[test]
    fn wg_config_invalid_peer_name_fails() {
        let mut cfg = sample_config();
        cfg.peers[0].name = "bad peer!".into();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidValue {
                field: "peer.name",
                ..
            }
        ));
    }

    #[test]
    fn wg_config_empty_peer_name_passes() {
        // Empty peer name is allowed (name is optional in the protocol).
        let mut cfg = sample_config();
        cfg.peers[0].name = String::new();
        assert!(cfg.validate().is_ok());
    }
}
