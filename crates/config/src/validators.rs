//! Validators for hot-reloaded config sections.
//!
//! Each validator performs structural checks and lightweight business
//! validation (field presence, range, format) on the parsed JSON value.
//! Deep cross-section validation (e.g., credentials reference resolution)
//! belongs in the startup path via Provider `validate()` methods.

use crate::manager::ConfigSection;
use crate::providers::channels::ALLOWED_CHANNEL_TYPES;
use crate::SectionValidator;

// ---------------------------------------------------------------------------
// Public helpers
// ---------------------------------------------------------------------------

/// Build the default `SectionValidator` for a given config section.
///
/// Returns a boxed `dyn Fn` that can be passed directly to
/// `reload_section()`.
pub fn for_section(section: ConfigSection) -> Box<SectionValidator> {
    match section {
        ConfigSection::Models => Box::new(validate_models),
        ConfigSection::Channels => Box::new(validate_channels),
        ConfigSection::Gateway => Box::new(validate_gateway),
        ConfigSection::Plugins => Box::new(validate_plugins),
        ConfigSection::System => Box::new(validate_system),
        // Credentials is a directory, not a JSON section — no validator needed.
        ConfigSection::Session => Box::new(validate_session),
        ConfigSection::Credentials => Box::new(|_| Ok(())),
        ConfigSection::Accounts => Box::new(validate_accounts),
    }
}

// ---------------------------------------------------------------------------
// Section validators
// ---------------------------------------------------------------------------

/// Validate the **models** config section.
///
/// - Top-level must be a JSON object.
/// - If a `models` key is present, it must be an array.
/// - Each provider ID (map key) must be non-empty.
/// - Each model ID must be non-empty.
/// - `baseUrl`, if present, must start with `http://` or `https://` (or be
///   empty/absent).
fn validate_models(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "models")?;
    if let Some(arr) = value.get("models") {
        ensure_array(arr, "models.models")?;
    }
    // Business validation: iterate providers and models
    if let Some(providers) = value.get("providers") {
        if let Some(obj) = providers.as_object() {
            for (provider_id, provider_val) in obj {
                if provider_id.is_empty() {
                    return Err("models provider ID cannot be empty".to_string());
                }
                validate_provider(provider_id, provider_val)?;
            }
        }
    }
    Ok(())
}

/// Validate a single provider entry within the models section.
fn validate_provider(provider_id: &str, provider: &serde_json::Value) -> Result<(), String> {
    if !provider.is_object() {
        return Err(format!(
            "models.providers.{} must be a JSON object",
            provider_id
        ));
    }
    // Validate base_url format if present
    if let Some(base_url) = provider.get("baseUrl") {
        if let Some(url) = base_url.as_str() {
            if !url.is_empty() && !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(format!(
                    "models.providers.{}.baseUrl must start with \
                     http:// or https://",
                    provider_id
                ));
            }
        }
    }
    // Validate credentialPath format and existence if present
    if let Some(cred_path) = provider.get("credentialPath") {
        if let Some(path_str) = cred_path.as_str() {
            if path_str.is_empty() {
                return Err(format!(
                    "models.providers.{}.credentialPath cannot be empty",
                    provider_id
                ));
            }
            if !std::path::Path::new(path_str).exists() {
                return Err(format!(
                    "models.providers.{}.credentialPath '{}' does not exist",
                    provider_id, path_str
                ));
            }
        } else if !cred_path.is_null() {
            return Err(format!(
                "models.providers.{}.credentialPath must be a string",
                provider_id
            ));
        }
    }
    // Validate each model entry
    if let Some(models) = provider.get("models") {
        if let Some(arr) = models.as_array() {
            for model in arr {
                validate_model(provider_id, model)?;
            }
        }
    }
    Ok(())
}

/// Validate a single model entry within a provider.
fn validate_model(provider_id: &str, model: &serde_json::Value) -> Result<(), String> {
    if !model.is_object() {
        return Err(format!(
            "models.providers.{}.models[] must be objects",
            provider_id
        ));
    }
    // Model ID is required and must be non-empty
    match model.get("id") {
        Some(serde_json::Value::String(id)) if id.is_empty() => {
            return Err(format!(
                "models.providers.{}.models[].id cannot be empty",
                provider_id
            ));
        }
        None => {
            return Err(format!(
                "models.providers.{}.models[].id is required",
                provider_id
            ));
        }
        _ => {}
    }
    Ok(())
}

