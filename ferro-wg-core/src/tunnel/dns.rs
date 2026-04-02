//! DNS configuration for active tunnels.
//!
//! Applies DNS server and search-domain settings when a tunnel is brought up
//! and reverts them cleanly on teardown. Platform-specific implementations
//! are gated with `#[cfg(target_os = ...)]`.
//!
//! # Platform support
//! - **macOS** — `networksetup`, targeting the primary network service.
//! - **Linux** — `resolvectl` (systemd-resolved) with `/etc/resolv.conf` fallback.
//! - **Other** — no-op with a warning log.

use std::net::IpAddr;

use tracing::debug;

use crate::error::WgError;

// ── Error type ───────────────────────────────────────────────────────────────

/// Errors that can occur while applying or reverting tunnel DNS configuration.
#[derive(Debug, thiserror::Error)]
pub enum DnsError {
    /// An I/O error occurred launching a command or accessing a system file.
    ///
    /// `#[from]` allows using `?` directly on [`std::io::Result`] values
    /// without any `.map_err()` boilerplate.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// `networksetup` exited with a non-zero status (macOS).
    #[error("networksetup failed: {0}")]
    NetworkSetup(String),

    /// `resolvectl` exited with a non-zero status (Linux).
    #[error("resolvectl failed: {0}")]
    Resolvectl(String),

    /// The default-route interface could not be mapped to a network service
    /// (macOS).
    #[error("cannot determine primary network service: {0}")]
    ServiceDetection(String),
}

impl From<DnsError> for WgError {
    fn from(e: DnsError) -> Self {
        Self::Tunnel(e.to_string())
    }
}

