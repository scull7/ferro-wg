//! Native TOML configuration loader and writer.
//!
//! Loads and saves [`WgConfig`] and [`AppConfig`] in the ferro-wg native
//! format, stored at `~/.config/ferro-wg/config.toml` by default.

use std::path::Path;

use crate::config::{AppConfig, WgConfig};
use crate::error::ConfigError;

/// Load a [`WgConfig`] from a TOML file on disk.
///
/// # Errors
///
/// Returns [`ConfigError::TomlParse`] if the file cannot be read or parsed,
/// or a validation error from [`WgConfig::validate`].
pub fn load_from_file(path: &Path) -> Result<WgConfig, ConfigError> {
    let contents =
        std::fs::read_to_string(path).map_err(|e| ConfigError::TomlParse(e.to_string()))?;
    load_from_str(&contents)
}

/// Parse a [`WgConfig`] from a TOML string.
///
/// # Errors
///
/// Returns [`ConfigError::TomlParse`] if deserialization fails,
/// or a validation error from [`WgConfig::validate`].
pub fn load_from_str(s: &str) -> Result<WgConfig, ConfigError> {
    let config: WgConfig = toml::from_str(s).map_err(|e| ConfigError::TomlParse(e.to_string()))?;
    config.validate()?;
    Ok(config)
}

/// Serialize a [`WgConfig`] to a pretty-printed TOML string.
///
/// # Errors
///
/// Returns [`ConfigError::TomlParse`] if serialization fails (unlikely for valid configs).
pub fn save_to_string(config: &WgConfig) -> Result<String, ConfigError> {
    toml::to_string_pretty(config).map_err(|e| ConfigError::TomlParse(e.to_string()))
}

/// Write a [`WgConfig`] to a TOML file on disk.
///
/// Creates parent directories if they don't exist.
///
/// # Errors
///
/// Returns [`ConfigError::TomlParse`] if serialization or file I/O fails.
pub fn save_to_file(config: &WgConfig, path: &Path) -> Result<(), ConfigError> {
    let contents = save_to_string(config)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ConfigError::TomlParse(format!("create dir: {e}")))?;
    }
    std::fs::write(path, contents).map_err(|e| ConfigError::TomlParse(e.to_string()))
}

// -- AppConfig (named connections) --

/// Load an [`AppConfig`] from a TOML file on disk.
///
/// # Errors
///
/// Returns [`ConfigError::TomlParse`] if the file cannot be read or parsed.
pub fn load_app_config(path: &Path) -> Result<AppConfig, ConfigError> {
    let contents =
        std::fs::read_to_string(path).map_err(|e| ConfigError::TomlParse(e.to_string()))?;
    load_app_config_str(&contents)
}

/// Parse an [`AppConfig`] from a TOML string.
///
/// # Errors
///
/// Returns [`ConfigError::TomlParse`] if deserialization fails.
pub fn load_app_config_str(s: &str) -> Result<AppConfig, ConfigError> {
    let config: AppConfig =
        ::toml::from_str(s).map_err(|e| ConfigError::TomlParse(e.to_string()))?;
    config.validate()?;
    Ok(config)
}

/// Serialize an [`AppConfig`] to a pretty-printed TOML string.
///
/// # Errors
///
/// Returns [`ConfigError::TomlParse`] if serialization fails.
pub fn save_app_config_string(config: &AppConfig) -> Result<String, ConfigError> {
    ::toml::to_string_pretty(config).map_err(|e| ConfigError::TomlParse(e.to_string()))
}