/// Validate the **channels** config section.
///
/// - Top-level must be a JSON object.
/// - `channels` key, if present, must be a JSON object whose keys are
///   known channel types (non-empty, in the allowed list).
/// - `bindings` key, if present, must be a JSON array.  Each entry must
///   have non-empty `agentId`, `match.channel`, and `match.accountId`.
fn validate_channels(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "channels")?;

    // Validate channel type keys
    if let Some(channels) = value.get("channels") {
        if !channels.is_object() {
            return Err(format!(
                "channels.channels must be a JSON object, got {}",
                type_name(channels)
            ));
        }
        if let Some(obj) = channels.as_object() {
            for (channel_type, _) in obj {
                if channel_type.is_empty() {
                    return Err("channels type cannot be empty".to_string());
                }
                if !ALLOWED_CHANNEL_TYPES.contains(&channel_type.as_str()) {
                    return Err(format!(
                        "unknown channel type '{}'. Allowed: {}",
                        channel_type,
                        ALLOWED_CHANNEL_TYPES.join(", ")
                    ));
                }
            }
        }
    }

    // Collect defined channel type keys for binding reference validation
    let channel_types: std::collections::HashSet<String> = value
        .get("channels")
        .and_then(|c| c.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default();

    // Validate bindings
    if let Some(bindings) = value.get("bindings") {
        ensure_array(bindings, "channels.bindings")?;
        if let Some(arr) = bindings.as_array() {
            for (i, entry) in arr.iter().enumerate() {
                validate_binding_entry(i, entry, &channel_types)?;
            }
        }
    }

    Ok(())
}

/// Validate a single binding entry within the channels section.
fn validate_binding_entry(
    index: usize,
    entry: &serde_json::Value,
    channel_types: &std::collections::HashSet<String>,
) -> Result<(), String> {
    if !entry.is_object() {
        return Err(format!(
            "channels.bindings[{}] must be a JSON object",
            index
        ));
    }
    require_non_empty(
        entry,
        "agentId",
        &format!("channels.bindings[{}].agentId", index),
    )?;
    // match sub-object
    let match_obj = match entry.get("match") {
        Some(m) if m.is_object() => m,
        Some(_) => {
            return Err(format!(
                "channels.bindings[{}].match must be a JSON object",
                index
            ));
        }
        None => {
            return Err(format!("channels.bindings[{}].match is required", index));
        }
    };
    require_non_empty(
        match_obj,
        "channel",
        &format!("channels.bindings[{}].match.channel", index),
    )?;
    // Verify match.channel references a defined channel type
    let channel = match_obj
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !channel.is_empty() && !channel_types.contains(channel) {
        let defined = if channel_types.is_empty() {
            "none".to_string()
        } else {
            channel_types
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        };
        return Err(format!(
            "channels.bindings[{}].match.channel '{}' references an undefined \
             channel type. Defined types: {}",
            index, channel, defined
        ));
    }
    require_non_empty(
        match_obj,
        "accountId",
        &format!("channels.bindings[{}].match.accountId", index),
    )?;
    Ok(())
}

/// Validate the **gateway** config section.
///
/// - Top-level must be a JSON object.
/// - `port`, if present, must be a number in 1..=65535.
/// - `timeout`, if present, must be a non-negative number.
fn validate_gateway(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "gateway")?;
    if let Some(port) = value.get("port") {
        match port.as_u64() {
            Some(p) if p == 0 || p > 65535 => {
                return Err(format!(
                    "gateway.port must be in range 1-65535, got {} (port 0 is reserved by the OS)",
                    p
                ));
            }
            Some(_) => {}
            None => {
                return Err("gateway.port must be a non-negative integer".to_string());
            }
        }
    }
    if let Some(timeout) = value.get("timeout") {
        if !timeout.is_number() {
            return Err("gateway.timeout must be a number".to_string());
        }
        // Negative values: as_f64() returns Some for valid floats,
        // but negative numbers should be rejected.
        match timeout.as_f64() {
            Some(t) if t < 0.0 => {
                return Err("gateway.timeout must be non-negative".to_string());
            }
            Some(_) => {}
            None => {
                return Err("gateway.timeout must be a number".to_string());
            }
        }
    }
    Ok(())
}

