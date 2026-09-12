//! Memory config section validator.
//!
//! Validates per-subsystem fields per design doc `docs/design/memory/config.md`:
//! - Each subsystem's `enabled` must be a boolean (if present).
//! - Numeric parameters must be non-negative.
//! - Model/path fields must be valid strings.

use super::{ensure_object, type_name};

/// Validate the **memory** config section.
///
/// - Top-level must be a JSON object.
/// - Each subsystem (`mining`, `dreaming`, `search`) `enabled` field,
///   if present, must be a boolean.
/// - Numeric parameters across all subsystems must be non-negative.
/// - Model and path fields, if present, must be non-empty strings.
pub fn validate_memory(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "memory")?;

    // Validate storage paths
    if let Some(storage) = value.get("storage") {
        if !storage.is_object() {
            return Err(format!(
                "memory.storage must be a JSON object, got {}",
                type_name(storage)
            ));
        }
        validate_optional_non_empty_string(storage, "dbPath", "memory.storage.dbPath")?;
        validate_optional_non_empty_string(storage, "memoryMdPath", "memory.storage.memoryMdPath")?;
    }

    // Validate mining subsystem
    if let Some(mining) = value.get("mining") {
        validate_mining(mining)?;
    }

    // Validate dreaming subsystem
    if let Some(dreaming) = value.get("dreaming") {
        validate_dreaming(dreaming)?;
    }

    // Validate search subsystem
    if let Some(search) = value.get("search") {
        validate_search(search)?;
    }

    // Validate forgetting subsystem
    if let Some(forgetting) = value.get("forgetting") {
        validate_forgetting(forgetting)?;
    }

    Ok(())
}

/// Validate the `mining` subsystem config.
fn validate_mining(value: &serde_json::Value) -> Result<(), String> {
    if !value.is_object() {
        return Err(format!(
            "memory.mining must be a JSON object, got {}",
            type_name(value)
        ));
    }
    validate_bool_field(value, "enabled", "memory.mining.enabled")?;
    validate_optional_non_empty_string(value, "model", "memory.mining.model")?;
    validate_non_negative_i32(
        value,
        "maxEventsPerSession",
        "memory.mining.maxEventsPerSession",
    )?;
    validate_non_negative_i32(value, "dedupWindowDays", "memory.mining.dedupWindowDays")?;

    // transcript_clean_rules
    if let Some(rules) = value.get("transcriptCleanRules") {
        if !rules.is_object() {
            return Err(format!(
                "memory.mining.transcriptCleanRules must be a JSON object, got {}",
                type_name(rules)
            ));
        }
        validate_non_negative_i32(
            rules,
            "minTurns",
            "memory.mining.transcriptCleanRules.minTurns",
        )?;
        validate_non_negative_i32(
            rules,
            "minOwnerMsgs",
            "memory.mining.transcriptCleanRules.minOwnerMsgs",
        )?;
        validate_optional_non_empty_string(
            rules,
            "format",
            "memory.mining.transcriptCleanRules.format",
        )?;
    }

    Ok(())
}

/// Validate the `dreaming` subsystem config.
fn validate_dreaming(value: &serde_json::Value) -> Result<(), String> {
    if !value.is_object() {
        return Err(format!(
            "memory.dreaming must be a JSON object, got {}",
            type_name(value)
        ));
    }
    validate_bool_field(value, "enabled", "memory.dreaming.enabled")?;
    validate_optional_non_empty_string(value, "model", "memory.dreaming.model")?;
    validate_optional_non_empty_string(value, "schedule", "memory.dreaming.schedule")?;

    // scoring weights (can be negative — negative_signal_weight defaults to -0.5)
    if let Some(scoring) = value.get("scoring") {
        if !scoring.is_object() {
            return Err(format!(
                "memory.dreaming.scoring must be a JSON object, got {}",
                type_name(scoring)
            ));
        }
        validate_number_field(
            scoring,
            "frequencyWeight",
            "memory.dreaming.scoring.frequencyWeight",
        )?;
        validate_number_field(
            scoring,
            "recencyWeight",
            "memory.dreaming.scoring.recencyWeight",
        )?;
        validate_number_field(
            scoring,
            "explicitnessWeight",
            "memory.dreaming.scoring.explicitnessWeight",
        )?;
        validate_number_field(
            scoring,
            "crossAgentWeight",
            "memory.dreaming.scoring.crossAgentWeight",
        )?;
        validate_number_field(
            scoring,
            "negativeSignalWeight",
            "memory.dreaming.scoring.negativeSignalWeight",
        )?;
    }

    // threshold (non-negative)
    if let Some(threshold) = value.get("threshold") {
        if !threshold.is_object() {
            return Err(format!(
                "memory.dreaming.threshold must be a JSON object, got {}",
                type_name(threshold)
            ));
        }
        validate_non_negative_number(threshold, "absolute", "memory.dreaming.threshold.absolute")?;
        validate_non_negative_number(threshold, "relative", "memory.dreaming.threshold.relative")?;
    }

    // capacity
    if let Some(capacity) = value.get("capacity") {
        if !capacity.is_object() {
            return Err(format!(
                "memory.dreaming.capacity must be a JSON object, got {}",
                type_name(capacity)
            ));
        }
        validate_non_negative_usize(capacity, "maxRules", "memory.dreaming.capacity.maxRules")?;
    }

    // diary
    if let Some(diary) = value.get("diary") {
        if !diary.is_object() {
            return Err(format!(
                "memory.dreaming.diary must be a JSON object, got {}",
                type_name(diary)
            ));
        }
        validate_bool_field(diary, "enabled", "memory.dreaming.diary.enabled")?;
        validate_optional_non_empty_string(diary, "path", "memory.dreaming.diary.path")?;
    }

    Ok(())
}

