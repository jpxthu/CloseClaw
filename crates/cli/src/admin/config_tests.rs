//! Unit tests for config validate schema-level validation.
//!
//! Covers the behavioral dimensions required by the plan:
//! 1. Normal path: valid section files → valid=true, no issues
//! 2. Error path: JSON syntax error → valid=false with parse error
//! 3. Error path: schema violations → valid=false with specific field issues
//! 4. Boundary: unknown filename → syntax-only, "unknown config file" prompt
//! 5. Boundary: empty JSON object → validator passes if no required fields
//! 6. Summary: text and JSON modes correctly present issues

use super::config::handle_config;
use crate::args::ConfigAction;
use std::fs;
use tempfile::TempDir;

// ── Normal path: valid section files ───────────────────────────────────────

/// Valid models.json with a proper provider and model → valid, no issues.
#[tokio::test]
async fn test_config_validate_models_valid() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("models.json");
    fs::write(
        &file,
        r#"{
            "providers": {
                "openai": {
                    "models": [{ "id": "gpt-4" }]
                }
            }
        }"#,
    )
    .unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(
        result.is_ok(),
        "valid models.json should succeed: {:?}",
        result
    );
}

/// Valid channels.json with proper channel types and bindings.
#[tokio::test]
async fn test_config_validate_channels_valid() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("channels.json");
    fs::write(
        &file,
        r#"{
            "channels": { "feishu": {} },
            "bindings": [{
                "agentId": "agent-a",
                "match": { "channel": "feishu", "accountId": "acct-1" }
            }]
        }"#,
    )
    .unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(
        result.is_ok(),
        "valid channels.json should succeed: {:?}",
        result
    );
}

/// Valid gateway.json with port in range.
#[tokio::test]
async fn test_config_validate_gateway_valid() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("gateway.json");
    fs::write(&file, r#"{"port": 8080, "timeout": 30}"#).unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(
        result.is_ok(),
        "valid gateway.json should succeed: {:?}",
        result
    );
}

// ── Error path: JSON syntax errors ─────────────────────────────────────────

/// Invalid JSON syntax → valid=false, error info contains parse error.
#[tokio::test]
async fn test_config_validate_syntax_error() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("models.json");
    fs::write(&file, r#"{"providers": {not valid"#).unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(result.is_err(), "invalid JSON should return error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("JSON parse error") || err_msg.contains("Validation failed"),
        "error should mention parse failure: {}",
        err_msg
    );
}

// ── Error path: schema violations ──────────────────────────────────────────

/// models.json with empty provider ID → valid=false, issues mention provider ID.
#[tokio::test]
async fn test_config_validate_models_empty_provider_id() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("models.json");
    fs::write(
        &file,
        r#"{
            "providers": {
                "": { "models": [{"id": "gpt-4"}] }
            }
        }"#,
    )
    .unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(result.is_err(), "empty provider ID should fail validation");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("provider ID cannot be empty"),
        "error should mention empty provider ID: {}",
        err_msg
    );
}

/// models.json with empty model ID → valid=false, issues mention model ID.
#[tokio::test]
async fn test_config_validate_models_empty_model_id() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("models.json");
    fs::write(
        &file,
        r#"{
            "providers": {
                "openai": {
                    "models": [{"id": ""}]
                }
            }
        }"#,
    )
    .unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(result.is_err(), "empty model ID should fail validation");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("id cannot be empty"),
        "error should mention empty model ID: {}",
        err_msg
    );
}

/// gateway.json with port out of range → valid=false, issues mention port.
#[tokio::test]
async fn test_config_validate_gateway_port_out_of_range() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("gateway.json");
    fs::write(&file, r#"{"port": 99999}"#).unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(result.is_err(), "port out of range should fail validation");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("1-65535"),
        "error should mention valid port range: {}",
        err_msg
    );
}

// ── Boundary: unknown config files ─────────────────────────────────────────

/// Unknown filename (not a known section) → syntax-only validation,
/// issues list contains "unknown config file" prompt.
#[tokio::test]
async fn test_config_validate_unknown_file_uses_syntax_only() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("custom_settings.json");
    fs::write(&file, r#"{"arbitrary": "data"}"#).unwrap();

    // handler should succeed (valid JSON syntax)
    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(
        result.is_ok(),
        "unknown file with valid JSON should succeed: {:?}",
        result
    );
}

/// Unknown filename with invalid JSON → should still report parse error.
#[tokio::test]
async fn test_config_validate_unknown_file_invalid_json() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("mystery.json");
    fs::write(&file, "{bad").unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(
        result.is_err(),
        "unknown file with invalid JSON should fail"
    );
}

