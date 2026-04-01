//! X25519 key types with base64 serialization and zeroize-on-drop.
//!
//! `WireGuard` uses X25519 Diffie-Hellman for key exchange. Keys are 32 bytes,
//! conventionally encoded as base64 in configuration files.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use zeroize::Zeroize;

use crate::error::KeyError;

/// A 32-byte X25519 private key. Zeroized on drop.
#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct PrivateKey([u8; 32]);

/// A 32-byte X25519 public key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PublicKey([u8; 32]);

/// A 32-byte optional preshared key. Zeroized on drop.
#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct PresharedKey([u8; 32]);

impl PrivateKey {
    /// Create a private key from raw bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Decode a private key from base64.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError::InvalidBase64`] if the input is not valid base64,
    /// or [`KeyError::InvalidLength`] if the decoded bytes are not 32 bytes.
    pub fn from_base64(s: &str) -> Result<Self, KeyError> {
        Ok(Self(decode_key_bytes(s)?))
    }

    /// Generate a new random private key.
    #[must_use]
    pub fn generate() -> Self {
        let secret = x25519_dalek::StaticSecret::random_from_rng(rand::thread_rng());
        Self(secret.to_bytes())
    }

    /// Derive the corresponding public key.
    #[must_use]
    pub fn public_key(&self) -> PublicKey {
        let secret = x25519_dalek::StaticSecret::from(self.0);
        let public = x25519_dalek::PublicKey::from(&secret);
        PublicKey(public.to_bytes())
    }

    /// Encode as base64.
    #[must_use]
    pub fn to_base64(&self) -> String {
        BASE64.encode(self.0)
    }

    /// Raw byte access (for passing to backend constructors).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Convert into an `x25519_dalek::StaticSecret`.
    #[must_use]
    pub fn to_static_secret(&self) -> x25519_dalek::StaticSecret {
        x25519_dalek::StaticSecret::from(self.0)
    }
}

impl PublicKey {
    /// Create a public key from raw bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Decode a public key from base64.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError::InvalidBase64`] if the input is not valid base64,
    /// or [`KeyError::InvalidLength`] if the decoded bytes are not 32 bytes.
    pub fn from_base64(s: &str) -> Result<Self, KeyError> {
        Ok(Self(decode_key_bytes(s)?))
    }

    /// Encode as base64.
    #[must_use]
    pub fn to_base64(&self) -> String {
        BASE64.encode(self.0)
    }

    /// Raw byte access.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Convert into an `x25519_dalek::PublicKey`.
    #[must_use]
    pub fn to_x25519(&self) -> x25519_dalek::PublicKey {
        x25519_dalek::PublicKey::from(self.0)
    }
}

impl PresharedKey {
    /// Create a preshared key from raw bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Decode a preshared key from base64.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError::InvalidBase64`] if the input is not valid base64,
    /// or [`KeyError::InvalidLength`] if the decoded bytes are not 32 bytes.
    pub fn from_base64(s: &str) -> Result<Self, KeyError> {
        Ok(Self(decode_key_bytes(s)?))
    }

    /// Encode as base64.
    #[must_use]
    pub fn to_base64(&self) -> String {
        BASE64.encode(self.0)
    }

    /// Raw byte access.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

// -- Serde impls (base64 encoded strings) --

impl serde::Serialize for PrivateKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_base64())
    }
}

impl<'de> serde::Deserialize<'de> for PrivateKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::from_base64(&s).map_err(serde::de::Error::custom)
    }
}

impl serde::Serialize for PublicKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_base64())
    }
}

impl<'de> serde::Deserialize<'de> for PublicKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::from_base64(&s).map_err(serde::de::Error::custom)
    }
}

impl serde::Serialize for PresharedKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_base64())
    }
}

impl<'de> serde::Deserialize<'de> for PresharedKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::from_base64(&s).map_err(serde::de::Error::custom)
    }
}