/// Validate the **plugins** config section.
///
/// - Top-level must be a JSON object.
/// - Each plugin name in `entries` must be non-empty.
/// - Each plugin name in `allow` must be non-empty.
/// - For installed plugins (`installs`), the `installPath` must point to
///   an existing file or directory if present.
fn validate_plugins(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "plugins")?;

    // Validate entries: each plugin name must be non-empty
    if let Some(entries) = value.get("entries") {
        if let Some(obj) = entries.as_object() {
            for (name, _) in obj {
                if name.is_empty() {
                    return Err("plugins.entries plugin name cannot be empty".to_string());
                }
            }
        }
    }

    // Validate allow: each plugin name must be non-empty
    if let Some(allow) = value.get("allow") {
        if let Some(arr) = allow.as_array() {
            for (i, entry) in arr.iter().enumerate() {
                match entry {
                    serde_json::Value::String(s) if s.is_empty() => {
                        return Err(format!("plugins.allow[{}] plugin name cannot be empty", i));
                    }
                    serde_json::Value::String(_) => {}
                    _ => {
                        return Err(format!("plugins.allow[{}] must be a string", i));
                    }
                }
            }
        }
    }

    // Validate installs: installPath must exist if present
    if let Some(installs) = value.get("installs") {
        if let Some(obj) = installs.as_object() {
            for (name, info) in obj {
                if name.is_empty() {
                    return Err("plugins.installs plugin name cannot be empty".to_string());
                }
                validate_plugin_install(name, info)?;
            }
        }
    }

    Ok(())
}