// ── macOS ────────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod imp {
    use std::net::IpAddr;
    use std::process::Command;

    use tracing::debug;

    use super::DnsError;

    // Full paths for security in a root daemon context.
    const NETWORKSETUP: &str = "/usr/sbin/networksetup";
    const ROUTE: &str = "/sbin/route";

    /// The DNS configuration that was active on the primary network service
    /// before the tunnel was brought up.
    ///
    /// Stored inside [`DnsState`](super::DnsState) and used to restore the
    /// previous state when the tunnel is torn down.
    pub struct OriginalDnsConfig {
        pub(super) dns: Vec<IpAddr>,
        pub(super) search: Vec<String>,
    }

    /// DNS state captured at tunnel bring-up; used to restore the previous
    /// configuration when the tunnel is torn down.
    pub struct DnsState {
        pub(super) service_name: String,
        pub(super) original: OriginalDnsConfig,
    }

    /// Detect the primary macOS network service name by correlating the
    /// default-route interface with `networksetup -listnetworkserviceorder`.
    ///
    /// # Errors
    ///
    /// Returns [`DnsError::ServiceDetection`] if the default-route interface
    /// cannot be determined or mapped to a registered network service.
    pub fn detect_primary_service() -> Result<String, DnsError> {
        // 1. Find the interface used by the default route.
        let route_out = Command::new(ROUTE)
            .args(["-n", "get", "default"])
            .output()?;

        if !route_out.status.success() {
            return Err(DnsError::ServiceDetection(format!(
                "route -n get default failed: {}",
                String::from_utf8_lossy(&route_out.stderr).trim()
            )));
        }

        let route_stdout = String::from_utf8_lossy(&route_out.stdout);
        let iface = route_stdout
            .lines()
            .find_map(|l| {
                let l = l.trim();
                l.strip_prefix("interface:").map(str::trim)
            })
            .ok_or_else(|| {
                DnsError::ServiceDetection("could not parse default route interface".into())
            })?
            .to_owned();

        // 2. Map the interface name to a network service.
        // `networksetup -listnetworkserviceorder` emits lines like:
        //   (1) Wi-Fi
        //   (Hardware Port: Wi-Fi, Device: en0)
        let ns_out = Command::new(NETWORKSETUP)
            .args(["-listnetworkserviceorder"])
            .output()?;

        let ns_stdout = String::from_utf8_lossy(&ns_out.stdout);
        let mut service_name: Option<String> = None;
        let mut last_service: Option<String> = None;

        for line in ns_stdout.lines() {
            let line = line.trim();
            // Service name lines look like "(1) Wi-Fi"
            if line.starts_with('(') {
                if let Some(rest) = line.split_once(')') {
                    last_service = Some(rest.1.trim().to_owned());
                }
            }
            // Device lines look like "(Hardware Port: Wi-Fi, Device: en0)"
            if line.contains("Device:") {
                if let Some(dev) = line.split("Device:").nth(1) {
                    let dev = dev.trim().trim_end_matches(')').trim();
                    if dev == iface {
                        service_name.clone_from(&last_service);
                        break;
                    }
                }
            }
        }

        service_name.ok_or_else(|| {
            DnsError::ServiceDetection(format!(
                "interface {iface} not found in networksetup service list"
            ))
        })
    }

    /// Parse the output of `networksetup -getdnsservers <service>` into a
    /// list of [`IpAddr`] values.
    ///
    /// Returns an empty vec when no servers are configured (the command prints
    /// a human-readable sentence rather than IP addresses in that case).
    #[must_use]
    pub fn parse_networksetup_dns(output: &str) -> Vec<IpAddr> {
        output
            .lines()
            .filter_map(|l| l.trim().parse::<IpAddr>().ok())
            .collect()
    }

    /// Parse the output of `networksetup -getsearchdomains <service>` into a
    /// list of domain strings.
    ///
    /// Returns an empty vec when the output contains "There aren't any Search
    /// Domains" or similar.
    #[must_use]
    pub fn parse_networksetup_search(output: &str) -> Vec<String> {
        output
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.to_ascii_lowercase().contains("there aren"))
            .map(ToOwned::to_owned)
            .collect()
    }

    /// Apply DNS configuration via `networksetup`.
    ///
    /// The `iface` parameter is accepted for API uniformity with the Linux
    /// implementation but is not used on macOS — DNS is applied to the primary
    /// network service, not directly to a TUN interface.
    ///
    /// # Errors
    ///
    /// Returns [`DnsError`] if `networksetup` cannot be invoked or exits
    /// non-zero.
    pub fn apply(
        _iface: &str,
        servers: &[IpAddr],
        search: &[String],
    ) -> Result<DnsState, DnsError> {
        let service_name = detect_primary_service()?;

        // Save current DNS.
        let dns_out = Command::new(NETWORKSETUP)
            .args(["-getdnsservers", &service_name])
            .output()?;
        let prior_dns = parse_networksetup_dns(&String::from_utf8_lossy(&dns_out.stdout));

        // Save current search domains.
        let search_out = Command::new(NETWORKSETUP)
            .args(["-getsearchdomains", &service_name])
            .output()?;
        let prior_search = parse_networksetup_search(&String::from_utf8_lossy(&search_out.stdout));

        // Set new DNS servers.
        let mut args = vec!["-setdnsservers".to_owned(), service_name.clone()];
        args.extend(servers.iter().map(IpAddr::to_string));
        let out = Command::new(NETWORKSETUP).args(&args).output()?;
        if !out.status.success() {
            return Err(DnsError::NetworkSetup(
                String::from_utf8_lossy(&out.stderr).into_owned(),
            ));
        }

        // Set search domains if provided.
        if !search.is_empty() {
            let mut sargs = vec!["-setsearchdomains".to_owned(), service_name.clone()];
            sargs.extend(search.iter().cloned());
            let sout = Command::new(NETWORKSETUP).args(&sargs).output()?;
            if !sout.status.success() {
                return Err(DnsError::NetworkSetup(
                    String::from_utf8_lossy(&sout.stderr).into_owned(),
                ));
            }
        }

        debug!(
            "Applied DNS {:?} search {:?} on service '{service_name}'",
            servers, search
        );
        Ok(DnsState {
            service_name,
            original: OriginalDnsConfig {
                dns: prior_dns,
                search: prior_search,
            },
        })
    }

    /// Revert DNS configuration to the state captured in [`DnsState`].
    ///
    /// # Errors
    ///
    /// Returns [`DnsError`] if `networksetup` cannot be invoked or exits
    /// non-zero.
    pub fn revert(state: DnsState) -> Result<(), DnsError> {
        let DnsState {
            service_name,
            original,
        } = state;

        // Restore DNS servers (or clear them).
        let mut args = vec!["-setdnsservers".to_owned(), service_name.clone()];
        if original.dns.is_empty() {
            args.push("empty".to_owned());
        } else {
            args.extend(original.dns.iter().map(IpAddr::to_string));
        }
        let out = Command::new(NETWORKSETUP).args(&args).output()?;
        if !out.status.success() {
            return Err(DnsError::NetworkSetup(
                String::from_utf8_lossy(&out.stderr).into_owned(),
            ));
        }

        // Restore search domains.
        let mut sargs = vec!["-setsearchdomains".to_owned(), service_name.clone()];
        if original.search.is_empty() {
            sargs.push("empty".to_owned());
        } else {
            sargs.extend(original.search);
        }
        let sout = Command::new(NETWORKSETUP).args(&sargs).output()?;
        if !sout.status.success() {
            return Err(DnsError::NetworkSetup(
                String::from_utf8_lossy(&sout.stderr).into_owned(),
            ));
        }

        debug!("Reverted DNS on service '{service_name}'");
        Ok(())
    }
}