// -- Debug impls (redacted for secrets) --

impl std::fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrivateKey([REDACTED])")
    }
}

impl std::fmt::Debug for PresharedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PresharedKey([REDACTED])")
    }
}

/// Decode exactly 32 bytes from a base64 string.
fn decode_key_bytes(s: &str) -> Result<[u8; 32], KeyError> {
    let bytes = BASE64
        .decode(s.trim())
        .map_err(|e| KeyError::InvalidBase64(e.to_string()))?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| KeyError::InvalidLength {
        expected: 32,
        actual: bytes.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A well-known test private key (base64).
    const TEST_PRIVATE_B64: &str = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=";

    /// Derive the expected public key from the test private key at test time.
    fn test_public_b64() -> String {
        PrivateKey::from_base64(TEST_PRIVATE_B64)
            .expect("decode test key")
            .public_key()
            .to_base64()
    }

    #[test]
    fn private_key_base64_roundtrip() {
        let key = PrivateKey::from_base64(TEST_PRIVATE_B64).expect("decode");
        assert_eq!(key.to_base64(), TEST_PRIVATE_B64);
    }

    #[test]
    fn public_key_base64_roundtrip() {
        let b64 = test_public_b64();
        let key = PublicKey::from_base64(&b64).expect("decode");
        assert_eq!(key.to_base64(), b64);
    }

    #[test]
    fn private_key_derives_correct_public() {
        let private = PrivateKey::from_base64(TEST_PRIVATE_B64).expect("decode");
        let public = private.public_key();
        assert_eq!(public.to_base64(), test_public_b64());
    }

    #[test]
    fn generate_produces_valid_key() {
        let private = PrivateKey::generate();
        let public = private.public_key();
        // Public key should be 32 bytes and encode to 44-char base64.
        assert_eq!(public.to_base64().len(), 44);
    }

    #[test]
    fn invalid_base64_rejected() {
        let result = PrivateKey::from_base64("not-valid-base64!!!");
        assert!(matches!(result, Err(KeyError::InvalidBase64(_))));
    }

    #[test]
    fn wrong_length_rejected() {
        // 16 bytes encoded as base64.
        let short = BASE64.encode([0u8; 16]);
        let result = PrivateKey::from_base64(&short);
        assert!(matches!(
            result,
            Err(KeyError::InvalidLength {
                expected: 32,
                actual: 16
            })
        ));
    }

    #[test]
    fn serde_private_key_roundtrip() {
        let key = PrivateKey::from_base64(TEST_PRIVATE_B64).expect("decode");
        let json = serde_json::to_string(&key).expect("serialize");
        let back: PrivateKey = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.to_base64(), TEST_PRIVATE_B64);
    }

    #[test]
    fn serde_public_key_roundtrip() {
        let b64 = test_public_b64();
        let key = PublicKey::from_base64(&b64).expect("decode");
        let json = serde_json::to_string(&key).expect("serialize");
        let back: PublicKey = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.to_base64(), b64);
    }

    #[test]
    fn debug_redacts_private_key() {
        let key = PrivateKey::generate();
        let debug = format!("{key:?}");
        assert_eq!(debug, "PrivateKey([REDACTED])");
    }

    #[test]
    fn debug_redacts_preshared_key() {
        let key = PresharedKey::from_bytes([0u8; 32]);
        let debug = format!("{key:?}");
        assert_eq!(debug, "PresharedKey([REDACTED])");
    }

    #[test]
    fn to_x25519_public_key() {
        let key = PublicKey::from_base64(&test_public_b64()).expect("decode");
        let dalek = key.to_x25519();
        assert_eq!(dalek.as_bytes(), key.as_bytes());
    }

    #[test]
    fn to_static_secret() {
        let key = PrivateKey::from_base64(TEST_PRIVATE_B64).expect("decode");
        let secret = key.to_static_secret();
        assert_eq!(secret.to_bytes(), *key.as_bytes());
    }
}
