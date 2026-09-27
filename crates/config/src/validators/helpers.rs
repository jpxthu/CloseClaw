//! JSON-shape helpers shared by the section validators.

/// Ensure `value` is a JSON object; returns `Err` with a descriptive
/// message if not.
pub(crate) fn ensure_object(value: &serde_json::Value, section: &str) -> Result<(), String> {
    if !value.is_object() {
        return Err(format!(
            "{section} config must be a JSON object, got {}",
            type_name(value)
        ));
    }
    Ok(())
}

/// Return a human-readable type label for a JSON value.
pub(crate) fn type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Validate that an optional string field, if present, is non-empty.
pub(crate) fn validate_optional_non_empty_string(
    value: &serde_json::Value,
    field: &str,
    path: &str,
) -> Result<(), String> {
    if let Some(v) = value.get(field) {
        match v {
            serde_json::Value::String(s) if s.is_empty() => Err(format!("{path} cannot be empty")),
            serde_json::Value::String(_) => Ok(()),
            serde_json::Value::Null => Ok(()),
            _ => Err(format!("{path} must be a string")),
        }
    } else {
        Ok(())
    }
}

/// Validate that a numeric field, if present, is non-negative.
/// Rejects NaN and Infinity values.
pub(crate) fn validate_non_negative_field(
    value: &serde_json::Value,
    field: &str,
) -> Result<(), String> {
    if let Some(v) = value.get(field) {
        if !v.is_number() {
            return Err(format!("session.{} must be a number", field));
        }
        let n = v.as_f64().unwrap_or(0.0);
        if !n.is_finite() || n < 0.0 {
            return Err(format!("session.{} must be non-negative", field));
        }
    }
    Ok(())
}

/// Ensure a field in a JSON object is a non-empty string.
///
/// Returns `Err` if the field is absent or is an empty string.
/// `path` should be the full dotted path (e.g., `channels.bindings[0].match.channel`).
pub(crate) fn require_non_empty(
    obj: &serde_json::Value,
    field: &str,
    path: &str,
) -> Result<(), String> {
    match obj.get(field) {
        Some(serde_json::Value::String(s)) if s.is_empty() => {
            Err(format!("{} cannot be empty", path))
        }
        Some(serde_json::Value::String(_)) => Ok(()),
        None => Err(format!("{} is required", path)),
        _ => Err(format!("{} must be a string", path)),
    }
}

/// Ensure `value` is a JSON array; returns `Err` with a descriptive
/// message if not.
pub(crate) fn ensure_array(value: &serde_json::Value, path: &str) -> Result<(), String> {
    if !value.is_array() {
        return Err(format!(
            "{path} must be a JSON array, got {}",
            type_name(value)
        ));
    }
    Ok(())
}
