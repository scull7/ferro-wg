//! Shared utility functions for the TUI.

use std::time::Duration;

/// Format a handshake age `Duration` as a compact human-readable string.
///
/// Shows only the most significant unit so the result fits neatly in a
/// table cell: `"25s ago"`, `"2m ago"`, `"1h ago"`, `"3days ago"`.
///
/// Uses [`humantime`] for unit selection and formatting.
#[must_use]
pub fn format_handshake_age(d: Duration) -> String {
    let full = humantime::format_duration(d).to_string();
    let first = full.split_whitespace().next().unwrap_or("?");
    format!("{first} ago")
}

/// Format a byte count into a human-readable string.
///
/// Uses binary units (KiB, MiB, GiB) with one decimal place.
#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;

    if bytes >= GIB {
        #[allow(clippy::cast_precision_loss)]
        let val = bytes as f64 / GIB as f64;
        format!("{val:.1} GiB")
    } else if bytes >= MIB {
        #[allow(clippy::cast_precision_loss)]
        let val = bytes as f64 / MIB as f64;
        format!("{val:.1} MiB")
    } else if bytes >= KIB {
        #[allow(clippy::cast_precision_loss)]
        let val = bytes as f64 / KIB as f64;
        format!("{val:.1} KiB")
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_handshake_age_seconds() {
        assert_eq!(format_handshake_age(Duration::from_secs(25)), "25s ago");
    }

    #[test]
    fn format_handshake_age_minutes() {
        assert_eq!(format_handshake_age(Duration::from_secs(120)), "2m ago");
    }

    #[test]
    fn format_handshake_age_hours() {
        assert_eq!(format_handshake_age(Duration::from_secs(3600)), "1h ago");
    }

    #[test]
    fn format_handshake_age_days() {
        assert_eq!(format_handshake_age(Duration::from_secs(86400)), "1day ago");
    }

    #[test]
    fn format_handshake_age_shows_only_most_significant_unit() {
        // 1h 30m 5s — should show only "1h", not "1h 30m 5s"
        assert_eq!(format_handshake_age(Duration::from_secs(5405)), "1h ago");
    }

    #[test]
    fn format_bytes_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1_048_576), "1.0 MiB");
        assert_eq!(format_bytes(1_073_741_824), "1.0 GiB");
    }
}
