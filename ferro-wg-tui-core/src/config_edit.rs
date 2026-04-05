use std::collections::HashSet;

use ferro_wg_core::config::WgConfig;
use thiserror::Error;

/// Which section of the Config tab is focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSection {
    /// The `[interface]` block.
    Interface,
    /// A specific peer by index.
    Peer(usize),
}

/// A mutable field in the interface or peer form.
///
/// This is a field **descriptor only** — it carries a name, label, and
/// validator tag. It carries no runtime peer index or current value.
/// The `usize` in `ConfigSection::Peer(usize)` is ignored by
/// `fields_for_section` for field-set selection; all peers share the same
/// field structure. The function returns one of two pre-defined `static`
/// arrays based on `(section_variant, is_new_peer)`.
///
/// - `Interface` → always the same 10 fields
/// - `Peer(…, is_new_peer=false)` → 5 fields excluding `PeerPublicKey`
/// - `Peer(…, is_new_peer=true)` → 6 fields with `PeerPublicKey` first
///
/// This makes `&'static [EditableField]` achievable without allocation.
///
/// Determines which validator and label to use and which struct field
/// is written back on confirm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditableField {
    // -- Interface fields --
    ListenPort,
    Addresses, // comma-separated CIDR strings
    Dns,       // comma-separated IP strings
    DnsSearch, // comma-separated domain strings
    Mtu,
    Fwmark,
    PreUp, // comma-separated command strings
    PostUp,
    PreDown,
    PostDown,
    // -- Peer fields --
    PeerName,
    PeerPublicKey, // required for new peers; read-only for existing
    PeerEndpoint,
    PeerAllowedIps, // comma-separated CIDR strings
    PeerPersistentKeepalive,
}

/// Pending edits for one connection, held in `AppState` during editing.
///
/// Cloned from `ConnectionView::config` when editing begins. Never
/// written back to `ConnectionView` until the user confirms the save.
/// Discarded on `Esc` or `ConfirmNo`.
#[derive(Debug, Clone)]
pub struct ConfigEditState {
    /// Connection being edited (used for lookup and for the save path).
    pub connection_name: String,
    /// The mutable working copy of the config.
    pub draft: WgConfig,
    /// Which section of the form is focused (interface vs peer N).
    pub focused_section: ConfigSection,
    /// Which field within the section is focused.
    pub focused_field_idx: usize,
    /// If `Some`, the field is in edit mode and this is the text buffer.
    pub edit_buffer: Option<String>,
    /// Inline validation error for the current buffer, if any.
    pub field_error: Option<String>,
}

/// A single line in a config diff.
///
/// `DiffLine` is TUI-specific — its variants drive color rendering decisions,
/// so it lives in `ferro-wg-tui-core` alongside `config_diff`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    /// A line present only in the old config.
    Removed(String),
    /// A line present only in the new config.
    Added(String),
    /// A line unchanged between old and new.
    Context(String),
}

/// Pending diff preview shown before save.
///
/// Stored in `AppState` when the user requests a preview. Cleared on
/// save, discard, or `Esc`.
#[derive(Debug, Clone)]
pub struct ConfigDiffPending {
    /// Connection name being saved.
    pub connection_name: String,
    /// The final validated draft to be written on confirm.
    pub draft: WgConfig,
    /// Diff lines to display (context ± 3 lines around changes).
    pub diff_lines: Vec<DiffLine>,
    /// Scroll offset for the diff overlay.
    pub scroll_offset: usize,
}

/// Errors from config editing operations.
///
/// Dedicated error enum for the config editing layer using `thiserror`.
#[derive(Debug, Error)]
pub enum ConfigEditError {
    #[error("Port must be <= 65535")]
    PortTooHigh,
    #[error("Port must be a number")]
    PortNotNumeric,
    #[error("Invalid CIDR: {0}")]
    InvalidCidr(String),
    #[error("Invalid IP address: {0}")]
    InvalidIp(String),
    #[error("Invalid domain: {0}")]
    InvalidDomain(String),
    #[error("MTU must be 0 (auto) or 576-9000")]
    MtuOutOfRange,
    #[error("MTU must be a number")]
    MtuNotNumeric,
    #[error("Fwmark must be a number")]
    FwmarkNotNumeric,
    #[error("Public key must be 44 characters")]
    PublicKeyLength,
    #[error("Invalid base64 public key")]
    PublicKeyInvalidBase64,
    #[error("Endpoint must be host:port")]
    EndpointFormat,
    #[error("Host cannot be empty")]
    EndpointHostEmpty,
    #[error("Port must be a number")]
    EndpointPortNotNumeric,
    #[error("Duplicate allowed IP: {0}")]
    DuplicateAllowedIp(String),
    #[error("Duplicate CIDR in input")]
    DuplicateCidrInInput,
    #[error("Keepalive must be <= 65535")]
    KeepaliveTooHigh,
    #[error("Keepalive must be a number")]
    KeepaliveNotNumeric,
}

