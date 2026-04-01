//! Error types for the `WireGuard` core library.

use std::fmt;
use std::net::AddrParseError;

/// Top-level error type for `ferro-wg-core`.
#[derive(Debug, thiserror::Error)]
pub enum WgError {
    /// Invalid `WireGuard` configuration.
    #[error("config error: {0}")]
    Config(#[from] ConfigError),

    /// Key encoding or decoding failure.
    #[error("key error: {0}")]
    Key(#[from] KeyError),

    /// Tunnel-level protocol error.
    #[error("tunnel error: {0}")]
    Tunnel(String),

    /// The requested backend is not compiled in.
    #[error("backend {0} not available (enable the cargo feature)")]
    BackendUnavailable(String),
}

/// Errors related to configuration parsing and validation.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A required field is missing.
    #[error("missing required field: {0}")]
    MissingField(&'static str),

    /// A field value is out of range or malformed.
    #[error("invalid value for {field}: {reason}")]
    InvalidValue {
        /// The field name.
        field: &'static str,
        /// Why the value is invalid.
        reason: String,
    },

    /// Failed to parse an IP address or CIDR.
    #[error("address parse error: {0}")]
    AddrParse(#[from] AddrParseError),

    /// TOML deserialization failure.
    #[error("toml parse error: {0}")]
    TomlParse(String),

    /// `wg-quick` config parse failure.
    #[error("wg-quick parse error at line {line}: {reason}")]
    WgQuickParse {
        /// Line number (1-based).
        line: usize,
        /// What went wrong.
        reason: String,
    },
}

/// Errors related to cryptographic key operations.
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    /// Base64 decoding failed.
    #[error("invalid base64: {0}")]
    InvalidBase64(String),

    /// Decoded key has the wrong length.
    #[error("expected {expected} bytes, got {actual}")]
    InvalidLength {
        /// Expected byte count.
        expected: usize,
        /// Actual byte count.
        actual: usize,
    },
}

/// Identifies which backend produced a tunnel error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum BackendKind {
    /// Cloudflare's `boringtun`.
    Boringtun,
    /// `NordSecurity`'s `neptun`.
    Neptun,
    /// Mullvad's `gotatun`.
    Gotatun,
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boringtun => f.write_str("boringtun"),
            Self::Neptun => f.write_str("neptun"),
            Self::Gotatun => f.write_str("gotatun"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind_display() {
        assert_eq!(BackendKind::Boringtun.to_string(), "boringtun");
        assert_eq!(BackendKind::Neptun.to_string(), "neptun");
        assert_eq!(BackendKind::Gotatun.to_string(), "gotatun");
    }

    #[test]
    fn config_error_missing_field() {
        let err = ConfigError::MissingField("private_key");
        assert_eq!(err.to_string(), "missing required field: private_key");
    }

    #[test]
    fn key_error_invalid_length() {
        let err = KeyError::InvalidLength {
            expected: 32,
            actual: 16,
        };
        assert_eq!(err.to_string(), "expected 32 bytes, got 16");
    }

    #[test]
    fn wg_error_from_config_error() {
        let config_err = ConfigError::MissingField("endpoint");
        let wg_err: WgError = config_err.into();
        assert!(matches!(wg_err, WgError::Config(_)));
    }

    #[test]
    fn wg_error_from_key_error() {
        let key_err = KeyError::InvalidBase64("bad input".into());
        let wg_err: WgError = key_err.into();
        assert!(matches!(wg_err, WgError::Key(_)));
    }

    #[test]
    fn backend_kind_serde_roundtrip() {
        let kind = BackendKind::Gotatun;
        let json = serde_json::to_string(&kind).expect("serialize");
        let back: BackendKind = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, kind);
    }
}