/// Validate the `search` subsystem config.
fn validate_search(value: &serde_json::Value) -> Result<(), String> {
    if !value.is_object() {
        return Err(format!(
            "memory.search must be a JSON object, got {}",
            type_name(value)
        ));
    }
    validate_bool_field(value, "enabled", "memory.search.enabled")?;
    validate_optional_non_empty_string(value, "model", "memory.search.model")?;
    validate_non_negative_usize(value, "contextTurns", "memory.search.contextTurns")?;
    validate_non_negative_u64(value, "timeoutMs", "memory.search.timeoutMs")?;
    validate_non_negative_usize(value, "maxSummaryChars", "memory.search.maxSummaryChars")?;
    validate_non_negative_u32(value, "minEntityHits", "memory.search.minEntityHits")?;
    validate_non_negative_usize(value, "topKEvents", "memory.search.topKEvents")?;
    Ok(())
}

/// Validate the `forgetting` subsystem config.
fn validate_forgetting(value: &serde_json::Value) -> Result<(), String> {
    if !value.is_object() {
        return Err(format!(
            "memory.forgetting must be a JSON object, got {}",
            type_name(value)
        ));
    }
    validate_non_negative_i64(value, "initialTtlDays", "memory.forgetting.initialTtlDays")?;
    validate_non_negative_i64(
        value,
        "reidentifyExtensionDays",
        "memory.forgetting.reidentifyExtensionDays",
    )?;
    validate_non_negative_i64(
        value,
        "injectionExtensionDays",
        "memory.forgetting.injectionExtensionDays",
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Validate that a field, if present, is a boolean.
fn validate_bool_field(value: &serde_json::Value, field: &str, path: &str) -> Result<(), String> {
    if let Some(v) = value.get(field) {
        if !v.is_boolean() {
            return Err(format!("{} must be a boolean, got {}", path, type_name(v)));
        }
    }
    Ok(())
}

/// Validate that a field, if present, is a number.
fn validate_number_field(value: &serde_json::Value, field: &str, path: &str) -> Result<(), String> {
    if let Some(v) = value.get(field) {
        if !v.is_number() {
            return Err(format!("{} must be a number, got {}", path, type_name(v)));
        }
    }
    Ok(())
}

/// Validate that a JSON number value is non-negative.
/// Works for all integer and floating-point subtypes.
/// Rejects NaN and Infinity.
fn validate_non_negative_number_value(v: &serde_json::Value, path: &str) -> Result<(), String> {
    let n = v.as_f64().unwrap_or(0.0);
    if !n.is_finite() || n < 0.0 {
        return Err(format!("{} must be non-negative", path));
    }
    Ok(())
}

/// Validate that a field, if present, is a non-negative number
/// (any integer or floating-point subtype).
fn validate_non_negative_number(
    value: &serde_json::Value,
    field: &str,
    path: &str,
) -> Result<(), String> {
    if let Some(v) = value.get(field) {
        if !v.is_number() {
            return Err(format!("{} must be a number, got {}", path, type_name(v)));
        }
        validate_non_negative_number_value(v, path)?;
    }
    Ok(())
}

/// Validate that a field, if present, is a non-negative i32.
fn validate_non_negative_i32(
    value: &serde_json::Value,
    field: &str,
    path: &str,
) -> Result<(), String> {
    validate_non_negative_number(value, field, path)
}

/// Validate that a field, if present, is a non-negative i64.
fn validate_non_negative_i64(
    value: &serde_json::Value,
    field: &str,
    path: &str,
) -> Result<(), String> {
    validate_non_negative_number(value, field, path)
}

/// Validate that a field, if present, is a non-negative u64.
fn validate_non_negative_u64(
    value: &serde_json::Value,
    field: &str,
    path: &str,
) -> Result<(), String> {
    validate_non_negative_number(value, field, path)
}

/// Validate that a field, if present, is a non-negative u32.
fn validate_non_negative_u32(
    value: &serde_json::Value,
    field: &str,
    path: &str,
) -> Result<(), String> {
    validate_non_negative_number(value, field, path)
}

/// Validate that a field, if present, is a non-negative usize.
fn validate_non_negative_usize(
    value: &serde_json::Value,
    field: &str,
    path: &str,
) -> Result<(), String> {
    validate_non_negative_number(value, field, path)
}

/// Validate that an optional string field, if present, is non-empty.
fn validate_optional_non_empty_string(
    value: &serde_json::Value,
    field: &str,
    path: &str,
) -> Result<(), String> {
    if let Some(v) = value.get(field) {
        match v {
            serde_json::Value::String(s) if s.is_empty() => {
                Err(format!("{} cannot be empty", path))
            }
            serde_json::Value::String(_) => Ok(()),
            serde_json::Value::Null => Ok(()),
            _ => Err(format!("{} must be a string, got {}", path, type_name(v))),
        }
    } else {
        Ok(())
    }
}
