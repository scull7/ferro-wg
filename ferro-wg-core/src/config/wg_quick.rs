//! Parser for `wg-quick`-style `.conf` files.
//!
//! Converts the INI-like format used by `wg-quick(8)` into our native
//! [`WgConfig`] type. Supports all standard `[Interface]` and `[Peer]`
//! directives including `PreUp`/`PostUp`/`PreDown`/`PostDown`.

use std::net::IpAddr;
use std::path::Path;

use crate::config::{InterfaceConfig, PeerConfig, WgConfig};
use crate::error::ConfigError;
use crate::key::{PresharedKey, PrivateKey, PublicKey};

/// Which INI section we are currently parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Interface,
    Peer,
}

/// Parse a `wg-quick` config from a file path.
///
/// # Errors
///
/// Returns [`ConfigError::WgQuickParse`] on syntax errors, or
/// [`ConfigError::MissingField`] if required fields are absent.
pub fn load_from_file(path: &Path) -> Result<WgConfig, ConfigError> {
    let contents =
        std::fs::read_to_string(path).map_err(|e| ConfigError::TomlParse(e.to_string()))?;
    load_from_str(&contents)
}

/// Parse a `wg-quick` config from a string.
///
/// # Errors
///
/// Returns [`ConfigError::WgQuickParse`] on syntax errors, or
/// [`ConfigError::MissingField`] if required fields are absent.
pub fn load_from_str(input: &str) -> Result<WgConfig, ConfigError> {
    let mut section = Section::None;
    let mut iface = InterfaceBuilder::default();
    let mut peers: Vec<PeerBuilder> = Vec::new();

    for (line_idx, raw_line) in input.lines().enumerate() {
        let line_num = line_idx + 1;
        let line = raw_line.trim();

        // Skip empty lines and comments.
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        // Section headers.
        if line.eq_ignore_ascii_case("[interface]") {
            section = Section::Interface;
            continue;
        }
        if line.eq_ignore_ascii_case("[peer]") {
            section = Section::Peer;
            peers.push(PeerBuilder::default());
            continue;
        }

        // Key = Value pairs.
        let (key, value) = split_kv(line, line_num)?;

        match section {
            Section::None => {
                return Err(ConfigError::WgQuickParse {
                    line: line_num,
                    reason: "key-value pair outside of a section".into(),
                });
            }
            Section::Interface => parse_interface_kv(&mut iface, key, value, line_num)?,
            Section::Peer => {
                // Safety: we always push a PeerBuilder when we see [Peer].
                let Some(peer) = peers.last_mut() else {
                    unreachable!("peer vec is non-empty after [Peer] header");
                };
                parse_peer_kv(peer, key, value, line_num)?;
            }
        }
    }

    let config = WgConfig {
        interface: iface.build()?,
        peers: peers
            .into_iter()
            .map(PeerBuilder::build)
            .collect::<Result<Vec<_>, _>>()?,
    };
    config.validate()?;
    Ok(config)
}

/// Export a [`WgConfig`] to `wg-quick` format.
#[must_use]
pub fn export_to_string(config: &WgConfig) -> String {
    use std::fmt::Write;

    let mut out = String::with_capacity(512);
    out.push_str("[Interface]\n");
    let _ = writeln!(
        out,
        "PrivateKey = {}",
        config.interface.private_key.to_base64()
    );
    if config.interface.listen_port != 0 {
        let _ = writeln!(out, "ListenPort = {}", config.interface.listen_port);
    }
    for addr in &config.interface.addresses {
        let _ = writeln!(out, "Address = {addr}");
    }
    if !config.interface.dns.is_empty() || !config.interface.dns_search.is_empty() {
        let mut dns_entries: Vec<String> = config
            .interface
            .dns
            .iter()
            .map(ToString::to_string)
            .collect();
        dns_entries.extend(config.interface.dns_search.iter().cloned());
        let _ = writeln!(out, "DNS = {}", dns_entries.join(", "));
    }
    if config.interface.mtu != 0 {
        let _ = writeln!(out, "MTU = {}", config.interface.mtu);
    }
    for cmd in &config.interface.pre_up {
        let _ = writeln!(out, "PreUp = {cmd}");
    }
    for cmd in &config.interface.post_up {
        let _ = writeln!(out, "PostUp = {cmd}");
    }
    for cmd in &config.interface.pre_down {
        let _ = writeln!(out, "PreDown = {cmd}");
    }
    for cmd in &config.interface.post_down {
        let _ = writeln!(out, "PostDown = {cmd}");
    }

    for peer in &config.peers {
        out.push_str("\n[Peer]\n");
        if !peer.name.is_empty() {
            let _ = writeln!(out, "# Name = {}", peer.name);
        }
        let _ = writeln!(out, "PublicKey = {}", peer.public_key.to_base64());
        if let Some(psk) = &peer.preshared_key {
            let _ = writeln!(out, "PresharedKey = {}", psk.to_base64());
        }
        if let Some(ep) = &peer.endpoint {
            let _ = writeln!(out, "Endpoint = {ep}");
        }
        if !peer.allowed_ips.is_empty() {
            let _ = writeln!(out, "AllowedIPs = {}", peer.allowed_ips.join(", "));
        }
        if peer.persistent_keepalive != 0 {
            let _ = writeln!(out, "PersistentKeepalive = {}", peer.persistent_keepalive);
        }
    }

    out
}

