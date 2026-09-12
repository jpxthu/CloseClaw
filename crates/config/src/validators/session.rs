//! Session config section validator.
//!
//! Validates per-role idleMinutes / purgeAfterMinutes at the correct
//! nested level (defaults→role, agents→id→role) per design doc.

use closeclaw_common::compaction::CompactConfig;

use super::{ensure_object, validate_non_negative_field};

/// Validate the **session** config section.
///
/// - Top-level must be a JSON object.
/// - If `sweeperIntervalSeconds` is present, it must be a positive number.
/// - Per-role session config (in `defaults` and `agents`) must have
///   `idleMinutes` non-negative and `purgeAfterMinutes` non-negative.
/// - If `planArchiveDays` is present, it must be non-negative.
/// - If `auditLogLimit` is present, it must be non-negative.
/// - If `compact` is present and non-null, it must deserialize to a valid
///   `CompactConfig` (positive `chars_per_token`, thresholds in [0,1],
///   `auto_compact_threshold_pct` < `warning_threshold_pct`).
pub fn validate_session(value: &serde_json::Value) -> Result<(), String> {
    ensure_object(value, "session")?;
    if let Some(secs) = value.get("sweeperIntervalSeconds") {
        if !secs.is_number() || secs.as_u64().unwrap_or(0) == 0 {
            return Err("session.sweeperIntervalSeconds must be a positive number".to_string());
        }
    }
    // Validate per-role idleMinutes / purgeAfterMinutes in defaults
    if let Some(defaults) = value.get("defaults") {
        if defaults.is_null() {
            return Err("session.defaults must be a JSON object, got null".to_string());
        }
        if let Some(obj) = defaults.as_object() {
            for (role, role_config) in obj {
                let path = format!("session.defaults.{}", role);
                validate_per_agent_session_role(role_config, &path)?;
            }
        } else {
            return Err(format!(
                "session.defaults must be a JSON object, got {}",
                super::type_name(defaults)
            ));
        }
    }
    // Validate per-role idleMinutes / purgeAfterMinutes in agents overrides
    if let Some(agents) = value.get("agents") {
        if agents.is_null() {
            return Err("session.agents must be a JSON object, got null".to_string());
        }
        if let Some(agents_obj) = agents.as_object() {
            for (agent_id, agent_roles) in agents_obj {
                if let Some(roles_obj) = agent_roles.as_object() {
                    for (role, role_config) in roles_obj {
                        let path = format!("session.agents.{}.{}", agent_id, role);
                        validate_per_agent_session_role(role_config, &path)?;
                    }
                }
            }
        } else {
            return Err(format!(
                "session.agents must be a JSON object, got {}",
                super::type_name(agents)
            ));
        }
    }
    validate_non_negative_field(value, "planArchiveDays")?;
    validate_non_negative_field(value, "auditLogLimit")?;
    if let Some(compact) = value.get("compact") {
        if !compact.is_null() {
            let config: CompactConfig = serde_json::from_value(compact.clone())
                .map_err(|e| format!("session.compact: invalid config: {}", e))?;
            config
                .validate()
                .map_err(|e| format!("session.compact: {}", e))?;
        }
    }
    Ok(())
}

/// Validate a per-agent session role config entry.
/// `path` is the dotted path prefix for error messages.
fn validate_per_agent_session_role(config: &serde_json::Value, path: &str) -> Result<(), String> {
    if !config.is_object() {
        return Err(format!("{} must be a JSON object", path));
    }
    if let Some(idle) = config.get("idleMinutes") {
        if !idle.is_number() {
            return Err(format!("{}.idleMinutes must be a number", path));
        }
        let n = idle.as_f64().unwrap_or(0.0);
        if !n.is_finite() || n < 0.0 {
            return Err(format!("{}.idleMinutes must be non-negative", path));
        }
    }
    if let Some(purge) = config.get("purgeAfterMinutes") {
        if !purge.is_number() {
            return Err(format!("{}.purgeAfterMinutes must be a number", path));
        }
        let n = purge.as_f64().unwrap_or(0.0);
        if !n.is_finite() || n < 0.0 {
            return Err(format!("{}.purgeAfterMinutes must be non-negative", path));
        }
    }
    Ok(())
}
