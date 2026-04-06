use std::collections::HashSet;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
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
/// - `Peer(…, is_new_peer=false)` → 4 fields excluding `PeerPublicKey`
/// - `Peer(…, is_new_peer=true)` → 5 fields with `PeerPublicKey` first
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
    /// Indices into `draft.peers` that were created during this editing session
    /// and whose public key has not yet been confirmed by the user.
    ///
    /// A peer is inserted here by `AddConfigPeer` and removed once
    /// `PeerPublicKey` is committed via `ConfigEditKey(Enter)`. Peers present
    /// in this set block `PreviewConfig` from proceeding.
    pub new_peer_indices: HashSet<usize>,
    /// A session-level error that spans the entire config edit session,
    /// not tied to a specific field. Shown in the status line of the config
    /// editor. Set by `PreviewConfig` when new peers have unconfirmed keys.
    pub session_error: Option<String>,
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
/// - Rejects values > 65535 with [`ConfigEditError::PortTooHigh`].
/// - Rejects non-numeric, non-empty strings with [`ConfigEditError::PortNotNumeric`].
#[allow(clippy::missing_errors_doc)]
pub fn validate_port(s: &str) -> Result<(), ConfigEditError> {
    if s.is_empty() {
        return Ok(());
    }
    let n: u32 = s.parse().map_err(|_| ConfigEditError::PortNotNumeric)?;
    if n > 65535 {
        return Err(ConfigEditError::PortTooHigh);
    }
    Ok(())
}

/// Validate a comma-separated list of CIDR addresses (e.g. `10.0.0.2/24`).
#[allow(clippy::missing_errors_doc)]
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
#[allow(clippy::missing_errors_doc)]
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
#[allow(clippy::missing_errors_doc)]
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
#[allow(clippy::missing_errors_doc)]
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
#[allow(clippy::missing_errors_doc)]
pub fn validate_fwmark(s: &str) -> Result<(), ConfigEditError> {
    if s.is_empty() {
        return Ok(());
    }
    s.parse::<u32>()
        .map_err(|_| ConfigEditError::FwmarkNotNumeric)?;
    Ok(())
}

/// Validate a `WireGuard` base64 public key (44 characters, valid base64).
///
/// # Errors
///
/// Returns `ConfigEditError::PublicKeyLength` if the string is not 44 characters long.
/// Returns `ConfigEditError::PublicKeyInvalidBase64` if the string is not valid base64 or does not decode to 32 bytes.
pub fn validate_public_key(s: &str) -> Result<(), ConfigEditError> {
    if s.len() != 44 {
        return Err(ConfigEditError::PublicKeyLength);
    }
    let decoded = BASE64
        .decode(s)
        .map_err(|_| ConfigEditError::PublicKeyInvalidBase64)?;
    if decoded.len() != 32 {
        return Err(ConfigEditError::PublicKeyInvalidBase64);
    }
    Ok(())
}

/// Validate a peer endpoint (`host:port` or empty for receive-only peers).
#[allow(clippy::missing_errors_doc)]
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
/// `WireGuard` forbids duplicate allowed-IP entries. Note: only exact string
/// duplicates are rejected; CIDR overlaps that are not exact duplicates are
/// permitted (`WireGuard` kernel enforcement handles overlap detection at
/// runtime). The `other_peers_allowed_ips` slice is a flat list of all
/// existing allowed-IP strings across all other peers; callers flatten
/// `peer.allowed_ips.iter()` into a collected `Vec<String>` before calling.
#[allow(clippy::missing_errors_doc)]
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
#[allow(clippy::missing_errors_doc)]
pub fn validate_persistent_keepalive(s: &str) -> Result<(), ConfigEditError> {
    if s.is_empty() {
        return Ok(());
    }
    let n: u32 = s
        .parse()
        .map_err(|_| ConfigEditError::KeepaliveNotNumeric)?;
    if n > 65535 {
        return Err(ConfigEditError::KeepaliveTooHigh);
    }
    Ok(())
}

