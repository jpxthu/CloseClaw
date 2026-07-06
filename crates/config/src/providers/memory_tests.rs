//! Tests for global memory configuration provider.
//!
//! Covers: normal load (full memory.json), empty file (defaults),
//! partial config (undeclared fields use defaults), and edge cases.

use super::{ConfigProvider, MemoryConfigData};

// ── Helper ───────────────────────────────────────────────────────────────

/// Build a MemoryConfigData from a JSON string, panicking on error.
fn parse(json: &str) -> MemoryConfigData {
    MemoryConfigData::from_json_str(json).expect("should parse")
}

// ── Normal load: complete memory.json ────────────────────────────────────

#[test]
fn test_full_memory_json_all_sections() {
    let json = r#"{
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
        "storage": {
            "dbPath": "custom/memory.db",
            "memoryMdPath": "custom/MEMORY.md"
        }
    }"#;
    let data = parse(json);

    // Mining
    assert_eq!(data.config.mining.enabled, Some(true));
    assert_eq!(data.config.mining.model.as_deref(), Some("gpt-4o"));
    assert_eq!(data.config.mining.max_events_per_session, Some(20));
    assert_eq!(data.config.mining.dedup_window_days, Some(14));
    assert_eq!(data.config.mining.transcript_clean_rules.min_turns, Some(3));
    assert_eq!(
        data.config.mining.transcript_clean_rules.min_owner_msgs,
        Some(2)
    );
    assert_eq!(
        data.config.mining.transcript_clean_rules.format,
        Some("json".to_string())
    );

    // Dreaming
    assert_eq!(data.config.dreaming.enabled, Some(true));
    assert_eq!(data.config.dreaming.model.as_deref(), Some("claude-3"));
    assert_eq!(data.config.dreaming.schedule, Some("0 4 * * *".to_string()));
    assert_eq!(data.config.dreaming.scoring.frequency_weight, Some(2.0));
    assert_eq!(data.config.dreaming.scoring.recency_weight, Some(1.0));
    assert_eq!(data.config.dreaming.scoring.explicitness_weight, Some(3.0));
    assert_eq!(data.config.dreaming.scoring.cross_agent_weight, Some(2.5));
    assert_eq!(
        data.config.dreaming.scoring.negative_signal_weight,
        Some(-1.0)
    );
    assert_eq!(data.config.dreaming.threshold.absolute, Some(3.5));
    assert_eq!(data.config.dreaming.threshold.relative, Some(0.6));
    assert_eq!(data.config.dreaming.capacity.max_rules, Some(50));
    assert_eq!(data.config.dreaming.diary.enabled, Some(true));
    assert_eq!(
        data.config.dreaming.diary.path,
        Some("custom/diary/".to_string())
    );

    // Search
    assert_eq!(data.config.search.enabled, Some(true));
    assert_eq!(data.config.search.model.as_deref(), Some("search-model"));
    assert_eq!(data.config.search.context_turns, Some(10));
    assert_eq!(data.config.search.timeout_ms, Some(8000));
    assert_eq!(data.config.search.max_summary_chars, Some(1000));
    assert_eq!(data.config.search.min_entity_hits, Some(3));
    assert_eq!(data.config.search.top_k_events, Some(7));

    // Storage
    assert_eq!(
        data.config.storage.db_path,
        Some("custom/memory.db".to_string())
    );
    assert_eq!(
        data.config.storage.memory_md_path,
        Some("custom/MEMORY.md".to_string())
    );

    // is_default should be false (mining enabled)
    assert!(!data.is_default());
}

// ── Empty memory.json → all features default disabled ────────────────────

#[test]
fn test_empty_object_all_disabled() {
    let data = parse("{}");
    assert!(!data.config.mining.enabled.unwrap_or(false));
    assert!(!data.config.dreaming.enabled.unwrap_or(false));
    assert!(!data.config.search.enabled.unwrap_or(false));
    assert!(data.is_default());
}

#[test]
fn test_empty_array_not_valid_json_object() {
    // JSON array is not a valid MemoryConfigData
    let result = MemoryConfigData::from_json_str("[]");
    assert!(result.is_err());
}

#[test]
fn test_empty_string_not_valid_json() {
    let result = MemoryConfigData::from_json_str("");
    assert!(result.is_err());
}

// ── Partial config → undeclared fields use defaults ──────────────────────

