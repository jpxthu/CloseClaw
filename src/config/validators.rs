//! Default validators for hot-reloaded config sections.
//!
//! Each validator performs lightweight structural checks on the parsed JSON
//! value — verifying the top-level type and presence of expected fields.
//! Deep business validation (e.g., model name existence) is intentionally
//! out of scope; those checks belong in the callers.

use super::manager::ConfigSection;
use super::manager_reload::SectionValidator;

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
        ConfigSection::Credentials => Box::new(|_| Ok(())),
    }
}

// ---------------------------------------------------------------------------
// Section validators
// ---------------------------------------------------------------------------

/// Validate the **models** config section.
///
/// - Top-level must be a JSON object.
/// - If a `models` key is present, it must be an array.
fn validate_models(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "models")?;
    if let Some(arr) = value.get("models") {
        ensure_array(arr, "models.models")?;
    }
    Ok(())
}

/// Validate the **channels** config section.
///
/// - Top-level must be a JSON object.
/// - If a `channels` key is present, it must be an array.
fn validate_channels(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "channels")?;
    if let Some(arr) = value.get("channels") {
        ensure_array(arr, "channels.channels")?;
    }
    Ok(())
}

/// Validate the **gateway** config section.
///
/// - Top-level must be a JSON object.
fn validate_gateway(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "gateway")
}

/// Validate the **plugins** config section.
///
/// - Top-level must be a JSON object.
/// - If a `plugins` key is present, it must be an array.
fn validate_plugins(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "plugins")?;
    if let Some(arr) = value.get("plugins") {
        ensure_array(arr, "plugins.plugins")?;
    }
    Ok(())
}

/// Validate the **system** config section.
///
/// - Top-level must be a JSON object.
fn validate_system(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "system")
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

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
