//! JSON-shape helpers shared by the section validators.

use std::collections::HashSet;

use super::helpers::{
    ensure_array, ensure_object, require_non_empty, type_name, validate_optional_non_empty_string,
};

/// Validate a single **credentials** file.
pub fn validate_credentials(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "credentials")?;
    require_non_empty(value, "provider", "credentials.provider")?;
    validate_optional_non_empty_string(value, "apiKey", "credentials.apiKey")?;
    validate_optional_non_empty_string(value, "profile", "credentials.profile")?;
    Ok(())
}

/// Validate the **gateway** config section.
///
/// - Top-level must be a JSON object.
/// - `port`, if present, must be a number in 1..=65535.
/// - `timeout`, if present, must be a non-negative number.
/// - `inboundQueueCapacity`, if present, must be a positive integer (> 0).
pub fn validate_gateway(value: &serde_json::Value) -> Result<(), String> {
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
    if let Some(cap) = value.get("inboundQueueCapacity") {
        if cap.as_u64() == Some(0) {
            return Err("gateway.inboundQueueCapacity must be greater than 0".to_string());
        }
        if !cap.is_number() || cap.as_u64().is_none() {
            return Err("gateway.inboundQueueCapacity must be a positive integer".to_string());
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
pub fn validate_plugins(value: &serde_json::Value) -> Result<(), String> {
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
/// - `cron.schedule`, if present, must be a valid cron expression.
pub fn validate_system(value: &serde_json::Value) -> Result<(), String> {
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
    if let Some(cron_obj) = value.get("cron") {
        if !cron_obj.is_object() {
            return Err(format!(
                "system.cron must be a JSON object, got {}",
                type_name(cron_obj)
            ));
        }
        // cron.schedule: if present, must be a valid cron expression
        if let Some(schedule) = cron_obj.get("schedule") {
            if let Some(expr) = schedule.as_str() {
                if !expr.is_empty() {
                    use cron::Schedule;
                    use std::str::FromStr;
                    if Schedule::from_str(expr).is_err() {
                        return Err(
                            "system.cron.schedule must be a valid cron expression".to_string()
                        );
                    }
                }
            } else if !schedule.is_null() {
                return Err("system.cron.schedule must be a string".to_string());
            }
        }
    }

    Ok(())
}

/// Validate the **agents** config section.
///
/// - Top-level must be a JSON object.
/// - `agents` field, if present, must be a JSON array.
/// - Each agent ID must be non-empty.
/// - No duplicate agent IDs.
pub fn validate_agents(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "agents")?;
    if let Some(arr) = value.get("agents").and_then(|a| a.as_array()) {
        let mut seen = HashSet::new();
        for (i, entry) in arr.iter().enumerate() {
            let s = match entry {
                serde_json::Value::String(s) if !s.is_empty() => s,
                serde_json::Value::String(_) => {
                    return Err(format!("agents.agents[{}] cannot be empty", i));
                }
                _ => {
                    return Err(format!(
                        "agents.agents[{}] must be a string, got {}",
                        i,
                        type_name(entry)
                    ));
                }
            };
            if !seen.insert(s.clone()) {
                return Err(format!(
                    "duplicate agent ID '{}' at agents.agents[{}]",
                    s, i
                ));
            }
        }
    }
    Ok(())
}

/// Validate the **media** config section.
///
/// - Top-level must be a JSON object.
/// - Detailed field validation (storageDir empty/null-byte) is performed by
///   `MediaConfigData::validate()` in the provider; this structural validator
///   only enforces the top-level shape to avoid duplicate rules.
pub fn validate_media(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "media")?;
    Ok(())
}

/// Validate the **skills** config section.
///
/// - Top-level must be a JSON object.
/// - `extraDirs`, if present, must be a JSON array of strings.
/// - Each path must be non-empty and must not contain null bytes.
/// - Empty/absent `extraDirs` is valid (uses defaults).
pub fn validate_skills(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "skills")?;
    if let Some(extra_dirs) = value.get("extraDirs") {
        ensure_array(extra_dirs, "skills.extraDirs")?;
        if let Some(arr) = extra_dirs.as_array() {
            for (i, item) in arr.iter().enumerate() {
                match item {
                    serde_json::Value::String(s) => {
                        if s.is_empty() {
                            return Err(format!("skills.extraDirs[{}] cannot be an empty path", i));
                        }
                        if s.contains('\0') {
                            return Err(format!("skills.extraDirs[{}] contains a null byte", i));
                        }
                    }
                    _ => {
                        return Err(format!(
                            "skills.extraDirs[{}] must be a string, got {}",
                            i,
                            type_name(item)
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}