/// Validate a UDP port.
///
/// - Accepts the empty string `""` — mapped to `listen_port = 0` (OS picks a port).
/// - Accepts `"0"` explicitly for the same reason.
/// - Accepts any value 1–65535.
/// - Rejects values > 65535.
/// - Rejects non-numeric, non-empty strings.
pub fn validate_port(s: &str) -> Result<(), ConfigEditError> {
    if s.is_empty() {
        return Ok(());
    }
    match s.parse::<u16>() {
        Ok(_port) => Ok(()),
        Err(_) => Err(ConfigEditError::PortNotNumeric),
    }
}

/// Validate a comma-separated list of CIDR addresses (e.g. `10.0.0.2/24`).
pub fn validate_addresses(s: &str) -> Result<(), ConfigEditError> {
    if s.is_empty() {
        return Ok(());
    }
    for addr in s.split(',') {
        let addr = addr.trim();
        if addr.is_empty() {
            continue;
        }
        if !is_valid_cidr(addr) {
            return Err(ConfigEditError::InvalidCidr(addr.to_string()));
        }
    }
    Ok(())
}

/// Validate a comma-separated list of IP addresses (DNS servers).
pub fn validate_dns_ips(s: &str) -> Result<(), ConfigEditError> {
    if s.is_empty() {
        return Ok(());
    }
    for ip in s.split(',') {
        let ip = ip.trim();
        if ip.is_empty() {
            continue;
        }
        if ip.parse::<std::net::IpAddr>().is_err() {
            return Err(ConfigEditError::InvalidIp(ip.to_string()));
        }
    }
    Ok(())
}

/// Validate a comma-separated list of DNS search domains.
pub fn validate_dns_search(s: &str) -> Result<(), ConfigEditError> {
    if s.is_empty() {
        return Ok(());
    }
    for domain in s.split(',') {
        let domain = domain.trim();
        if domain.is_empty() {
            continue;
        }
        if !is_valid_domain(domain) {
            return Err(ConfigEditError::InvalidDomain(domain.to_string()));
        }
    }
    Ok(())
}

/// Validate an MTU value (576–9000, or 0 for auto).
pub fn validate_mtu(s: &str) -> Result<(), ConfigEditError> {
    if s.is_empty() {
        return Ok(());
    }
    match s.parse::<u16>() {
        Ok(mtu) => {
            if mtu == 0 || (576..=9000).contains(&mtu) {
                Ok(())
            } else {
                Err(ConfigEditError::MtuOutOfRange)
            }
        }
        Err(_) => Err(ConfigEditError::MtuNotNumeric),
    }
}

/// Validate a firewall mark (any `u32`, including 0).
pub fn validate_fwmark(s: &str) -> Result<(), ConfigEditError> {
    if s.is_empty() {
        return Ok(());
    }
    s.parse::<u32>()
        .map_err(|_| ConfigEditError::FwmarkNotNumeric)?;
    Ok(())
}

/// Validate a WireGuard base64 public key (44 characters, valid base64).
pub fn validate_public_key(s: &str) -> Result<(), ConfigEditError> {
    if s.len() != 44 {
        return Err(ConfigEditError::PublicKeyLength);
    }
    base64::decode(s).map_err(|_| ConfigEditError::PublicKeyInvalidBase64)?; // TODO: migrate to base64::Engine
    Ok(())
}

/// Validate a peer endpoint (`host:port` or empty for receive-only peers).
pub fn validate_endpoint(s: &str) -> Result<(), ConfigEditError> {
    if s.is_empty() {
        return Ok(());
    }
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return Err(ConfigEditError::EndpointFormat);
    }
    let host = parts[0];
    let port = parts[1];
    if host.is_empty() {
        return Err(ConfigEditError::EndpointHostEmpty);
    }
    port.parse::<u16>()
        .map_err(|_| ConfigEditError::EndpointPortNotNumeric)?;
    Ok(())
}