/// Validate a single plugin install entry.
fn validate_plugin_install(name: &str, info: &serde_json::Value) -> Result<(), String> {
    if !info.is_object() {
        return Err(format!("plugins.installs.{} must be a JSON object", name));
    }
    // If installPath is present, verify the path exists
    if let Some(path_val) = info.get("installPath") {
        if let Some(path_str) = path_val.as_str() {
            if !path_str.is_empty() {
                let path = std::path::Path::new(path_str);
                if !path.exists() {
                    return Err(format!(
                        "plugins.installs.{}.installPath '{}' does not exist",
                        name, path_str
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Validate the **system** config section.
///
/// - Top-level must be a JSON object.
/// - `version`, if present, must be a non-empty string.
/// - `cron`, if present, must be a JSON object.
fn validate_system(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "system")?;

    // version field: if present, must be a non-empty string
    if let Some(version) = value.get("version") {
        match version {
            serde_json::Value::String(s) if s.is_empty() => {
                return Err("system.version cannot be an empty string".to_string());
            }
            serde_json::Value::String(_) => {}
            _ => {
                return Err("system.version must be a string".to_string());
            }
        }
    }

    // cron field: if present, must be a JSON object
    if let Some(cron) = value.get("cron") {
        if !cron.is_object() {
            return Err(format!(
                "system.cron must be a JSON object, got {}",
                type_name(cron)
            ));
        }
    }

    Ok(())
}

/// Validate the **session** config section.
///
/// - Top-level must be a JSON object.
/// - If `sweeperIntervalSecs` is present, it must be a positive number.
/// - If `idleMinutes` is present, it must be non-negative.
/// - If `purgeAfterMinutes` is present, it must be non-negative.
fn validate_session(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "session")?;
    if let Some(secs) = value.get("sweeperIntervalSecs") {
        if !secs.is_number() || secs.as_u64().unwrap_or(0) == 0 {
            return Err("session.sweeperIntervalSecs must be a positive number".to_string());
        }
    }
    validate_non_negative_field(value, "idleMinutes")?;
    validate_non_negative_field(value, "purgeAfterMinutes")?;
    Ok(())
}

/// Validate the **accounts** config section.
///
/// - Top-level must be a JSON object.
/// - Each account must have a non-empty `accountId`.
/// - Each account must have a non-empty `senderId`.
/// - All `accountId` values must be unique.
/// - Each `platform` must be one of the allowed channel types.
fn validate_accounts(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "accounts")?;

    let accounts = match value.get("accounts") {
        Some(arr) if arr.is_array() => arr.as_array().unwrap(),
        Some(_) => {
            return Err(format!(
                "accounts.accounts must be a JSON array, got {}",
                type_name(value.get("accounts").unwrap())
            ));
        }
        None => return Ok(()), // no accounts key is fine (empty list)
    };

    let mut seen_ids = std::collections::HashSet::new();

    for (i, entry) in accounts.iter().enumerate() {
        if !entry.is_object() {
            return Err(format!("accounts.accounts[{}] must be a JSON object", i));
        }

        require_non_empty(
            entry,
            "accountId",
            &format!("accounts.accounts[{}].accountId", i),
        )?;
        require_non_empty(
            entry,
            "senderId",
            &format!("accounts.accounts[{}].senderId", i),
        )?;

        check_account_id_unique(entry, i, &mut seen_ids)?;
        validate_account_platform(entry, i)?;
    }

    Ok(())
}

/// Check that `accountId` is unique across all accounts.
fn check_account_id_unique(
    entry: &serde_json::Value,
    index: usize,
    seen_ids: &mut std::collections::HashSet<String>,
) -> Result<(), String> {
    if let Some(id) = entry.get("accountId").and_then(|v| v.as_str()) {
        if !seen_ids.insert(id.to_string()) {
            return Err(format!(
                "duplicate accountId '{}' at accounts.accounts[{}]",
                id, index
            ));
        }
    }
    Ok(())
}

/// Validate that `platform` is a known channel type.
fn validate_account_platform(entry: &serde_json::Value, index: usize) -> Result<(), String> {
    if let Some(platform) = entry.get("platform").and_then(|v| v.as_str()) {
        if platform.is_empty() {
            return Err(format!(
                "accounts.accounts[{}].platform cannot be empty",
                index
            ));
        }
        if !ALLOWED_CHANNEL_TYPES.contains(&platform) {
            return Err(format!(
                "accounts.accounts[{}].platform '{}' is not a known \
                 channel type. Allowed: {}",
                index,
                platform,
                ALLOWED_CHANNEL_TYPES.join(", ")
            ));
        }
    }
    Ok(())
}

/// Validate that a numeric field, if present, is non-negative.
fn validate_non_negative_field(value: &serde_json::Value, field: &str) -> Result<(), String> {
    if let Some(v) = value.get(field) {
        if !v.is_number() {
            return Err(format!("session.{} must be a number", field));
        }
        if let Some(n) = v.as_f64() {
            if n < 0.0 {
                return Err(format!("session.{} must be non-negative", field));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Ensure a field in a JSON object is a non-empty string.
///
/// Returns `Err` if the field is absent or is an empty string.
/// `path` should be the full dotted path (e.g., `channels.bindings[0].match.channel`).
fn require_non_empty(obj: &serde_json::Value, field: &str, path: &str) -> Result<(), String> {
    match obj.get(field) {
        Some(serde_json::Value::String(s)) if s.is_empty() => {
            Err(format!("{} cannot be empty", path))
        }
        Some(serde_json::Value::String(_)) => Ok(()),
        None => Err(format!("{} is required", path)),
        _ => Err(format!("{} must be a string", path)),
    }
}

/// Ensure `value` is a JSON object; returns `Err` with a descriptive
/// message if not.
fn ensure_object(value: &serde_json::Value, section: &str) -> Result<(), String> {
    if !value.is_object() {
        return Err(format!(
            "{section} config must be a JSON object, got {}",
            type_name(value)
        ));
    }
    Ok(())
}

/// Ensure `value` is a JSON array; returns `Err` with a descriptive
/// message if not.
fn ensure_array(value: &serde_json::Value, path: &str) -> Result<(), String> {
    if !value.is_array() {
        return Err(format!(
            "{path} must be a JSON array, got {}",
            type_name(value)
        ));
    }
    Ok(())
}

/// Return a human-readable type label for a JSON value.
fn type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

// ---------------------------------------------------------------------------
// ConfigSection extension
// ---------------------------------------------------------------------------

impl ConfigSection {
    /// Return the default structural validator for this section.
    pub fn default_validator(&self) -> Box<SectionValidator> {
        for_section(*self)
    }
}

#[cfg(test)]
#[path = "validators_tests.rs"]
mod tests;