// ── Linux ────────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod imp {
    use std::net::IpAddr;
    use std::process::Command;

    use tracing::debug;

    use super::DnsError;

    // Full path for security in a root daemon context.
    const RESOLVECTL: &str = "/usr/bin/resolvectl";

    /// DNS state captured at tunnel bring-up; used to restore the previous
    /// configuration when the tunnel is torn down.
    pub enum DnsState {
        /// DNS was applied via `resolvectl`; revert with `resolvectl revert`.
        Resolved {
            /// The tunnel interface name.
            iface: String,
        },
        /// DNS was applied by prepending lines to `/etc/resolv.conf`; revert
        /// by writing back the saved original contents.
        ResolvConf {
            /// Original contents of `/etc/resolv.conf` before modification.
            backup: String,
        },
    }

    /// Returns `true` if `resolvectl` is available at its canonical path.
    fn resolvectl_available() -> bool {
        std::path::Path::new(RESOLVECTL).exists()
    }

    /// Apply DNS via `resolvectl` (systemd-resolved).
    fn apply_resolvectl(
        iface: &str,
        servers: &[IpAddr],
        search: &[String],
    ) -> Result<(), DnsError> {
        // Set DNS servers on the interface.
        let mut args = vec!["dns".to_owned(), iface.to_owned()];
        args.extend(servers.iter().map(IpAddr::to_string));
        let out = Command::new(RESOLVECTL).args(&args).output()?;
        if !out.status.success() {
            return Err(DnsError::Resolvectl(
                String::from_utf8_lossy(&out.stderr).into_owned(),
            ));
        }

        // Set domain routing: ~. catches all queries; append explicit search
        // domains if provided.
        let mut dargs = vec!["domain".to_owned(), iface.to_owned(), "~.".to_owned()];
        dargs.extend(search.iter().cloned());
        let dout = Command::new(RESOLVECTL).args(&dargs).output()?;
        if !dout.status.success() {
            return Err(DnsError::Resolvectl(
                String::from_utf8_lossy(&dout.stderr).into_owned(),
            ));
        }

        debug!("Applied DNS via resolvectl on {iface}");
        Ok(())
    }

    /// Apply DNS by atomically replacing `/etc/resolv.conf`.
    ///
    /// Writes to a temp file in the same directory and renames into place to
    /// avoid a window where `/etc/resolv.conf` is truncated but not yet
    /// written (race condition visible to concurrent DNS resolvers).
    fn apply_resolv_conf(servers: &[IpAddr], search: &[String]) -> Result<String, DnsError> {
        let backup = std::fs::read_to_string("/etc/resolv.conf")?;

        let mut prepend = String::new();
        for ip in servers {
            prepend.push_str(&format!("nameserver {ip}\n"));
        }
        if !search.is_empty() {
            prepend.push_str(&format!("search {}\n", search.join(" ")));
        }

        let new_contents = format!("{prepend}{backup}");

        // Write atomically: temp file → rename.
        let tmp_path = "/etc/resolv.conf.ferro-wg.tmp";
        std::fs::write(tmp_path, &new_contents)?;
        std::fs::rename(tmp_path, "/etc/resolv.conf")?;

        debug!("Applied DNS via /etc/resolv.conf (atomic replace)");
        Ok(backup)
    }

    /// Apply DNS configuration for the given interface.
    ///
    /// # Errors
    ///
    /// Returns [`DnsError`] if both `resolvectl` and `/etc/resolv.conf`
    /// modification fail.
    pub fn apply(iface: &str, servers: &[IpAddr], search: &[String]) -> Result<DnsState, DnsError> {
        if resolvectl_available() {
            apply_resolvectl(iface, servers, search)?;
            Ok(DnsState::Resolved {
                iface: iface.to_owned(),
            })
        } else {
            let backup = apply_resolv_conf(servers, search)?;
            Ok(DnsState::ResolvConf { backup })
        }
    }

    /// Revert DNS configuration to the state captured in [`DnsState`].
    ///
    /// # Errors
    ///
    /// Returns [`DnsError`] if the revert command fails or the file cannot be
    /// restored.
    pub fn revert(state: DnsState) -> Result<(), DnsError> {
        match state {
            DnsState::Resolved { iface } => {
                let out = Command::new(RESOLVECTL).args(["revert", &iface]).output()?;
                if !out.status.success() {
                    return Err(DnsError::Resolvectl(
                        String::from_utf8_lossy(&out.stderr).into_owned(),
                    ));
                }
                debug!("Reverted DNS via resolvectl on {iface}");
            }
            DnsState::ResolvConf { backup } => {
                // Restore atomically.
                let tmp_path = "/etc/resolv.conf.ferro-wg.tmp";
                std::fs::write(tmp_path, &backup)?;
                std::fs::rename(tmp_path, "/etc/resolv.conf")?;
                debug!("Restored /etc/resolv.conf from backup (atomic replace)");
            }
        }
        Ok(())
    }
}