/// Write an [`AppConfig`] to a TOML file on disk.
///
/// Creates parent directories if they don't exist.
///
/// # Errors
///
/// Returns [`ConfigError::TomlParse`] if serialization or file I/O fails.
pub fn save_app_config(config: &AppConfig, path: &Path) -> Result<(), ConfigError> {
    let contents = save_app_config_string(config)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ConfigError::TomlParse(format!("create dir: {e}")))?;
    }
    std::fs::write(path, contents).map_err(|e| ConfigError::TomlParse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::PrivateKey;

    const SAMPLE_TOML: &str = r#"
[interface]
private_key = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk="
listen_port = 51820
addresses = ["10.0.0.2/24"]
dns = ["1.1.1.1"]
mtu = 1420

[[peers]]
name = "tw-dc-sjc01"
public_key = "HIgo9xNzJMWLKASShiTqIybxZ0U3wGLiUeJ1PKf8ykw="
endpoint = "198.51.100.1:51820"
allowed_ips = ["10.100.0.0/16"]
persistent_keepalive = 25
"#;

    #[test]
    fn parse_sample_toml() {
        let config = load_from_str(SAMPLE_TOML).expect("parse");
        assert_eq!(config.interface.listen_port, 51820);
        assert_eq!(config.interface.mtu, 1420);
        assert_eq!(config.peers.len(), 1);
        assert_eq!(config.peers[0].name, "tw-dc-sjc01");
        assert_eq!(config.peers[0].persistent_keepalive, 25);
    }

    #[test]
    fn roundtrip_toml() {
        let config = load_from_str(SAMPLE_TOML).expect("parse");
        let serialized = save_to_string(&config).expect("serialize");
        let reparsed = load_from_str(&serialized).expect("reparse");
        assert_eq!(reparsed.interface.listen_port, config.interface.listen_port);
        assert_eq!(reparsed.peers.len(), config.peers.len());
        assert_eq!(reparsed.peers[0].name, config.peers[0].name);
    }

    #[test]
    fn save_and_load_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.toml");

        let config = load_from_str(SAMPLE_TOML).expect("parse");
        save_to_file(&config, &path).expect("save");

        let loaded = load_from_file(&path).expect("load");
        assert_eq!(loaded.interface.listen_port, 51820);
        assert_eq!(loaded.peers[0].name, "tw-dc-sjc01");
    }

    #[test]
    fn missing_peers_rejected() {
        let toml = r#"
[interface]
private_key = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk="
"#;
        let result = load_from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_toml_rejected() {
        let result = load_from_str("this is not valid toml {{{}}}");
        assert!(matches!(result, Err(ConfigError::TomlParse(_))));
    }

    #[test]
    fn multiple_peers_parsed() {
        let private = PrivateKey::generate();
        let pub1 = PrivateKey::generate().public_key();
        let pub2 = PrivateKey::generate().public_key();

        let toml = format!(
            r#"
[interface]
private_key = "{}"

[[peers]]
name = "dc1"
public_key = "{}"
allowed_ips = ["10.0.0.0/8"]
endpoint = "1.2.3.4:51820"

[[peers]]
name = "dc2"
public_key = "{}"
allowed_ips = ["172.16.0.0/12"]
endpoint = "5.6.7.8:51820"
"#,
            private.to_base64(),
            pub1.to_base64(),
            pub2.to_base64()
        );

        let config = load_from_str(&toml).expect("parse");
        assert_eq!(config.peers.len(), 2);
        assert_eq!(config.peers[0].name, "dc1");
        assert_eq!(config.peers[1].name, "dc2");
    }

    #[test]
    fn app_config_roundtrip() {
        let mut app = AppConfig::default();
        app.insert("mia".into(), load_from_str(SAMPLE_TOML).expect("parse"));

        let serialized = save_app_config_string(&app).expect("serialize");
        let back = load_app_config_str(&serialized).expect("reparse");
        assert_eq!(back.connection_names(), vec!["mia"]);
        assert_eq!(back.get("mia").expect("mia").interface.listen_port, 51820);
    }

    #[test]
    fn app_config_multiple_connections() {
        let mut app = AppConfig::default();
        let conn1 = load_from_str(SAMPLE_TOML).expect("parse");
        let conn2 = load_from_str(SAMPLE_TOML).expect("parse");
        app.insert("mia".into(), conn1);
        app.insert("tus1".into(), conn2);

        let serialized = save_app_config_string(&app).expect("serialize");
        let back = load_app_config_str(&serialized).expect("reparse");
        assert_eq!(back.connections.len(), 2);
        assert!(back.get("mia").is_some());
        assert!(back.get("tus1").is_some());
    }

    #[test]
    fn app_config_save_and_load_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.toml");

        let mut app = AppConfig::default();
        app.insert("mia".into(), load_from_str(SAMPLE_TOML).expect("parse"));

        save_app_config(&app, &path).expect("save");
        let loaded = load_app_config(&path).expect("load");
        assert_eq!(loaded.connection_names(), vec!["mia"]);
    }
}