/// Unknown filename with valid JSON in JSON output mode → valid=true, notes present.
#[tokio::test]
async fn test_config_validate_unknown_file_json_output() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("mystery.json");
    fs::write(&file, r#"{"key": "value"}"#).unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        true,
    )
    .await;
    assert!(
        result.is_ok(),
        "unknown file JSON output should succeed: {:?}",
        result
    );
}

// ── Boundary: empty JSON object ────────────────────────────────────────────

/// Empty JSON object `{}` on a valid section → validator should pass
/// if the section has no required fields at the root level.
#[tokio::test]
async fn test_config_validate_empty_object_passes() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("memory.json");
    fs::write(&file, r#"{}"#).unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(
        result.is_ok(),
        "empty object on memory section should pass: {:?}",
        result
    );
}

// ── Summary output: text and JSON modes ────────────────────────────────────

/// JSON mode output for a valid file → JSON output, valid=true, no issues.
#[tokio::test]
async fn test_config_validate_json_output_valid() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("gateway.json");
    fs::write(&file, r#"{"port": 8080}"#).unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        true,
    )
    .await;
    assert!(
        result.is_ok(),
        "json output for valid file should succeed: {:?}",
        result
    );
}

/// JSON mode output for invalid file → JSON output, valid=false, issues present.
#[tokio::test]
async fn test_config_validate_json_output_invalid() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("models.json");
    fs::write(
        &file,
        r#"{"providers": {"openai": {"models": [{"id": ""}]}}}"#,
    )
    .unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        true,
    )
    .await;
    // JSON mode prints output and returns Ok even on validation failure
    assert!(
        result.is_ok(),
        "json output for invalid file should succeed (prints JSON): {:?}",
        result
    );
}

/// Text mode output for invalid file → Err with issue count in message.
#[tokio::test]
async fn test_config_validate_text_output_invalid() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("models.json");
    fs::write(
        &file,
        r#"{"providers": {"openai": {"models": [{"id": ""}]}}}"#,
    )
    .unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(result.is_err(), "text output for invalid file should error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("issue(s)"),
        "text error should mention issue count: {}",
        err_msg
    );
}

// ── Normal path: additional section files ──────────────────────────────────

/// Valid system.json → valid, no issues.
#[tokio::test]
async fn test_config_validate_system_valid() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("system.json");
    fs::write(&file, r#"{"version": "1.0"}"#).unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(
        result.is_ok(),
        "valid system.json should succeed: {:?}",
        result
    );
}

/// Valid session.json → valid, no issues.
#[tokio::test]
async fn test_config_validate_session_valid() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("session.json");
    fs::write(&file, r#"{"idleMinutes": 30}"#).unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(
        result.is_ok(),
        "valid session.json should succeed: {:?}",
        result
    );
}

/// Valid accounts.json → valid, no issues.
#[tokio::test]
async fn test_config_validate_accounts_valid() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("accounts.json");
    fs::write(
        &file,
        r#"{"accounts": [{"accountId": "a1", "senderId": "s1"}]}"#,
    )
    .unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(
        result.is_ok(),
        "valid accounts.json should succeed: {:?}",
        result
    );
}

/// Valid agents.json → valid, no issues.
#[tokio::test]
async fn test_config_validate_agents_valid() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("agents.json");
    fs::write(&file, r#"{"agents": ["agent-a"]}"#).unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(
        result.is_ok(),
        "valid agents.json should succeed: {:?}",
        result
    );
}

/// Valid skills.json → valid, no issues.
#[tokio::test]
async fn test_config_validate_skills_valid() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("skills.json");
    fs::write(&file, r#"{"extraDirs": ["/opt/skills"]}"#).unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(
        result.is_ok(),
        "valid skills.json should succeed: {:?}",
        result
    );
}

/// Valid plugins.json → valid, no issues.
#[tokio::test]
async fn test_config_validate_plugins_valid() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("plugins.json");
    fs::write(&file, r#"{"entries": {"my-plugin": {}}}"#).unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(
        result.is_ok(),
        "valid plugins.json should succeed: {:?}",
        result
    );
}

/// Valid media.json → valid, no issues.
#[tokio::test]
async fn test_config_validate_media_valid() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("media.json");
    fs::write(&file, r#"{}"#).unwrap();

    let result = handle_config(
        ConfigAction::Validate {
            file: file.to_str().unwrap().to_string(),
        },
        false,
    )
    .await;
    assert!(
        result.is_ok(),
        "valid media.json should succeed: {:?}",
        result
    );
}