#[test]
fn test_partial_mining_only() {
    let json = r#"{"mining": {"enabled": true, "maxEventsPerSession": 5}}"#;
    let data = parse(json);

    // Declared fields
    assert_eq!(data.config.mining.enabled, Some(true));
    assert_eq!(data.config.mining.max_events_per_session, Some(5));

    // Undeclared mining fields → None (inherit global at merge time)
    assert!(data.config.mining.model.is_none());
    assert!(data.config.mining.dedup_window_days.is_none());
    assert!(data
        .config
        .mining
        .transcript_clean_rules
        .min_turns
        .is_none());

    // Other sections → defaults
    assert!(!data.config.dreaming.enabled.unwrap_or(false));
    assert!(!data.config.search.enabled.unwrap_or(false));
}

#[test]
fn test_partial_dreaming_threshold_only() {
    let json = r#"{"dreaming": {"threshold": {"absolute": 5.0}}}"#;
    let data = parse(json);

    // Declared field
    assert_eq!(data.config.dreaming.threshold.absolute, Some(5.0));

    // Undeclared dreaming fields → None (inherit global at merge time)
    assert!(!data.config.dreaming.enabled.unwrap_or(false));
    assert!(data.config.dreaming.model.is_none());
    assert!(data.config.dreaming.threshold.relative.is_none());
    assert!(data.config.dreaming.capacity.max_rules.is_none());
    assert!(data.config.dreaming.schedule.is_none());
    assert!(data.config.dreaming.scoring.frequency_weight.is_none());
}

#[test]
fn test_partial_search_enabled_only() {
    let json = r#"{"search": {"enabled": true}}"#;
    let data = parse(json);

    // Declared field
    assert_eq!(data.config.search.enabled, Some(true));

    // Undeclared search fields → None (inherit global at merge time)
    assert!(data.config.search.model.is_none());
    assert!(data.config.search.timeout_ms.is_none());
    assert!(data.config.search.max_summary_chars.is_none());
    assert!(data.config.search.min_entity_hits.is_none());
    assert!(data.config.search.top_k_events.is_none());
    assert!(data.config.search.context_turns.is_none());
}

// ── ConfigProvider trait compliance ──────────────────────────────────────

#[test]
fn test_version_string() {
    let data = MemoryConfigData::default();
    assert_eq!(data.version(), "1.0.0");
}

#[test]
fn test_validate_always_succeeds() {
    let data = MemoryConfigData::default();
    assert!(data.validate().is_ok());
}

#[test]
fn test_is_default_all_disabled() {
    let data = MemoryConfigData::default();
    assert!(data.is_default());
}

#[test]
fn test_is_default_mining_enabled() {
    let json = r#"{"mining": {"enabled": true}}"#;
    let data = parse(json);
    assert!(!data.is_default());
}

#[test]
fn test_is_default_dreaming_enabled() {
    let json = r#"{"dreaming": {"enabled": true}}"#;
    let data = parse(json);
    assert!(!data.is_default());
}

#[test]
fn test_is_default_search_enabled() {
    let json = r#"{"search": {"enabled": true}}"#;
    let data = parse(json);
    assert!(!data.is_default());
}

// ── Enabled field semantics (Option<bool>) ──────────────────────────────

#[test]
fn test_enabled_true_explicit() {
    let json = r#"{"mining": {"enabled": true}}"#;
    let data = parse(json);
    assert_eq!(data.config.mining.enabled, Some(true));
}

#[test]
fn test_enabled_false_explicit() {
    let json = r#"{"mining": {"enabled": false}}"#;
    let data = parse(json);
    assert_eq!(data.config.mining.enabled, Some(false));
}

#[test]
fn test_enabled_not_present_is_none() {
    let json = r#"{"mining": {}}"#;
    let data = parse(json);
    assert_eq!(data.config.mining.enabled, None);
}

// ── Serialization round-trip ────────────────────────────────────────────

#[test]
fn test_roundtrip_preserves_all_fields() {
    let json = r#"{
        "mining": {"enabled": true, "model": "gpt-4o"},
        "dreaming": {"enabled": true, "threshold": {"absolute": 3.0}},
        "search": {"enabled": true, "timeoutMs": 5000}
    }"#;
    let data = parse(json);
    let serialized = serde_json::to_string(&data).unwrap();
    let deserialized: MemoryConfigData = serde_json::from_str(&serialized).unwrap();

    assert_eq!(
        data.config.mining.enabled,
        deserialized.config.mining.enabled
    );
    assert_eq!(data.config.mining.model, deserialized.config.mining.model);
    assert_eq!(
        data.config.dreaming.threshold.absolute,
        deserialized.config.dreaming.threshold.absolute
    );
    assert_eq!(
        data.config.search.timeout_ms,
        deserialized.config.search.timeout_ms
    );
}
