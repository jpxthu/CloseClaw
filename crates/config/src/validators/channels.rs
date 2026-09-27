//! Channels config section validator.

use std::collections::HashSet;

use crate::providers::channels::ALLOWED_CHANNEL_TYPES;

use super::cross_ref::CrossRefData;
use super::helpers::{ensure_array, ensure_object, require_non_empty, type_name};

/// Validate the **channels** config section.
///
/// - Top-level must be a JSON object.
/// - `channels` key, if present, must be a JSON object whose keys are
///   known channel types (non-empty, in the allowed list).
/// - `bindings` key, if present, must be a JSON array.  Each entry must
///   have non-empty `agentId`, `match.channel`, and `match.accountId`.
pub fn validate_channels(value: &serde_json::Value) -> Result<(), String> {
    validate_channels_with_refs(value, None)
}

/// Validate the **channels** config section with optional cross-reference data.
///
/// When `cross_ref` is provided, binding entries are additionally
/// validated against the registered agent and account ID sets.
pub fn validate_channels_with_refs(
    value: &serde_json::Value,
    cross_ref: Option<&CrossRefData>,
) -> Result<(), String> {
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
    let channel_types: HashSet<String> = value
        .get("channels")
        .and_then(|c| c.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default();

    // Validate bindings
    if let Some(bindings) = value.get("bindings") {
        ensure_array(bindings, "channels.bindings")?;
        if let Some(arr) = bindings.as_array() {
            for (i, entry) in arr.iter().enumerate() {
                validate_binding_entry(i, entry, &channel_types, cross_ref)?;
            }
        }
    }

    Ok(())
}

/// Validate a single binding entry within the channels section.
fn validate_binding_entry(
    index: usize,
    entry: &serde_json::Value,
    channel_types: &HashSet<String>,
    cross_ref: Option<&CrossRefData>,
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

    // Cross-reference validation: verify agentId and accountId exist
    if let Some(cr) = cross_ref {
        let agent_id = entry.get("agentId").and_then(|v| v.as_str()).unwrap_or("");
        if !agent_id.is_empty() && !cr.agent_ids.is_empty() && !cr.agent_ids.contains(agent_id) {
            let known: Vec<&str> = cr.agent_ids.iter().map(String::as_str).collect();
            return Err(format!(
                "channels.bindings[{}].agentId '{}' references an unknown agent. \
                 Known agents: {}",
                index,
                agent_id,
                known.join(", ")
            ));
        }
        let account_id = match_obj
            .get("accountId")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !account_id.is_empty()
            && !cr.account_ids.is_empty()
            && !cr.account_ids.contains(account_id)
        {
            let known: Vec<&str> = cr.account_ids.iter().map(String::as_str).collect();
            return Err(format!(
                "channels.bindings[{}].match.accountId '{}' references an unknown \
                 account. Known accounts: {}",
                index,
                account_id,
                known.join(", ")
            ));
        }
    }

    Ok(())
}