// -- Internal helpers --

/// Split a line into key and value on the first `=`.
fn split_kv(line: &str, line_num: usize) -> Result<(&str, &str), ConfigError> {
    let (key, value) = line.split_once('=').ok_or(ConfigError::WgQuickParse {
        line: line_num,
        reason: format!("expected 'Key = Value', got: {line}"),
    })?;
    Ok((key.trim(), value.trim()))
}

/// Builder for accumulating `[Interface]` fields.
#[derive(Default)]
struct InterfaceBuilder {
    private_key: Option<PrivateKey>,
    listen_port: u16,
    addresses: Vec<String>,
    dns: Vec<IpAddr>,
    dns_search: Vec<String>,
    mtu: u16,
    fwmark: u32,
    pre_up: Vec<String>,
    post_up: Vec<String>,
    pre_down: Vec<String>,
    post_down: Vec<String>,
}

impl InterfaceBuilder {
    fn build(self) -> Result<InterfaceConfig, ConfigError> {
        Ok(InterfaceConfig {
            private_key: self
                .private_key
                .ok_or(ConfigError::MissingField("PrivateKey"))?,
            listen_port: self.listen_port,
            addresses: self.addresses,
            dns: self.dns,
            dns_search: self.dns_search,
            mtu: self.mtu,
            fwmark: self.fwmark,
            pre_up: self.pre_up,
            post_up: self.post_up,
            pre_down: self.pre_down,
            post_down: self.post_down,
        })
    }
}

/// Builder for accumulating `[Peer]` fields.
#[derive(Default)]
struct PeerBuilder {
    name: String,
    public_key: Option<PublicKey>,
    preshared_key: Option<PresharedKey>,
    endpoint: Option<String>,
    allowed_ips: Vec<String>,
    persistent_keepalive: u16,
}

impl PeerBuilder {
    fn build(self) -> Result<PeerConfig, ConfigError> {
        Ok(PeerConfig {
            name: self.name,
            public_key: self
                .public_key
                .ok_or(ConfigError::MissingField("PublicKey"))?,
            preshared_key: self.preshared_key,
            endpoint: self.endpoint,
            allowed_ips: self.allowed_ips,
            persistent_keepalive: self.persistent_keepalive,
        })
    }
}

/// Parse a single key-value pair in the `[Interface]` section.
fn parse_interface_kv(
    iface: &mut InterfaceBuilder,
    key: &str,
    value: &str,
    line_num: usize,
) -> Result<(), ConfigError> {
    match key.to_ascii_lowercase().as_str() {
        "privatekey" => {
            iface.private_key =
                Some(
                    PrivateKey::from_base64(value).map_err(|e| ConfigError::WgQuickParse {
                        line: line_num,
                        reason: format!("PrivateKey: {e}"),
                    })?,
                );
        }
        "listenport" => {
            iface.listen_port = value.parse().map_err(|e| ConfigError::WgQuickParse {
                line: line_num,
                reason: format!("ListenPort: {e}"),
            })?;
        }
        "address" => {
            for addr in value.split(',') {
                iface.addresses.push(addr.trim().to_owned());
            }
        }
        "dns" => {
            for token in value.split(',') {
                let token = token.trim();
                if let Ok(ip) = token.parse::<IpAddr>() {
                    iface.dns.push(ip);
                } else {
                    iface.dns_search.push(token.to_owned());
                }
            }
        }
        "mtu" => {
            iface.mtu = value.parse().map_err(|e| ConfigError::WgQuickParse {
                line: line_num,
                reason: format!("MTU: {e}"),
            })?;
        }
        "fwmark" => {
            iface.fwmark = value.parse().map_err(|e| ConfigError::WgQuickParse {
                line: line_num,
                reason: format!("FwMark: {e}"),
            })?;
        }
        "preup" => iface.pre_up.push(value.to_owned()),
        "postup" => iface.post_up.push(value.to_owned()),
        "predown" => iface.pre_down.push(value.to_owned()),
        "postdown" => iface.post_down.push(value.to_owned()),
        _ => {
            return Err(ConfigError::WgQuickParse {
                line: line_num,
                reason: format!("unknown Interface key: {key}"),
            });
        }
    }
    Ok(())
}