// ── Unsupported platforms ────────────────────────────────────────────────────

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    use std::net::IpAddr;

    use super::DnsError;

    /// No-op DNS state for unsupported platforms.
    pub struct DnsState;

    pub fn apply(
        _iface: &str,
        _servers: &[IpAddr],
        _search: &[String],
    ) -> Result<DnsState, DnsError> {
        Ok(DnsState)
    }

    pub fn revert(_state: DnsState) -> Result<(), DnsError> {
        Ok(())
    }
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Opaque handle capturing the DNS state applied during tunnel bring-up.
///
/// This is an explicit teardown token — call [`remove_dns`] with this value
/// when the tunnel comes down to revert all DNS changes. If `remove_dns` is
/// not called (e.g. process crash), the system DNS remains in the tunnel
/// state; a future bring-up of the same tunnel will overwrite and restore it
/// correctly.
///
/// # RAII note
///
/// There is intentionally no `Drop` impl: DNS revert is fallible and requires
/// logging, neither of which is appropriate inside `drop`. Always call
/// [`remove_dns`] explicitly in the teardown path.
pub struct DnsState(imp::DnsState);

/// Apply DNS servers and search domains for the given tunnel interface.
///
/// Returns `Ok(None)` when `servers` is empty — no system changes are made.
/// Returns `Err(DnsError)` on platform-level failure so the caller can decide
/// whether to treat it as fatal.
///
/// # Errors
///
/// Returns [`DnsError`] if the platform DNS command fails.
pub fn apply_dns(
    iface: &str,
    servers: &[IpAddr],
    search: &[String],
) -> Result<Option<DnsState>, DnsError> {
    if servers.is_empty() {
        debug!("No DNS servers configured for {iface}, skipping");
        return Ok(None);
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        tracing::warn!("DNS configuration is not supported on this platform");
        let _ = (iface, servers, search);
        return Ok(None);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let state = imp::apply(iface, servers, search)?;
        Ok(Some(DnsState(state)))
    }
}

/// Revert DNS changes captured in a [`DnsState`] returned by [`apply_dns`].
///
/// # Errors
///
/// Returns [`DnsError`] if the platform revert command fails.
pub fn remove_dns(state: DnsState) -> Result<(), DnsError> {
    imp::revert(state.0)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_skipped_when_empty_servers() {
        // Must not invoke any shell commands — safe to run anywhere.
        let result = apply_dns("utun0", &[], &[]);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    // macOS-specific pure-function tests (no root required).
    #[cfg(target_os = "macos")]
    mod macos {
        use std::net::IpAddr;

        use super::imp::{parse_networksetup_dns, parse_networksetup_search};
        use super::{apply_dns, remove_dns};

        #[test]
        fn parse_networksetup_dns_output_empty() {
            let output = "There aren't any DNS Servers set on Wi-Fi.\n";
            assert!(parse_networksetup_dns(output).is_empty());
        }

        #[test]
        fn parse_networksetup_dns_output_ips() {
            let output = "1.1.1.1\n8.8.8.8\n";
            let ips = parse_networksetup_dns(output);
            assert_eq!(ips.len(), 2);
            assert_eq!(ips[0].to_string(), "1.1.1.1");
            assert_eq!(ips[1].to_string(), "8.8.8.8");
        }

        #[test]
        fn parse_networksetup_dns_ipv6() {
            let output = "2606:4700:4700::1111\n";
            let ips = parse_networksetup_dns(output);
            assert_eq!(ips.len(), 1);
        }

        #[test]
        fn parse_networksetup_search_empty() {
            let output = "There aren't any Search Domains set on Wi-Fi.\n";
            assert!(parse_networksetup_search(output).is_empty());
        }

        #[test]
        fn parse_networksetup_search_domains() {
            let output = "corp.internal\ndev.internal\n";
            let domains = parse_networksetup_search(output);
            assert_eq!(domains, vec!["corp.internal", "dev.internal"]);
        }

        #[test]
        #[ignore = "requires root and a real macOS network interface"]
        fn apply_and_remove_dns_macos() {
            let servers: Vec<IpAddr> = vec!["1.1.1.1".parse().unwrap()];
            let search = vec!["test.internal".to_owned()];
            let state = apply_dns("utun0", &servers, &search)
                .expect("apply")
                .expect("state");
            remove_dns(state).expect("remove");
        }
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use std::net::IpAddr;

        use super::{apply_dns, remove_dns};

        #[test]
        #[ignore = "requires systemd-resolved and a real tunnel interface"]
        fn apply_and_remove_dns_linux_resolvectl() {
            let servers: Vec<IpAddr> = vec!["1.1.1.1".parse().unwrap()];
            let state = apply_dns("wg0", &servers, &[])
                .expect("apply")
                .expect("state");
            remove_dns(state).expect("remove");
        }
    }
}
