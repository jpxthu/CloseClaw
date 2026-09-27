//! Accounts config section validator.

use crate::providers::channels::ALLOWED_CHANNEL_TYPES;

use super::helpers::{ensure_object, require_non_empty, type_name};

/// Validate the **accounts** config section.
///
/// - Top-level must be a JSON object.
/// - Each account must have a non-empty `accountId`.
/// - Each account must have a non-empty `senderId`.
/// - All `accountId` values must be unique.
/// - Each `platform` must be one of the allowed channel types.
/// - If `channels_config` is provided, each account's `platform` must
///   correspond to a channel configured in `channels.json`.
pub fn validate_accounts(
    value: &serde_json::Value,
    channels_config: Option<&serde_json::Value>,
) -> Result<(), String> {
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

    // Collect configured channel type keys for cross-reference validation.
    let configured_channels: std::collections::HashSet<String> = channels_config
        .and_then(|c| c.get("channels"))
        .and_then(|c| c.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default();
    let has_channels_config = channels_config.is_some();

    let mut seen_ids = std::collections::HashSet::new();
    let mut seen_bindings: std::collections::HashSet<(String, String, String)> =
        std::collections::HashSet::new();

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
        check_platform_binding_unique(entry, i, &mut seen_bindings)?;

        // Cross-reference: account.platform must have a matching channel config.
        if has_channels_config {
            validate_account_channel_reference(entry, i, &configured_channels)?;
        }
    }

    // Validate bindings array
    validate_account_bindings(value)?;

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

/// Check that `(bot_app_id, sender_id)` is unique within the same platform.
///
/// Within the same platform, the combination of bot application and sender
/// must be unique (design doc: accounts 校验规则).
fn check_platform_binding_unique(
    entry: &serde_json::Value,
    index: usize,
    seen_bindings: &mut std::collections::HashSet<(String, String, String)>,
) -> Result<(), String> {
    let platform = entry
        .get("platform")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let bot_app_id = entry
        .get("botAppId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let sender_id = entry
        .get("senderId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let key = (platform.clone(), bot_app_id.clone(), sender_id.clone());
    if !seen_bindings.insert(key) {
        return Err(format!(
            "accounts.accounts[{}]. duplicate binding: \
             platform='{}', bot_app_id='{}', \
             sender_id='{}'",
            index, platform, bot_app_id, sender_id
        ));
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

/// Validate that an account's `platform` has a corresponding channel
/// configured in `channels.json`.
///
/// `configured_channels` is the set of keys from the `channels` object
/// in `channels.json`.
fn validate_account_channel_reference(
    entry: &serde_json::Value,
    index: usize,
    configured_channels: &std::collections::HashSet<String>,
) -> Result<(), String> {
    if let Some(platform) = entry.get("platform").and_then(|v| v.as_str()) {
        if !configured_channels.is_empty() && !configured_channels.contains(platform) {
            let defined: Vec<&str> = configured_channels.iter().map(String::as_str).collect();
            return Err(format!(
                "accounts.accounts[{}].platform '{}' does not correspond \
                 to any configured channel. Configured channels: {}",
                index,
                platform,
                defined.join(", ")
            ));
        }
    }
    Ok(())
}

/// Validate the `bindings` array in accounts config.
///
/// - `bindings`, if present, must be a JSON array.
/// - Each binding must have non-empty `bot_app_id` and `agent_id`.
/// - Each `bot_app_id` must map to exactly one `agent_id` (same
///   `bot_app_id` with different `agent_id` is rejected; same
///   `bot_app_id` with the same `agent_id` appearing multiple times
///   is allowed).
fn validate_account_bindings(value: &serde_json::Value) -> Result<(), String> {
    let bindings = match value.get("bindings") {
        Some(arr) if arr.is_array() => arr.as_array().unwrap(),
        Some(_) => {
            return Err(format!(
                "accounts.bindings must be a JSON array, got {}",
                type_name(value.get("bindings").unwrap())
            ));
        }
        None => return Ok(()),
    };
    // bot_app_id → (agent_id, first occurrence index)
    let mut bot_agent_map: std::collections::HashMap<String, (String, usize)> =
        std::collections::HashMap::new();
    for (i, entry) in bindings.iter().enumerate() {
        if !entry.is_object() {
            return Err(format!("accounts.bindings[{}] must be a JSON object", i));
        }
        require_non_empty(
            entry,
            "bot_app_id",
            &format!("accounts.bindings[{}].bot_app_id", i),
        )?;
        require_non_empty(
            entry,
            "agent_id",
            &format!("accounts.bindings[{}].agent_id", i),
        )?;
        let bot_id = entry
            .get("bot_app_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let agent_id = entry.get("agent_id").and_then(|v| v.as_str()).unwrap_or("");
        if let Some((existing_agent, first_idx)) = bot_agent_map.get(bot_id) {
            if existing_agent != agent_id {
                return Err(format!(
                    "accounts.bindings[{}].bot_app_id '{}' is bound to \
                     agent_id '{}' but already bound to '{}' at \
                     accounts.bindings[{}]; each bot_app_id must map \
                     to exactly one agent_id",
                    i, bot_id, agent_id, existing_agent, first_idx
                ));
            }
            // Same bot_app_id + same agent_id -> allowed (not a conflict)
        } else {
            bot_agent_map.insert(bot_id.to_string(), (agent_id.to_string(), i));
        }
    }
    Ok(())
}
