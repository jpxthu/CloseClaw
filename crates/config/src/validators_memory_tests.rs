//! Tests for memory config section validator.
//!
//! Covers: enabled must be boolean, numeric params non-negative,
//! model/path fields valid, and full valid config passes.

use crate::validators::validate_memory;

// ── Top-level shape ──────────────────────────────────────────────────────

#[test]
fn test_validate_memory_pass_empty_object() {
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(validate_memory(&v).is_ok());
}

#[test]
fn test_validate_memory_fail_not_object() {
    for json in [r#"[1]"#, r#""string""#, r#"null"#, r#"42"#] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let err = validate_memory(&v).unwrap_err();
        assert!(err.contains("JSON object"), "json={}: error: {}", json, err);
    }
}

// ── enabled must be boolean ──────────────────────────────────────────────

#[test]
fn test_validate_memory_mining_enabled_string_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"mining":{"enabled":"yes"}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.mining.enabled must be a boolean"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_dreaming_enabled_number_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"dreaming":{"enabled":1}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.dreaming.enabled must be a boolean"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_search_enabled_null_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"search":{"enabled":null}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.search.enabled must be a boolean"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_enabled_boolean_passes() {
    for json in [
        r#"{"mining":{"enabled":true}}"#,
        r#"{"dreaming":{"enabled":false}}"#,
        r#"{"search":{"enabled":true}}"#,
    ] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(validate_memory(&v).is_ok(), "json={}", json);
    }
}

// ── Numeric params must be non-negative ──────────────────────────────────

#[test]
fn test_validate_memory_mining_negative_events_rejected() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"mining":{"maxEventsPerSession":-1}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.mining.maxEventsPerSession must be non-negative"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_mining_negative_dedup_rejected() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"mining":{"dedupWindowDays":-5}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.mining.dedupWindowDays must be non-negative"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_mining_not_number_rejected() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"mining":{"maxEventsPerSession":"abc"}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.mining.maxEventsPerSession must be a number"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_dreaming_negative_threshold_rejected() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"dreaming":{"threshold":{"absolute":-1.0}}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.dreaming.threshold.absolute must be non-negative"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_dreaming_negative_max_rules_rejected() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"dreaming":{"capacity":{"maxRules":-10}}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.dreaming.capacity.maxRules must be non-negative"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_search_negative_timeout_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"search":{"timeoutMs":-1}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.search.timeoutMs must be non-negative"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_search_not_number_rejected() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"search":{"contextTurns":"abc"}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.search.contextTurns must be a number"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_forgetting_negative_rejected() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"forgetting":{"initialTtlDays":-1}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.forgetting.initialTtlDays must be non-negative"),
        "error: {}",
        err
    );
}

// ── Zero values are valid (non-negative includes zero) ───────────────────

#[test]
fn test_validate_memory_zero_values_pass() {
    let json = r#"{
        "mining": {"maxEventsPerSession": 0, "dedupWindowDays": 0},
        "search": {"timeoutMs": 0, "contextTurns": 0, "maxSummaryChars": 0, "minEntityHits": 0, "topKEvents": 0},
        "forgetting": {"initialTtlDays": 0, "reidentifyExtensionDays": 0, "injectionExtensionDays": 0}
    }"#;
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    assert!(validate_memory(&v).is_ok());
}

// ── Model/path fields ────────────────────────────────────────────────────

#[test]
fn test_validate_memory_model_empty_string_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"mining":{"model":""}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.mining.model cannot be empty"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_model_not_string_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"mining":{"model":123}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.mining.model must be a string"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_model_null_passes() {
    let v: serde_json::Value = serde_json::from_str(r#"{"mining":{"model":null}}"#).unwrap();
    assert!(validate_memory(&v).is_ok());
}

#[test]
fn test_validate_memory_model_valid_string_passes() {
    let v: serde_json::Value = serde_json::from_str(r#"{"mining":{"model":"gpt-4o"}}"#).unwrap();
    assert!(validate_memory(&v).is_ok());
}

// ── Storage paths ────────────────────────────────────────────────────────

#[test]
fn test_validate_memory_storage_not_object_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"storage":"bad"}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.storage must be a JSON object"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_storage_empty_path_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"storage":{"dbPath":""}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.storage.dbPath cannot be empty"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_storage_valid_path_passes() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"storage":{"dbPath":"custom/memory.db","memoryMdPath":"custom/MEMORY.md"}}"#,
    )
    .unwrap();
    assert!(validate_memory(&v).is_ok());
}

// ── Subsystem not-object rejection ───────────────────────────────────────

#[test]
fn test_validate_memory_mining_not_object_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"mining":"bad"}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.mining must be a JSON object"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_dreaming_not_object_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"dreaming":[1,2,3]}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.dreaming must be a JSON object"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_search_not_object_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"search":true}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.search must be a JSON object"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_forgetting_not_object_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"forgetting":"bad"}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.forgetting must be a JSON object"),
        "error: {}",
        err
    );
}