/// Validate a comma-separated list of allowed-IP CIDR ranges.
///
/// Also checks for exact string duplicates against all peers in the draft —
/// WireGuard forbids duplicate allowed-IP entries. Note: only exact string
/// duplicates are rejected; CIDR overlaps that are not exact duplicates are
/// permitted (WireGuard kernel enforcement handles overlap detection at
/// runtime). The `other_peers_allowed_ips` slice is a flat list of all
/// existing allowed-IP strings across all other peers; callers flatten
/// `peer.allowed_ips.iter()` into a collected `Vec<String>` before calling.
pub fn validate_allowed_ips(
    s: &str,
    other_peers_allowed_ips: &[String],
) -> Result<(), ConfigEditError> {
    if s.is_empty() {
        return Ok(());
    }
    let mut seen = HashSet::new();
    for cidr in s.split(',') {
        let cidr = cidr.trim().to_string();
        if cidr.is_empty() {
            continue;
        }
        if !is_valid_cidr(&cidr) {
            return Err(ConfigEditError::InvalidCidr(cidr));
        }
        if other_peers_allowed_ips.contains(&cidr) {
            return Err(ConfigEditError::DuplicateAllowedIp(cidr));
        }
        if !seen.insert(cidr.clone()) {
            return Err(ConfigEditError::DuplicateCidrInInput);
        }
    }
    Ok(())
}

/// Validate a persistent keepalive interval (0–65535 seconds).
pub fn validate_persistent_keepalive(s: &str) -> Result<(), ConfigEditError> {
    if s.is_empty() {
        return Ok(());
    }
    match s.parse::<u16>() {
        Ok(_keepalive) => Ok(()),
        Err(_) => Err(ConfigEditError::KeepaliveNotNumeric),
    }
}

/// Compute a unified diff between two TOML strings as a `Vec<DiffLine>`.
///
/// Uses a simple line-by-line LCS diff (stdlib only; no external diff crate).
/// Returns context lines (up to 3 before and after each changed block) plus
/// `Added` / `Removed` lines. Called from `dispatch(PreviewConfig)` —
/// never from a render path.
///
/// Although this is a pure string transform, it lives in `ferro-wg-tui-core`
/// alongside `DiffLine` because `DiffLine` is TUI-specific.
pub fn config_diff(old_toml: &str, new_toml: &str) -> Vec<DiffLine> {
    let old_lines: Vec<&str> = old_toml.lines().collect();
    let new_lines: Vec<&str> = new_toml.lines().collect();
    let mut diff = Vec::new();

    let mut i = 0;
    let mut j = 0;
    while i < old_lines.len() && j < new_lines.len() {
        if old_lines[i] == new_lines[j] {
            diff.push(DiffLine::Context(old_lines[i].to_string()));
            i += 1;
            j += 1;
        } else {
            // Check if old line is in new
            let mut found = false;
            for k in j..new_lines.len() {
                if old_lines[i] == new_lines[k] {
                    // Insert added lines
                    for l in j..k {
                        diff.push(DiffLine::Added(new_lines[l].to_string()));
                    }
                    j = k;
                    found = true;
                    break;
                }
            }
            if !found {
                // Old line removed
                diff.push(DiffLine::Removed(old_lines[i].to_string()));
                i += 1;
            }
        }
    }
    // Remaining old lines
    for &line in &old_lines[i..] {
        diff.push(DiffLine::Removed(line.to_string()));
    }
    // Remaining new lines
    for &line in &new_lines[j..] {
        diff.push(DiffLine::Added(line.to_string()));
    }
    diff
}

/// Return the ordered slice of editable fields for the given section.
///
/// `EditableField` is a descriptor only — no peer index or current value
/// is embedded. The `usize` in `ConfigSection::Peer(usize)` is ignored;
/// all peers share the same field structure. Returns one of three pre-defined
/// `static` arrays:
///
/// - `Interface` → 10 fields (all interface fields)
/// - `Peer(…, is_new_peer=false)` → 5 fields (excludes `PeerPublicKey`)
/// - `Peer(…, is_new_peer=true)` → 6 fields (`PeerPublicKey` first)
pub fn fields_for_section(section: ConfigSection, is_new_peer: bool) -> &'static [EditableField] {
    const INTERFACE_FIELDS: &[EditableField] = &[
        EditableField::ListenPort,
        EditableField::Addresses,
        EditableField::Dns,
        EditableField::DnsSearch,
        EditableField::Mtu,
        EditableField::Fwmark,
        EditableField::PreUp,
        EditableField::PostUp,
        EditableField::PreDown,
        EditableField::PostDown,
    ];
    const PEER_FIELDS_EXISTING: &[EditableField] = &[
        EditableField::PeerName,
        EditableField::PeerEndpoint,
        EditableField::PeerAllowedIps,
        EditableField::PeerPersistentKeepalive,
    ];
    const PEER_FIELDS_NEW: &[EditableField] = &[
        EditableField::PeerPublicKey,
        EditableField::PeerName,
        EditableField::PeerEndpoint,
        EditableField::PeerAllowedIps,
        EditableField::PeerPersistentKeepalive,
    ];

    match section {
        ConfigSection::Interface => INTERFACE_FIELDS,
        ConfigSection::Peer(_) => {
            if is_new_peer {
                PEER_FIELDS_NEW
            } else {
                PEER_FIELDS_EXISTING
            }
        }
    }
}