/// Validate a buffer value for the given field within the current draft.
///
/// Dispatches to the appropriate type-specific validator. Called from
/// `dispatch(ConfigEditKey(Enter))` before committing the buffer.
///
/// `section` is needed for `PeerAllowedIps` to exclude the current peer's
/// existing allowed-IPs from the duplicate check.
///
/// # Errors
///
/// Returns the first validation error for the buffer value.
pub fn validate_field(
    field: EditableField,
    value: &str,
    draft: &WgConfig,
    section: ConfigSection,
) -> Result<(), ConfigEditError> {
    match field {
        EditableField::ListenPort => validate_port(value),
        EditableField::Addresses => validate_addresses(value),
        EditableField::Dns => validate_dns_ips(value),
        EditableField::DnsSearch => validate_dns_search(value),
        EditableField::Mtu => validate_mtu(value),
        EditableField::Fwmark => validate_fwmark(value),
        EditableField::PreUp
        | EditableField::PostUp
        | EditableField::PreDown
        | EditableField::PostDown
        | EditableField::PeerName => Ok(()),
        EditableField::PeerPublicKey => validate_public_key(value),
        EditableField::PeerEndpoint => validate_endpoint(value),
        EditableField::PeerAllowedIps => {
            let current_peer_idx = if let ConfigSection::Peer(i) = section {
                Some(i)
            } else {
                None
            };
            let other_ips: Vec<String> = draft
                .peers
                .iter()
                .enumerate()
                .filter(|(i, _)| Some(*i) != current_peer_idx)
                .flat_map(|(_, p)| p.allowed_ips.iter().cloned())
                .collect();
            validate_allowed_ips(value, &other_ips)
        }
        EditableField::PeerPersistentKeepalive => validate_persistent_keepalive(value),
    }
}