// ── Dreaming sub-objects ─────────────────────────────────────────────────

#[test]
fn test_validate_memory_dreaming_scoring_not_object_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"dreaming":{"scoring":"bad"}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.dreaming.scoring must be a JSON object"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_dreaming_threshold_not_object_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"dreaming":{"threshold":42}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.dreaming.threshold must be a JSON object"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_dreaming_capacity_not_object_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"dreaming":{"capacity":"bad"}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.dreaming.capacity must be a JSON object"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_dreaming_diary_not_object_rejected() {
    let v: serde_json::Value = serde_json::from_str(r#"{"dreaming":{"diary":42}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.dreaming.diary must be a JSON object"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_dreaming_scoring_not_number_rejected() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"dreaming":{"scoring":{"frequencyWeight":"abc"}}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.dreaming.scoring.frequencyWeight must be a number"),
        "error: {}",
        err
    );
}

// ── Dreaming negative_signal_weight is allowed (negative) ────────────────

#[test]
fn test_validate_memory_dreaming_negative_signal_weight_allowed() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"dreaming":{"scoring":{"negativeSignalWeight":-0.5}}}"#).unwrap();
    assert!(validate_memory(&v).is_ok());
}

// ── Full valid memory.json passes ────────────────────────────────────────

#[test]
fn test_validate_memory_full_valid_config() {
    let json = r#"{
        "storage": {
            "dbPath": "memory/memory.db",
            "memoryMdPath": "memory/MEMORY.md"
        },
        "mining": {
            "enabled": true,
            "model": "gpt-4o",
            "maxEventsPerSession": 20,
            "dedupWindowDays": 14,
            "transcriptCleanRules": {
                "minTurns": 3,
                "minOwnerMsgs": 2,
                "format": "json"
            }
        },
        "dreaming": {
            "enabled": true,
            "model": "claude-3",
            "schedule": "0 4 * * *",
            "scoring": {
                "frequencyWeight": 2.0,
                "recencyWeight": 1.0,
                "explicitnessWeight": 3.0,
                "crossAgentWeight": 2.5,
                "negativeSignalWeight": -1.0
            },
            "threshold": {
                "absolute": 3.5,
                "relative": 0.6
            },
            "capacity": {
                "maxRules": 50
            },
            "diary": {
                "enabled": true,
                "path": "custom/diary/"
            }
        },
        "search": {
            "enabled": true,
            "model": "search-model",
            "contextTurns": 10,
            "timeoutMs": 8000,
            "maxSummaryChars": 1000,
            "minEntityHits": 3,
            "topKEvents": 7
        },
        "forgetting": {
            "initialTtlDays": 90,
            "reidentifyExtensionDays": 90,
            "injectionExtensionDays": 7
        }
    }"#;
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    assert!(validate_memory(&v).is_ok());
}

// ── Empty memory.json (only defaults) passes ─────────────────────────────

#[test]
fn test_validate_memory_empty_json_passes() {
    let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
    assert!(validate_memory(&v).is_ok());
}

// ── Partial config with valid values passes ──────────────────────────────

#[test]
fn test_validate_memory_partial_config_passes() {
    let json = r#"{
        "mining": {"enabled": true},
        "search": {"timeoutMs": 5000}
    }"#;
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    assert!(validate_memory(&v).is_ok());
}

// ── transcriptCleanRules validation ──────────────────────────────────────

#[test]
fn test_validate_memory_mining_transcript_rules_not_object_rejected() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"mining":{"transcriptCleanRules":"bad"}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.mining.transcriptCleanRules must be a JSON object"),
        "error: {}",
        err
    );
}

#[test]
fn test_validate_memory_mining_transcript_rules_negative_rejected() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"mining":{"transcriptCleanRules":{"minTurns":-1}}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.mining.transcriptCleanRules.minTurns must be non-negative"),
        "error: {}",
        err
    );
}

// ── Forgetting not-number rejection ──────────────────────────────────────

#[test]
fn test_validate_memory_forgetting_not_number_rejected() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"forgetting":{"initialTtlDays":"abc"}}"#).unwrap();
    let err = validate_memory(&v).unwrap_err();
    assert!(
        err.contains("memory.forgetting.initialTtlDays must be a number"),
        "error: {}",
        err
    );
}

// ── ConfigProvider validate() integration ────────────────────────────────

#[test]
fn test_memory_config_provider_validate_with_valid_config() {
    use crate::providers::memory::MemoryConfigData;
    use crate::providers::ConfigProvider;

    let data = MemoryConfigData::from_json_str(r#"{"mining":{"enabled":true}}"#).unwrap();
    assert!(data.validate().is_ok());
}

#[test]
fn test_memory_config_provider_validate_empty_passes() {
    use crate::providers::memory::MemoryConfigData;
    use crate::providers::ConfigProvider;

    let data = MemoryConfigData::from_json_str(r#"{}"#).unwrap();
    assert!(data.validate().is_ok());
}
