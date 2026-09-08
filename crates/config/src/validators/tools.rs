//! Validator for the **tools** config section.

use super::{ensure_object, type_name};

/// Validate the **tools** config section.
///
/// - Top-level must be a JSON object.
/// - `read.max_tokens`, if present, must be a positive integer.
pub fn validate_tools(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "tools")?;
    if let Some(read_obj) = value.get("read") {
        if let Some(max_tokens) = read_obj.get("max_tokens") {
            match max_tokens {
                serde_json::Value::Number(n) => {
                    if let Some(v) = n.as_u64() {
                        if v == 0 {
                            return Err(
                                "tools.read.max_tokens must be a positive integer".to_string()
                            );
                        }
                    } else {
                        return Err("tools.read.max_tokens must be a positive integer".to_string());
                    }
                }
                _ => {
                    return Err(format!(
                        "tools.read.max_tokens must be a number, got {}",
                        type_name(max_tokens)
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_tools;

    #[test]
    fn test_validate_tools_valid() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"read":{"max_tokens":10000}}"#).unwrap();
        assert!(validate_tools(&v).is_ok());
    }

    #[test]
    fn test_validate_tools_empty_object() {
        let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
        assert!(validate_tools(&v).is_ok());
    }

    #[test]
    fn test_validate_tools_not_object() {
        let v: serde_json::Value = serde_json::from_str(r#"[1]"#).unwrap();
        assert!(validate_tools(&v).is_err());
    }

    #[test]
    fn test_validate_tools_max_tokens_zero() {
        let v: serde_json::Value = serde_json::from_str(r#"{"read":{"max_tokens":0}}"#).unwrap();
        let err = validate_tools(&v).unwrap_err();
        assert!(err.contains("must be a positive integer"), "error: {}", err);
    }

    #[test]
    fn test_validate_tools_max_tokens_negative() {
        let v: serde_json::Value = serde_json::from_str(r#"{"read":{"max_tokens":-1}}"#).unwrap();
        let err = validate_tools(&v).unwrap_err();
        assert!(err.contains("must be a positive integer"), "error: {}", err);
    }

    #[test]
    fn test_validate_tools_max_tokens_string() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"read":{"max_tokens":"abc"}}"#).unwrap();
        let err = validate_tools(&v).unwrap_err();
        assert!(err.contains("must be a number"), "error: {}", err);
    }

    #[test]
    fn test_validate_tools_max_tokens_null() {
        let v: serde_json::Value = serde_json::from_str(r#"{"read":{"max_tokens":null}}"#).unwrap();
        let err = validate_tools(&v).unwrap_err();
        assert!(err.contains("must be a number"), "error: {}", err);
    }
}
