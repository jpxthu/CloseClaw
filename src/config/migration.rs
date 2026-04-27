//! Configuration migration from legacy `openclaw.json` to the new `config/` directory layout.
//!
//! This module handles the one-time migration of a legacy `openclaw.json` file into the
//! new structured `config/` directory, splitting it into domain-specific JSON files.

use std::fs;
pub use std::path::{Path, PathBuf};

pub use serde_json::Value;

/// Errors that can occur during configuration migration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigMigrationError {
    #[error("failed to read openclaw.json: {0}")]
    ReadError(std::io::Error),

    #[error("failed to parse openclaw.json: {0}")]
    ParseError(#[from] serde_json::Error),

    #[error("failed to write config file: {0}")]
    WriteError(std::io::Error),

    #[error("migration aborted: openclaw.json is malformed")]
    MalformedJson,

    #[error("openclaw.json does not exist at {0}")]
    NotFound(String),
}

/// Check if migration is needed and perform it if so.
///
/// Migration is triggered when ALL of the following conditions are met:
/// - `openclaw_json_path` exists (openclaw.json is present)
/// - `config_dir` does NOT exist (new config layout not yet present)
/// - `config_dir / "openclaw_migrated"` marker file does NOT exist
///
/// On success:
/// - `config/` directory is created with split JSON files
/// - `openclaw.json` is renamed to `openclaw.json.bak`
/// - `config/.backups/` directory is created
/// - `config/openclaw_migrated` marker file is created
///
/// # Arguments
/// * `openclaw_json_path` — absolute path to the legacy openclaw.json
/// * `config_dir` — absolute path to the config directory (e.g. `~/.closeclaw/config`)
pub fn migrate_if_needed(
    openclaw_json_path: impl AsRef<Path>,
    config_dir: impl AsRef<Path>,
) -> Result<bool, ConfigMigrationError> {
    let openclaw_path = openclaw_json_path.as_ref();
    let cfg_dir = config_dir.as_ref();

    // Condition 1: openclaw.json must exist
    if !openclaw_path.exists() {
        return Ok(false);
    }

    // Condition 2: config/ must not exist
    if cfg_dir.exists() {
        return Ok(false);
    }

    // Condition 3: marker file must not exist
    let marker_path = cfg_dir.join("openclaw_migrated");
    if marker_path.exists() {
        return Ok(false);
    }

    // Perform migration
    migrate(openclaw_path, cfg_dir)?;
    Ok(true)
}

/// Perform the actual migration from openclaw.json to config/ directory.
fn migrate(openclaw_path: &Path, config_dir: &Path) -> Result<(), ConfigMigrationError> {
    // Read and parse openclaw.json
    let content = fs::read_to_string(openclaw_path).map_err(ConfigMigrationError::ReadError)?;

    let json: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Err(ConfigMigrationError::MalformedJson),
    };

    // Create config/ directory
    fs::create_dir_all(config_dir).map_err(ConfigMigrationError::WriteError)?;

    // Create config/.backups/ directory
    let backups_dir = config_dir.join(".backups");
    fs::create_dir_all(&backups_dir).map_err(ConfigMigrationError::WriteError)?;

    // Write domain-specific config files
    write_if_present(config_dir, "models.json", json.get("models"))?;
    write_channels(config_dir, &json)?;
    write_if_present(config_dir, "gateway.json", json.get("gateway"))?;
    write_if_present(config_dir, "plugins.json", json.get("plugins"))?;
    write_system(config_dir, &json)?;
    write_credentials(config_dir, &json)?;

    // Rename openclaw.json → openclaw.json.bak
    let backup_path = PathBuf::from(format!("{}.bak", openclaw_path.to_string_lossy()));
    fs::rename(openclaw_path, &backup_path).map_err(ConfigMigrationError::WriteError)?;

    // Create marker file
    let marker_path = config_dir.join("openclaw_migrated");
    fs::write(&marker_path, "").map_err(ConfigMigrationError::WriteError)?;

    Ok(())
}

/// Write a JSON value to `config_dir/filename`, creating an empty `{}` if the value is None.
fn write_if_present(
    config_dir: &Path,
    filename: &str,
    value: Option<&Value>,
) -> Result<(), ConfigMigrationError> {
    let content = match value {
        Some(v) => serde_json::to_string_pretty(v),
        None => serde_json::to_string_pretty(&Value::Object(Default::default())),
    };
    let content = content.map_err(ConfigMigrationError::ParseError)?;
    let path = config_dir.join(filename);
    crate::config::manager::write_atomically(&path, content.as_bytes())
        .map_err(ConfigMigrationError::WriteError)?;
    Ok(())
}