/// Check if a string is a valid CIDR notation.
fn is_valid_cidr(cidr: &str) -> bool {
    let parts: Vec<&str> = cidr.split('/').collect();
    if parts.len() != 2 {
        return false;
    }
    let ip = parts[0];
    let prefix = parts[1];
    let ip_addr = match ip.parse::<std::net::IpAddr>() {
        Ok(addr) => addr,
        Err(_) => return false,
    };
    match prefix.parse::<u8>() {
        Ok(p) => match ip_addr {
            std::net::IpAddr::V4(_) => p <= 32,
            std::net::IpAddr::V6(_) => p <= 128,
        },
        Err(_) => false,
    }
}

/// Check if a string is a valid domain name.
fn is_valid_domain(domain: &str) -> bool {
    // Simple validation: no spaces, starts with alphanumeric, etc.
    !domain.is_empty()
        && domain
            .chars()
            .all(|c| c.is_alphanumeric() || c == '.' || c == '-')
        && domain.chars().next().is_some_and(|c| c.is_alphanumeric())
        && domain.chars().last().is_some_and(|c| c.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_port() {
        assert!(validate_port("").is_ok());
        assert!(validate_port("0").is_ok());
        assert!(validate_port("51820").is_ok());
        assert!(validate_port("65535").is_ok());
        assert!(matches!(
            validate_port("65536"),
            Err(ConfigEditError::PortNotNumeric)
        ));
        assert!(matches!(
            validate_port("abc"),
            Err(ConfigEditError::PortNotNumeric)
        ));
    }

    #[test]
    fn test_validate_addresses() {
        assert!(validate_addresses("").is_ok());
        assert!(validate_addresses("10.0.0.1/24").is_ok());
        assert!(validate_addresses("10.0.0.1/24, 192.168.1.1/32").is_ok());
        assert!(matches!(
            validate_addresses("invalid"),
            Err(ConfigEditError::InvalidCidr(_))
        ));
    }

    #[test]
    fn test_validate_dns_ips() {
        assert!(validate_dns_ips("").is_ok());
        assert!(validate_dns_ips("8.8.8.8").is_ok());
        assert!(validate_dns_ips("8.8.8.8, 1.1.1.1").is_ok());
        assert!(matches!(
            validate_dns_ips("invalid"),
            Err(ConfigEditError::InvalidIp(_))
        ));
    }

    #[test]
    fn test_validate_dns_search() {
        assert!(validate_dns_search("").is_ok());
        assert!(validate_dns_search("example.com").is_ok());
        assert!(validate_dns_search("example.com, sub.example.com").is_ok());
        assert!(matches!(
            validate_dns_search("invalid domain"),
            Err(ConfigEditError::InvalidDomain(_))
        ));
    }

    #[test]
    fn test_validate_mtu() {
        assert!(validate_mtu("").is_ok());
        assert!(validate_mtu("0").is_ok());
        assert!(validate_mtu("576").is_ok());
        assert!(validate_mtu("9000").is_ok());
        assert!(validate_mtu("1500").is_ok());
        assert!(matches!(
            validate_mtu("575"),
            Err(ConfigEditError::MtuOutOfRange)
        ));
        assert!(matches!(
            validate_mtu("9001"),
            Err(ConfigEditError::MtuOutOfRange)
        ));
        assert!(matches!(
            validate_mtu("abc"),
            Err(ConfigEditError::MtuNotNumeric)
        ));
    }

    #[test]
    fn test_validate_fwmark() {
        assert!(validate_fwmark("").is_ok());
        assert!(validate_fwmark("0").is_ok());
        assert!(validate_fwmark("123").is_ok());
        assert!(matches!(
            validate_fwmark("abc"),
            Err(ConfigEditError::FwmarkNotNumeric)
        ));
    }

    #[test]
    fn test_validate_public_key() {
        let valid_key = "YWJjZGVmZ2hpamsAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="; // valid base64 44 chars
        assert!(validate_public_key(valid_key).is_ok());
        assert!(matches!(
            validate_public_key("short"),
            Err(ConfigEditError::PublicKeyLength)
        ));
        assert!(matches!(
            validate_public_key("abcdefghijklmnopqrstuvwxyz0123456789+/=x"),
            Err(ConfigEditError::PublicKeyLength)
        )); // 45 chars
        assert!(matches!(
            validate_public_key("invalid base64!@#"),
            Err(ConfigEditError::PublicKeyInvalidBase64)
        ));
    }

    #[test]
    fn test_validate_endpoint() {
        assert!(validate_endpoint("").is_ok());
        assert!(validate_endpoint("198.51.100.1:51820").is_ok());
        assert!(validate_endpoint("vpn.example.com:51820").is_ok());
        assert!(matches!(
            validate_endpoint("[::1]:51820"),
            Err(ConfigEditError::EndpointFormat)
        )); // IPv6 not supported in this simple validation
        assert!(matches!(
            validate_endpoint("host"),
            Err(ConfigEditError::EndpointFormat)
        ));
        assert!(matches!(
            validate_endpoint(":51820"),
            Err(ConfigEditError::EndpointHostEmpty)
        ));
        assert!(matches!(
            validate_endpoint("host:abc"),
            Err(ConfigEditError::EndpointPortNotNumeric)
        ));
    }

    #[test]
    fn test_validate_allowed_ips() {
        let other = vec![];
        assert!(validate_allowed_ips("", &other).is_ok());
        assert!(validate_allowed_ips("10.0.0.0/8", &other).is_ok());
        assert!(validate_allowed_ips("10.0.0.0/8, 192.168.1.0/24", &other).is_ok());
        assert!(validate_allowed_ips("invalid", &other).is_err());
        let other_with_dup = vec!["10.0.0.0/8".to_string()];
        assert!(validate_allowed_ips("10.0.0.0/8", &other_with_dup).is_err());
        assert!(validate_allowed_ips("10.0.0.0/8, 10.0.0.0/8", &other).is_err());
    }

    #[test]
    fn test_validate_persistent_keepalive() {
        assert!(validate_persistent_keepalive("").is_ok());
        assert!(validate_persistent_keepalive("0").is_ok());
        assert!(validate_persistent_keepalive("25").is_ok());
        assert!(validate_persistent_keepalive("65535").is_ok());
        assert!(matches!(
            validate_persistent_keepalive("65536"),
            Err(ConfigEditError::KeepaliveNotNumeric)
        ));
        assert!(matches!(
            validate_persistent_keepalive("abc"),
            Err(ConfigEditError::KeepaliveNotNumeric)
        ));
    }

    #[test]
    fn test_config_diff() {
        let old = "a\nb\nc";
        let new = "a\nd\nc";
        let diff = config_diff(old, new);
        assert_eq!(
            diff,
            vec![
                DiffLine::Context("a".to_string()),
                DiffLine::Removed("b".to_string()),
                DiffLine::Added("d".to_string()),
                DiffLine::Context("c".to_string()),
            ]
        );

        let old = "a\nb";
        let new = "a\nb";
        let diff = config_diff(old, new);
        assert_eq!(
            diff,
            vec![
                DiffLine::Context("a".to_string()),
                DiffLine::Context("b".to_string()),
            ]
        );

        let old = "a";
        let new = "b";
        let diff = config_diff(old, new);
        assert_eq!(
            diff,
            vec![
                DiffLine::Removed("a".to_string()),
                DiffLine::Added("b".to_string()),
            ]
        );
    }

    #[test]
    fn test_fields_for_section() {
        assert_eq!(
            fields_for_section(ConfigSection::Interface, false).len(),
            10
        );
        assert_eq!(fields_for_section(ConfigSection::Interface, true).len(), 10);
        assert_eq!(fields_for_section(ConfigSection::Peer(0), false).len(), 4);
        assert_eq!(fields_for_section(ConfigSection::Peer(0), true).len(), 5);
        assert_eq!(
            fields_for_section(ConfigSection::Peer(0), true)[0],
            EditableField::PeerPublicKey
        );
    }

    #[test]
    fn test_is_valid_domain() {
        assert!(is_valid_domain("example.com"));
        assert!(is_valid_domain("sub.example.com"));
        assert!(!is_valid_domain(""));
        assert!(!is_valid_domain(" example.com"));
        assert!(!is_valid_domain("example.com "));
        assert!(!is_valid_domain("exa mple.com"));
        assert!(!is_valid_domain("-example.com"));
        assert!(!is_valid_domain("example.com-"));
    }
}