/// Return the current value of a config field as an editable string.
///
/// Called from `dispatch(EnterConfigEdit)` to pre-populate the edit buffer
/// with the live value so the user can see and modify it.
///
/// Returns an empty string for fields whose current value is zero/empty
/// (e.g., `listen_port = 0`, `mtu = 0`).
#[must_use]
pub fn field_current_value(
    field: EditableField,
    section: ConfigSection,
    config: &WgConfig,
) -> String {
    let iface = &config.interface;
    match field {
        EditableField::ListenPort => {
            if iface.listen_port == 0 {
                String::new()
            } else {
                iface.listen_port.to_string()
            }
        }
        EditableField::Addresses => iface.addresses.join(", "),
        EditableField::Dns => iface
            .dns
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        EditableField::DnsSearch => iface.dns_search.join(", "),
        EditableField::Mtu => {
            if iface.mtu == 0 {
                String::new()
            } else {
                iface.mtu.to_string()
            }
        }
        EditableField::Fwmark => {
            if iface.fwmark == 0 {
                String::new()
            } else {
                iface.fwmark.to_string()
            }
        }
        EditableField::PreUp => iface.pre_up.join(", "),
        EditableField::PostUp => iface.post_up.join(", "),
        EditableField::PreDown => iface.pre_down.join(", "),
        EditableField::PostDown => iface.post_down.join(", "),
        EditableField::PeerName => {
            if let ConfigSection::Peer(idx) = section {
                config
                    .peers
                    .get(idx)
                    .map(|p| p.name.clone())
                    .unwrap_or_default()
            } else {
                String::new()
            }
        }
        EditableField::PeerPublicKey => {
            if let ConfigSection::Peer(idx) = section {
                config
                    .peers
                    .get(idx)
                    .map(|p| p.public_key.to_base64())
                    .unwrap_or_default()
            } else {
                String::new()
            }
        }
        EditableField::PeerEndpoint => {
            if let ConfigSection::Peer(idx) = section {
                config
                    .peers
                    .get(idx)
                    .and_then(|p| p.endpoint.clone())
                    .unwrap_or_default()
            } else {
                String::new()
            }
        }
        EditableField::PeerAllowedIps => {
            if let ConfigSection::Peer(idx) = section {
                config
                    .peers
                    .get(idx)
                    .map(|p| p.allowed_ips.join(", "))
                    .unwrap_or_default()
            } else {
                String::new()
            }
        }
        EditableField::PeerPersistentKeepalive => {
            if let ConfigSection::Peer(idx) = section {
                config
                    .peers
                    .get(idx)
                    .filter(|p| p.persistent_keepalive != 0)
                    .map(|p| p.persistent_keepalive.to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            }
        }
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
#[must_use]
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
            // Simple diff: assume removed then added
            diff.push(DiffLine::Removed(old_lines[i].to_string()));
            diff.push(DiffLine::Added(new_lines[j].to_string()));
            i += 1;
            j += 1;
        }
    }
    while i < old_lines.len() {
        diff.push(DiffLine::Removed(old_lines[i].to_string()));
        i += 1;
    }
    while j < new_lines.len() {
        diff.push(DiffLine::Added(new_lines[j].to_string()));
        j += 1;
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
/// - `Peer(…, is_new_peer=false)` → 4 fields (excludes `PeerPublicKey`)
/// - `Peer(…, is_new_peer=true)` → 5 fields (`PeerPublicKey` first)
#[must_use]
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

/// Apply a validated field value to the appropriate field in `draft`.
///
/// Called from `dispatch(ConfigEditKey(Enter))` after [`validate_field`] succeeds.
/// The `value` string is guaranteed to be syntactically valid for `field`; all
/// `parse().unwrap_or(0)` calls below are therefore safe — they can only be
/// reached when [`validate_field`] has already confirmed the string is parseable.
///
/// `section` is required to locate the target peer when `field` is a peer field.
///
/// # Panics
///
/// Never panics in practice: all `.unwrap_or(0)` fallbacks are unreachable
/// because `validate_field` verifies parseability before this function is called.
pub fn apply_field(
    field: EditableField,
    value: &str,
    draft: &mut WgConfig,
    section: ConfigSection,
) {
    match field {
        EditableField::ListenPort => {
            draft.interface.listen_port = value.parse().unwrap_or(0);
        }
        EditableField::Addresses => {
            draft.interface.addresses = parse_comma_list(value);
        }
        EditableField::Dns => {
            // `dns` is `Vec<IpAddr>`, not `Vec<String>` — parse each token as an IP address.
            // Tokens that fail to parse are dropped; `validate_field` guarantees all tokens
            // are valid before `apply_field` is called.
            draft.interface.dns = value
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse().ok())
                .collect();
        }
        EditableField::DnsSearch => {
            draft.interface.dns_search = parse_comma_list(value);
        }
        EditableField::Mtu => {
            draft.interface.mtu = value.parse().unwrap_or(0);
        }
        EditableField::Fwmark => {
            draft.interface.fwmark = value.parse().unwrap_or(0);
        }
        EditableField::PreUp => {
            draft.interface.pre_up = parse_comma_list(value);
        }
        EditableField::PostUp => {
            draft.interface.post_up = parse_comma_list(value);
        }
        EditableField::PreDown => {
            draft.interface.pre_down = parse_comma_list(value);
        }
        EditableField::PostDown => {
            draft.interface.post_down = parse_comma_list(value);
        }
        EditableField::PeerName => {
            if let ConfigSection::Peer(idx) = section
                && let Some(peer) = draft.peers.get_mut(idx)
            {
                peer.name = value.to_string();
            }
        }
        EditableField::PeerPublicKey => {
            // Handled separately in dispatch — write-back + new_peer_indices lifecycle.
            // apply_field must not be called for this variant.
        }
        EditableField::PeerEndpoint => {
            if let ConfigSection::Peer(idx) = section
                && let Some(peer) = draft.peers.get_mut(idx)
            {
                peer.endpoint = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
        }
        EditableField::PeerAllowedIps => {
            if let ConfigSection::Peer(idx) = section
                && let Some(peer) = draft.peers.get_mut(idx)
            {
                peer.allowed_ips = parse_comma_list(value);
            }
        }
        EditableField::PeerPersistentKeepalive => {
            if let ConfigSection::Peer(idx) = section
                && let Some(peer) = draft.peers.get_mut(idx)
            {
                peer.persistent_keepalive = value.parse().unwrap_or(0);
            }
        }
    }
}

/// Split a comma-separated string into a trimmed, non-empty `Vec<String>`.
fn parse_comma_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Check if a string is a valid CIDR notation.
fn is_valid_cidr(cidr: &str) -> bool {
    let parts: Vec<&str> = cidr.split('/').collect();
    if parts.len() != 2 {
        return false;
    }
    let ip = parts[0];
    let prefix = parts[1];
    let Ok(ip_addr) = ip.parse::<std::net::IpAddr>() else {
        return false;
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
        && domain.chars().next().is_some_and(char::is_alphanumeric)
        && domain.chars().last().is_some_and(char::is_alphanumeric)
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
            Err(ConfigEditError::PortTooHigh)
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
        let valid_key = "/yt5f1nclaUwO75kn6KosqO2ZD6kJ4Ld4SrYuG1csZg="; // valid base64 32-byte key
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
            validate_public_key("invalid base64!@#invalid base64!@#invalid!!!"),
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
            Err(ConfigEditError::KeepaliveTooHigh)
        ));
        assert!(matches!(
            validate_persistent_keepalive("abc"),
            Err(ConfigEditError::KeepaliveNotNumeric)
        ));
    }

    #[test]
    fn test_config_diff_middle_change() {
        let diff = config_diff("a\nb\nc", "a\nd\nc");
        assert_eq!(
            diff,
            vec![
                DiffLine::Context("a".to_string()),
                DiffLine::Removed("b".to_string()),
                DiffLine::Added("d".to_string()),
                DiffLine::Context("c".to_string()),
            ]
        );
    }

    #[test]
    fn test_config_diff_identical() {
        let diff = config_diff("a\nb", "a\nb");
        assert_eq!(
            diff,
            vec![
                DiffLine::Context("a".to_string()),
                DiffLine::Context("b".to_string()),
            ]
        );
    }

    #[test]
    fn test_config_diff_single_line_change() {
        let diff = config_diff("a", "b");
        assert_eq!(
            diff,
            vec![
                DiffLine::Removed("a".to_string()),
                DiffLine::Added("b".to_string()),
            ]
        );
    }

    #[test]
    fn test_config_diff_new_longer_than_old() {
        // New has extra lines at the end — trailing Added loop
        let diff = config_diff("a\nb", "a\nb\nc\nd");
        assert_eq!(
            diff,
            vec![
                DiffLine::Context("a".to_string()),
                DiffLine::Context("b".to_string()),
                DiffLine::Added("c".to_string()),
                DiffLine::Added("d".to_string()),
            ]
        );
    }

    #[test]
    fn test_config_diff_old_longer_than_new() {
        // Old has extra lines at the end — trailing Removed loop
        let diff = config_diff("a\nb\nc\nd", "a\nb");
        assert_eq!(
            diff,
            vec![
                DiffLine::Context("a".to_string()),
                DiffLine::Context("b".to_string()),
                DiffLine::Removed("c".to_string()),
                DiffLine::Removed("d".to_string()),
            ]
        );
    }

    #[test]
    fn test_config_diff_empty_old() {
        let diff = config_diff("", "a\nb");
        assert_eq!(
            diff,
            vec![
                DiffLine::Added("a".to_string()),
                DiffLine::Added("b".to_string()),
            ]
        );
    }

    #[test]
    fn test_config_diff_empty_new() {
        let diff = config_diff("a\nb", "");
        assert_eq!(
            diff,
            vec![
                DiffLine::Removed("a".to_string()),
                DiffLine::Removed("b".to_string()),
            ]
        );
    }

    // ── apply_field unit tests ────────────────────────────────────────────────

    fn make_draft() -> WgConfig {
        use ferro_wg_core::config::{InterfaceConfig, PeerConfig};
        use ferro_wg_core::key::PrivateKey;
        WgConfig {
            interface: InterfaceConfig {
                private_key: PrivateKey::generate(),
                listen_port: 0,
                addresses: Vec::new(),
                dns: Vec::new(),
                dns_search: Vec::new(),
                mtu: 0,
                fwmark: 0,
                pre_up: Vec::new(),
                post_up: Vec::new(),
                pre_down: Vec::new(),
                post_down: Vec::new(),
            },
            peers: vec![PeerConfig {
                name: String::new(),
                public_key: PrivateKey::generate().public_key(),
                preshared_key: None,
                endpoint: Some("198.51.100.1:51820".to_string()),
                allowed_ips: vec!["10.0.0.0/8".to_string()],
                persistent_keepalive: 0,
            }],
        }
    }

    #[test]
    fn apply_field_listen_port() {
        // Arrange
        let mut draft = make_draft();

        // Act
        apply_field(
            EditableField::ListenPort,
            "51820",
            &mut draft,
            ConfigSection::Interface,
        );

        // Assert
        assert_eq!(
            draft.interface.listen_port, 51820,
            "listen_port must be written to draft"
        );
    }

    #[test]
    fn apply_field_empty_port_sets_zero() {
        // Arrange
        let mut draft = make_draft();
        draft.interface.listen_port = 12345;

        // Act
        apply_field(
            EditableField::ListenPort,
            "",
            &mut draft,
            ConfigSection::Interface,
        );

        // Assert
        assert_eq!(
            draft.interface.listen_port, 0,
            "empty string must set listen_port to 0 (OS-assigned)"
        );
    }

    #[test]
    fn apply_field_addresses_comma_list() {
        // Arrange
        let mut draft = make_draft();

        // Act
        apply_field(
            EditableField::Addresses,
            "10.0.0.1/24, 192.168.1.0/32",
            &mut draft,
            ConfigSection::Interface,
        );

        // Assert
        assert_eq!(
            draft.interface.addresses,
            vec!["10.0.0.1/24", "192.168.1.0/32"],
            "addresses must be split on commas with whitespace trimmed"
        );
    }

    #[test]
    fn apply_field_peer_allowed_ips() {
        // Arrange
        let mut draft = make_draft();

        // Act
        apply_field(
            EditableField::PeerAllowedIps,
            "0.0.0.0/0, ::/0",
            &mut draft,
            ConfigSection::Peer(0),
        );

        // Assert
        assert_eq!(
            draft.peers[0].allowed_ips,
            vec!["0.0.0.0/0", "::/0"],
            "allowed_ips must be written to the correct peer"
        );
    }

    #[test]
    fn apply_field_peer_public_key_is_no_op() {
        // Arrange
        let mut draft = make_draft();
        let original_key = draft.peers[0].public_key.to_base64();
        let some_key = "/yt5f1nclaUwO75kn6KosqO2ZD6kJ4Ld4SrYuG1csZg=";

        // Act: calling apply_field for PeerPublicKey must be a no-op
        apply_field(
            EditableField::PeerPublicKey,
            some_key,
            &mut draft,
            ConfigSection::Peer(0),
        );

        // Assert: key is unchanged
        assert_eq!(
            draft.peers[0].public_key.to_base64(),
            original_key,
            "apply_field(PeerPublicKey) must not modify the draft (it is a no-op)"
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

    // ── apply_field — DNS / numeric / hook fields ────────────────────────────

    #[test]
    fn apply_field_dns_parses_ip_list() {
        // Arrange
        let mut draft = make_draft();

        // Act
        apply_field(
            EditableField::Dns,
            "8.8.8.8, 1.1.1.1",
            &mut draft,
            ConfigSection::Interface,
        );

        // Assert
        let expected: Vec<std::net::IpAddr> = vec![
            "8.8.8.8".parse().expect("parse 8.8.8.8"),
            "1.1.1.1".parse().expect("parse 1.1.1.1"),
        ];
        assert_eq!(
            draft.interface.dns, expected,
            "dns must contain both parsed IpAddr values"
        );
    }

    #[test]
    fn apply_field_dns_empty_clears_list() {
        // Arrange
        let mut draft = make_draft();
        draft.interface.dns = vec!["8.8.8.8".parse().expect("parse ip")];

        // Act
        apply_field(EditableField::Dns, "", &mut draft, ConfigSection::Interface);

        // Assert
        assert!(
            draft.interface.dns.is_empty(),
            "empty input must clear the dns list"
        );
    }

    #[test]
    fn apply_field_dns_search_comma_list() {
        // Arrange
        let mut draft = make_draft();

        // Act
        apply_field(
            EditableField::DnsSearch,
            "example.com, sub.example.com",
            &mut draft,
            ConfigSection::Interface,
        );

        // Assert
        assert_eq!(
            draft.interface.dns_search,
            vec!["example.com", "sub.example.com"],
            "dns_search must be split and trimmed from the comma-separated input"
        );
    }

    #[test]
    fn apply_field_mtu_writes_to_draft() {
        // Arrange
        let mut draft = make_draft();

        // Act
        apply_field(
            EditableField::Mtu,
            "1420",
            &mut draft,
            ConfigSection::Interface,
        );

        // Assert
        assert_eq!(draft.interface.mtu, 1420, "mtu must be written to draft");
    }

    #[test]
    fn apply_field_mtu_empty_sets_zero() {
        // Arrange
        let mut draft = make_draft();
        draft.interface.mtu = 1500;

        // Act
        apply_field(EditableField::Mtu, "", &mut draft, ConfigSection::Interface);

        // Assert
        assert_eq!(
            draft.interface.mtu, 0,
            "empty string must set mtu to 0 (auto)"
        );
    }

    #[test]
    fn apply_field_fwmark_writes_to_draft() {
        // Arrange
        let mut draft = make_draft();

        // Act
        apply_field(
            EditableField::Fwmark,
            "100",
            &mut draft,
            ConfigSection::Interface,
        );

        // Assert
        assert_eq!(
            draft.interface.fwmark, 100,
            "fwmark must be written to draft"
        );
    }

    #[test]
    fn apply_field_fwmark_empty_sets_zero() {
        // Arrange
        let mut draft = make_draft();
        draft.interface.fwmark = 42;

        // Act
        apply_field(
            EditableField::Fwmark,
            "",
            &mut draft,
            ConfigSection::Interface,
        );

        // Assert
        assert_eq!(
            draft.interface.fwmark, 0,
            "empty string must set fwmark to 0"
        );
    }

    #[test]
    fn apply_field_pre_up_comma_list() {
        // Arrange
        let mut draft = make_draft();

        // Act
        apply_field(
            EditableField::PreUp,
            "iptables -A, ip route add",
            &mut draft,
            ConfigSection::Interface,
        );

        // Assert
        assert_eq!(
            draft.interface.pre_up,
            vec!["iptables -A", "ip route add"],
            "pre_up must be split on commas with whitespace trimmed"
        );
    }

    // ── apply_field — peer fields ────────────────────────────────────────────

    #[test]
    fn apply_field_peer_name_writes_to_draft() {
        // Arrange
        let mut draft = make_draft();

        // Act
        apply_field(
            EditableField::PeerName,
            "office-vpn",
            &mut draft,
            ConfigSection::Peer(0),
        );

        // Assert
        assert_eq!(
            draft.peers[0].name, "office-vpn",
            "peer name must be written to the correct peer"
        );
    }

    #[test]
    fn apply_field_peer_endpoint_writes_to_draft() {
        // Arrange
        let mut draft = make_draft();

        // Act
        apply_field(
            EditableField::PeerEndpoint,
            "198.51.100.1:51820",
            &mut draft,
            ConfigSection::Peer(0),
        );

        // Assert
        assert_eq!(
            draft.peers[0].endpoint.as_deref(),
            Some("198.51.100.1:51820"),
            "peer endpoint must be written to the correct peer"
        );
    }

    #[test]
    fn apply_field_peer_endpoint_empty_clears_endpoint() {
        // Arrange
        let mut draft = make_draft();
        // make_draft already sets endpoint to Some("198.51.100.1:51820")

        // Act
        apply_field(
            EditableField::PeerEndpoint,
            "",
            &mut draft,
            ConfigSection::Peer(0),
        );

        // Assert
        assert_eq!(
            draft.peers[0].endpoint, None,
            "empty endpoint string must set peer.endpoint to None"
        );
    }

    #[test]
    fn apply_field_peer_persistent_keepalive() {
        // Arrange
        let mut draft = make_draft();

        // Act
        apply_field(
            EditableField::PeerPersistentKeepalive,
            "25",
            &mut draft,
            ConfigSection::Peer(0),
        );

        // Assert
        assert_eq!(
            draft.peers[0].persistent_keepalive, 25,
            "persistent_keepalive must be written to the correct peer"
        );
    }

    #[test]
    fn apply_field_peer_out_of_bounds_is_no_op() {
        // Arrange: draft has no peers
        let mut draft = make_draft();
        draft.peers.clear();

        // Act: targeting a non-existent peer index must not panic or mutate
        apply_field(
            EditableField::PeerName,
            "ghost",
            &mut draft,
            ConfigSection::Peer(99),
        );

        // Assert
        assert!(
            draft.peers.is_empty(),
            "out-of-bounds peer index must be a no-op — peers must remain empty"
        );
    }
}