/// Write channels.json by merging `channels` and `bindings` from the source JSON.
fn write_channels(config_dir: &Path, json: &Value) -> Result<(), ConfigMigrationError> {
    let default_channels = Value::Object(Default::default());
    let default_bindings = Value::Array(Default::default());
    let channels = json.get("channels").unwrap_or(&default_channels);
    let bindings = json.get("bindings").unwrap_or(&default_bindings);
    let merged = serde_json::json!({
        "channels": channels,
        "bindings": bindings,
    });
    let content =
        serde_json::to_string_pretty(&merged).map_err(ConfigMigrationError::ParseError)?;
    let path = config_dir.join("channels.json");
    crate::config::manager::write_atomically(&path, content.as_bytes())
        .map_err(ConfigMigrationError::WriteError)?;
    Ok(())
}

/// Write system.json by merging wizard, update, meta, messages, commands,
/// session, cron, hooks, browser, and auth (profiles only, no apiKey).
fn write_system(config_dir: &Path, json: &Value) -> Result<(), ConfigMigrationError> {
    let mut system = serde_json::Map::new();

    if let Some(v) = json.get("wizard") {
        system.insert("wizard".to_string(), v.clone());
    }
    if let Some(v) = json.get("update") {
        system.insert("update".to_string(), v.clone());
    }
    if let Some(v) = json.get("meta") {
        system.insert("meta".to_string(), v.clone());
    }
    if let Some(v) = json.get("messages") {
        system.insert("messages".to_string(), v.clone());
    }
    if let Some(v) = json.get("commands") {
        system.insert("commands".to_string(), v.clone());
    }
    if let Some(v) = json.get("session") {
        system.insert("session".to_string(), v.clone());
    }
    if let Some(v) = json.get("cron") {
        system.insert("cron".to_string(), v.clone());
    }
    if let Some(v) = json.get("hooks") {
        system.insert("hooks".to_string(), v.clone());
    }
    if let Some(v) = json.get("browser") {
        system.insert("browser".to_string(), v.clone());
    }
    if let Some(auth) = json.get("auth") {
        // Only carry forward profiles metadata, not apiKey
        let mut auth_map = serde_json::Map::new();
        if let Some(profiles) = auth.get("profiles") {
            auth_map.insert("profiles".to_string(), profiles.clone());
        }
        system.insert("auth".to_string(), Value::Object(auth_map));
    }

    let value = Value::Object(system);
    let content = serde_json::to_string_pretty(&value).map_err(ConfigMigrationError::ParseError)?;
    let path = config_dir.join("system.json");
    crate::config::manager::write_atomically(&path, content.as_bytes())
        .map_err(ConfigMigrationError::WriteError)?;
    Ok(())
}

/// Write per-provider credential files under `config/credentials/{provider}.json`
/// from `auth.profiles`.
fn write_credentials(config_dir: &Path, json: &Value) -> Result<(), ConfigMigrationError> {
    let auth = match json.get("auth") {
        Some(v) => v,
        None => return Ok(()),
    };
    let profiles = match auth.get("profiles") {
        Some(v) => v,
        None => return Ok(()),
    };
    let profiles_map = match profiles {
        Value::Object(m) => m,
        _ => return Ok(()),
    };

    let creds_dir = config_dir.join("credentials");
    fs::create_dir_all(&creds_dir).map_err(ConfigMigrationError::WriteError)?;

    for (provider, profile) in profiles_map {
        let mut cred_obj = serde_json::Map::new();
        cred_obj.insert("provider".to_string(), Value::String(provider.clone()));

        // Copy all credential fields: apiKey, appId, appSecret, botName, etc.
        if let Value::Object(profile_map) = profile {
            for (k, v) in profile_map {
                if k == "apiKey" || k == "appId" || k == "appSecret" || k == "botName" {
                    cred_obj.insert(k.clone(), v.clone());
                }
            }
        }

        let content = serde_json::to_string_pretty(&Value::Object(cred_obj))
            .map_err(ConfigMigrationError::ParseError)?;
        let filename = format!("{}.json", provider);
        let path = creds_dir.join(filename);
        crate::config::manager::write_atomically(&path, content.as_bytes())
            .map_err(ConfigMigrationError::WriteError)?;
    }

    Ok(())
}

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;