/// Parse a single key-value pair in the `[Peer]` section.
fn parse_peer_kv(
    peer: &mut PeerBuilder,
    key: &str,
    value: &str,
    line_num: usize,
) -> Result<(), ConfigError> {
    match key.to_ascii_lowercase().as_str() {
        "publickey" => {
            peer.public_key =
                Some(
                    PublicKey::from_base64(value).map_err(|e| ConfigError::WgQuickParse {
                        line: line_num,
                        reason: format!("PublicKey: {e}"),
                    })?,
                );
        }
        "presharedkey" => {
            peer.preshared_key =
                Some(
                    PresharedKey::from_base64(value).map_err(|e| ConfigError::WgQuickParse {
                        line: line_num,
                        reason: format!("PresharedKey: {e}"),
                    })?,
                );
        }
        "endpoint" => {
            peer.endpoint = Some(value.to_owned());
        }
        "allowedips" => {
            for ip in value.split(',') {
                peer.allowed_ips.push(ip.trim().to_owned());
            }
        }
        "persistentkeepalive" => {
            peer.persistent_keepalive = value.parse().map_err(|e| ConfigError::WgQuickParse {
                line: line_num,
                reason: format!("PersistentKeepalive: {e}"),
            })?;
        }
        _ => {
            return Err(ConfigError::WgQuickParse {
                line: line_num,
                reason: format!("unknown Peer key: {key}"),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_WG_QUICK: &str = r#"
[Interface]
PrivateKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=
ListenPort = 51820
Address = 10.0.0.2/24
DNS = 1.1.1.1, 8.8.8.8
MTU = 1420

[Peer]
PublicKey = HIgo9xNzJMWLKASShiTqIybxZ0U3wGLiUeJ1PKf8ykw=
Endpoint = 198.51.100.1:51820
AllowedIPs = 10.100.0.0/16
PersistentKeepalive = 25
"#;

    #[test]
    fn parse_standard_wg_quick() {
        let config = load_from_str(SAMPLE_WG_QUICK).expect("parse");
        assert_eq!(config.interface.listen_port, 51820);
        assert_eq!(config.interface.mtu, 1420);
        assert_eq!(config.interface.addresses, vec!["10.0.0.2/24"]);
        assert_eq!(config.interface.dns.len(), 2);
        assert_eq!(config.peers.len(), 1);
        assert_eq!(config.peers[0].persistent_keepalive, 25);
    }

    #[test]
    fn parse_comments_and_blank_lines() {
        let input = r#"
# This is a comment
; This is also a comment

[Interface]
PrivateKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=

[Peer]
# peer comment
PublicKey = HIgo9xNzJMWLKASShiTqIybxZ0U3wGLiUeJ1PKf8ykw=
AllowedIPs = 0.0.0.0/0
"#;
        let config = load_from_str(input).expect("parse");
        assert_eq!(config.peers.len(), 1);
    }

    #[test]
    fn parse_multiple_peers() {
        let pub1 = PrivateKey::generate().public_key();
        let pub2 = PrivateKey::generate().public_key();
        let private = PrivateKey::generate();

        let input = format!(
            r#"
[Interface]
PrivateKey = {}

[Peer]
PublicKey = {}
AllowedIPs = 10.0.0.0/8
Endpoint = 1.2.3.4:51820

[Peer]
PublicKey = {}
AllowedIPs = 172.16.0.0/12
Endpoint = 5.6.7.8:51820
"#,
            private.to_base64(),
            pub1.to_base64(),
            pub2.to_base64(),
        );

        let config = load_from_str(&input).expect("parse");
        assert_eq!(config.peers.len(), 2);
    }

    #[test]
    fn parse_pre_post_commands() {
        let private = PrivateKey::generate();
        let pub1 = PrivateKey::generate().public_key();

        let input = format!(
            r#"
[Interface]
PrivateKey = {}
PreUp = echo pre-up
PostUp = iptables -A FORWARD -i wg0 -j ACCEPT
PreDown = echo pre-down
PostDown = iptables -D FORWARD -i wg0 -j ACCEPT

[Peer]
PublicKey = {}
AllowedIPs = 0.0.0.0/0
"#,
            private.to_base64(),
            pub1.to_base64(),
        );

        let config = load_from_str(&input).expect("parse");
        assert_eq!(config.interface.pre_up, vec!["echo pre-up"]);
        assert_eq!(config.interface.post_down.len(), 1);
    }

    #[test]
    fn missing_private_key_rejected() {
        let pub1 = PrivateKey::generate().public_key();
        let input = format!(
            "[Interface]\nListenPort = 51820\n\n[Peer]\nPublicKey = {}\nAllowedIPs = 0.0.0.0/0\n",
            pub1.to_base64()
        );
        let err = load_from_str(&input).unwrap_err();
        assert!(matches!(err, ConfigError::MissingField("PrivateKey")));
    }

    #[test]
    fn missing_public_key_rejected() {
        let private = PrivateKey::generate();
        let input = format!(
            "[Interface]\nPrivateKey = {}\n\n[Peer]\nAllowedIPs = 0.0.0.0/0\n",
            private.to_base64()
        );
        let err = load_from_str(&input).unwrap_err();
        assert!(matches!(err, ConfigError::MissingField("PublicKey")));
    }

    #[test]
    fn export_roundtrip() {
        let config = load_from_str(SAMPLE_WG_QUICK).expect("parse");
        let exported = export_to_string(&config);
        let reparsed = load_from_str(&exported).expect("reparse");

        assert_eq!(reparsed.interface.listen_port, config.interface.listen_port);
        assert_eq!(reparsed.peers.len(), config.peers.len());
        assert_eq!(
            reparsed.peers[0].persistent_keepalive,
            config.peers[0].persistent_keepalive
        );
    }

    #[test]
    fn dns_search_domains_parsed() {
        let private = PrivateKey::generate();
        let pub1 = PrivateKey::generate().public_key();
        let input = format!(
            "[Interface]\nPrivateKey = {}\nDNS = 1.1.1.1, 8.8.8.8, corp.internal\n\n[Peer]\nPublicKey = {}\nAllowedIPs = 0.0.0.0/0\n",
            private.to_base64(),
            pub1.to_base64(),
        );
        let config = load_from_str(&input).expect("parse");
        assert_eq!(config.interface.dns.len(), 2);
        assert_eq!(config.interface.dns_search, vec!["corp.internal"]);
    }

    #[test]
    fn dns_all_ips_no_search() {
        let config = load_from_str(SAMPLE_WG_QUICK).expect("parse");
        assert_eq!(config.interface.dns.len(), 2);
        assert!(config.interface.dns_search.is_empty());
    }

    #[test]
    fn dns_export_with_search_domains() {
        let private = PrivateKey::generate();
        let pub1 = PrivateKey::generate().public_key();
        let input = format!(
            "[Interface]\nPrivateKey = {}\nDNS = 1.1.1.1, corp.internal\n\n[Peer]\nPublicKey = {}\nAllowedIPs = 0.0.0.0/0\n",
            private.to_base64(),
            pub1.to_base64(),
        );
        let config = load_from_str(&input).expect("parse");
        let exported = export_to_string(&config);
        assert!(exported.contains("DNS = 1.1.1.1, corp.internal"));
        // Round-trip: re-parse and verify search domain survives.
        let reparsed = load_from_str(&exported).expect("reparse");
        assert_eq!(reparsed.interface.dns_search, vec!["corp.internal"]);
    }

    #[test]
    fn key_value_outside_section_rejected() {
        let err = load_from_str("PrivateKey = abc123\n").unwrap_err();
        assert!(matches!(err, ConfigError::WgQuickParse { line: 1, .. }));
    }

    #[test]
    fn unknown_interface_key_rejected() {
        let input = "[Interface]\nPrivateKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=\nBogusKey = value\n";
        let err = load_from_str(input).unwrap_err();
        assert!(matches!(err, ConfigError::WgQuickParse { .. }));
    }

    #[test]
    fn preshared_key_parsed() {
        let private = PrivateKey::generate();
        let pub1 = PrivateKey::generate().public_key();
        let psk = PresharedKey::from_bytes([42u8; 32]);

        let input = format!(
            "[Interface]\nPrivateKey = {}\n\n[Peer]\nPublicKey = {}\nPresharedKey = {}\nAllowedIPs = 0.0.0.0/0\n",
            private.to_base64(),
            pub1.to_base64(),
            psk.to_base64(),
        );

        let config = load_from_str(&input).expect("parse");
        assert!(config.peers[0].preshared_key.is_some());
    }

    #[test]
    fn save_and_load_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("wg0.conf");

        let config = load_from_str(SAMPLE_WG_QUICK).expect("parse");
        let exported = export_to_string(&config);
        std::fs::write(&path, &exported).expect("write");

        let loaded = load_from_file(&path).expect("load");
        assert_eq!(loaded.interface.listen_port, 51820);
    }
}
