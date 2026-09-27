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
