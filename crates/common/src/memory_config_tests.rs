//! Tests for the memory config type family (serde / Default / merge_overrides).
//!
//! Migrated from `closeclaw_config::agents::config_types` alongside the type
//! definitions in `memory_config.rs` (issue #3344).

use super::*;

// ── MiningConfig tests ──────────────────────────────────────────────

#[test]
fn test_mining_config_defaults() {
    let config = MiningConfig::default();
    assert!(!config.enabled.unwrap_or(false));
    assert!(config.model.is_none());
    assert!(config.max_events_per_session.is_none());
    assert!(config.dedup_window_days.is_none());
    assert!(config.transcript_clean_rules.min_turns.is_none());
    assert!(config.transcript_clean_rules.min_owner_msgs.is_none());
    assert!(config.transcript_clean_rules.format.is_none());
}

#[test]
fn test_mining_config_deserialize_full() {
    let json = r#"{
        "enabled": true,
        "model": "gpt-4o",
        "maxEventsPerSession": 20,
        "dedupWindowDays": 14,
        "transcriptCleanRules": {
            "minTurns": 3,
            "minOwnerMsgs": 2,
            "format": "json"
        }
    }"#;
    let config: MiningConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled == Some(true));
    assert_eq!(config.model.as_deref(), Some("gpt-4o"));
    assert_eq!(config.max_events_per_session, Some(20));
    assert_eq!(config.dedup_window_days, Some(14));
    assert_eq!(config.transcript_clean_rules.min_turns, Some(3));
    assert_eq!(config.transcript_clean_rules.min_owner_msgs, Some(2));
    assert_eq!(
        config.transcript_clean_rules.format,
        Some("json".to_string())
    );
}

#[test]
fn test_mining_config_deserialize_minimal() {
    let json = r#"{"enabled": true}"#;
    let config: MiningConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled == Some(true));
    assert!(config.model.is_none());
    assert!(config.max_events_per_session.is_none());
    assert!(config.dedup_window_days.is_none());
}

#[test]
fn test_mining_config_camel_case_roundtrip() {
    let json = r#"{
        "enabled": true,
        "maxEventsPerSession": 5,
        "dedupWindowDays": 7
    }"#;
    let config: MiningConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.max_events_per_session, Some(5));
    assert_eq!(config.dedup_window_days, Some(7));
    let serialized = serde_json::to_string(&config).unwrap();
    assert!(serialized.contains("maxEventsPerSession"));
    assert!(serialized.contains("dedupWindowDays"));
}

// ── DreamingConfig tests ───────────────────────────────────────────

#[test]
fn test_dreaming_config_defaults() {
    let config = DreamingConfig::default();
    assert!(!config.enabled.unwrap_or(false));
    assert!(config.model.is_none());
    assert!(config.schedule.is_none());
    assert!(config.scoring.frequency_weight.is_none());
    assert!(config.scoring.recency_weight.is_none());
    assert!(config.scoring.explicitness_weight.is_none());
    assert!(config.scoring.cross_agent_weight.is_none());
    assert!(config.scoring.negative_signal_weight.is_none());
    assert!(config.threshold.absolute.is_none());
    assert!(config.threshold.relative.is_none());
    assert!(config.capacity.max_rules.is_none());
}

#[test]
fn test_dreaming_config_deserialize_full() {
    let json = r#"{
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
            "absolute": 3.0,
            "relative": 0.5
        },
        "capacity": {
            "maxRules": 50
        }
    }"#;
    let config: DreamingConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled == Some(true));
    assert_eq!(config.model.as_deref(), Some("claude-3"));
    assert_eq!(config.schedule, Some("0 4 * * *".to_string()));
    assert_eq!(config.scoring.frequency_weight, Some(2.0));
    assert_eq!(config.scoring.recency_weight, Some(1.0));
    assert_eq!(config.scoring.explicitness_weight, Some(3.0));
    assert_eq!(config.scoring.cross_agent_weight, Some(2.5));
    assert_eq!(config.scoring.negative_signal_weight, Some(-1.0));
    assert_eq!(config.threshold.absolute, Some(3.0));
    assert_eq!(config.threshold.relative, Some(0.5));
    assert_eq!(config.capacity.max_rules, Some(50));
}

#[test]
fn test_dreaming_config_deserialize_minimal() {
    let json = r#"{"enabled": true}"#;
    let config: DreamingConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled == Some(true));
    assert!(config.scoring.frequency_weight.is_none());
    assert!(config.threshold.absolute.is_none());
    assert!(config.capacity.max_rules.is_none());
}

// ── SearchConfig tests ─────────────────────────────────────────────

#[test]
fn test_search_config_defaults() {
    let config = SearchConfig::default();
    assert!(!config.enabled.unwrap_or(false));
    assert!(config.model.is_none());
    assert!(config.context_turns.is_none());
    assert!(config.timeout_ms.is_none());
    assert!(config.max_summary_chars.is_none());
    assert!(config.min_entity_hits.is_none());
    assert!(config.top_k_events.is_none());
}

#[test]
fn test_search_config_deserialize_full() {
    let json = r#"{
        "enabled": true,
        "model": "search-model",
        "contextTurns": 8,
        "timeoutMs": 5000,
        "maxSummaryChars": 1000,
        "minEntityHits": 2,
        "topKEvents": 10
    }"#;
    let config: SearchConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled == Some(true));
    assert_eq!(config.model.as_deref(), Some("search-model"));
    assert_eq!(config.context_turns, Some(8));
    assert_eq!(config.timeout_ms, Some(5000));
    assert_eq!(config.max_summary_chars, Some(1000));
    assert_eq!(config.min_entity_hits, Some(2));
    assert_eq!(config.top_k_events, Some(10));
}

#[test]
fn test_search_config_deserialize_minimal() {
    let json = r#"{"enabled": true}"#;
    let config: SearchConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled == Some(true));
    assert!(config.context_turns.is_none());
    assert!(config.timeout_ms.is_none());
    assert!(config.max_summary_chars.is_none());
    assert!(config.min_entity_hits.is_none());
    assert!(config.top_k_events.is_none());
}

#[test]
fn test_search_config_camel_case_roundtrip() {
    let json = r#"{
        "enabled": true,
        "contextTurns": 7,
        "timeoutMs": 4000,
        "maxSummaryChars": 800,
        "minEntityHits": 2,
        "topKEvents": 5
    }"#;
    let config: SearchConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.context_turns, Some(7));
    let serialized = serde_json::to_string(&config).unwrap();
    assert!(serialized.contains("contextTurns"));
    assert!(serialized.contains("timeoutMs"));
    assert!(serialized.contains("maxSummaryChars"));
    assert!(serialized.contains("minEntityHits"));
    assert!(serialized.contains("topKEvents"));
}

// ── TranscriptCleanRules tests ─────────────────────────────────────

#[test]
fn test_transcript_clean_rules_defaults() {
    let rules = TranscriptCleanRules::default();
    assert!(rules.min_turns.is_none());
    assert!(rules.min_owner_msgs.is_none());
    assert!(rules.format.is_none());
}

#[test]
fn test_transcript_clean_rules_camel_case() {
    let json = r#"{
        "minTurns": 3,
        "minOwnerMsgs": 2,
        "format": "json"
    }"#;
    let rules: TranscriptCleanRules = serde_json::from_str(json).unwrap();
    assert_eq!(rules.min_turns, Some(3));
    assert_eq!(rules.min_owner_msgs, Some(2));
    assert_eq!(rules.format, Some("json".to_string()));
}

// ── MemoryConfig full deserialization ──────────────────────────────

#[test]
fn test_memory_config_full_deserialize() {
    let json = r#"{
        "mining": {
            "enabled": true,
            "maxEventsPerSession": 15
        },
        "dreaming": {
            "enabled": true,
            "threshold": { "absolute": 1.0 }
        },
        "search": {
            "enabled": true,
            "timeoutMs": 6000
        }
    }"#;
    let config: MemoryConfig = serde_json::from_str(json).unwrap();
    assert!(config.mining.enabled == Some(true));
    assert_eq!(config.mining.max_events_per_session, Some(15));
    assert!(config.dreaming.enabled == Some(true));
    assert_eq!(config.dreaming.threshold.absolute, Some(1.0));
    assert!(config.search.enabled == Some(true));
    assert_eq!(config.search.timeout_ms, Some(6000));
}

// ── ForgettingConfig tests ──────────────────────────────────────────

#[test]
fn test_forgetting_config_defaults() {
    let config = ForgettingConfig::default();
    assert!(config.initial_ttl_days.is_none());
    assert!(config.reidentify_extension_days.is_none());
    assert!(config.injection_extension_days.is_none());
}

#[test]
fn test_forgetting_config_deserialize_full() {
    let json = r#"{
        "initialTtlDays": 30,
        "reidentifyExtensionDays": 60,
        "injectionExtensionDays": 14
    }"#;
    let config: ForgettingConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.initial_ttl_days, Some(30));
    assert_eq!(config.reidentify_extension_days, Some(60));
    assert_eq!(config.injection_extension_days, Some(14));
}

#[test]
fn test_forgetting_config_deserialize_minimal() {
    let json = r#"{"initialTtlDays": 30}"#;
    let config: ForgettingConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.initial_ttl_days, Some(30));
    assert!(config.reidentify_extension_days.is_none());
    assert!(config.injection_extension_days.is_none());
}

#[test]
fn test_forgetting_config_camel_case_roundtrip() {
    let json = r#"{
        "initialTtlDays": 60,
        "reidentifyExtensionDays": 45,
        "injectionExtensionDays": 10
    }"#;
    let config: ForgettingConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.initial_ttl_days, Some(60));
    assert_eq!(config.reidentify_extension_days, Some(45));
    assert_eq!(config.injection_extension_days, Some(10));
    let serialized = serde_json::to_string(&config).unwrap();
    assert!(serialized.contains("initialTtlDays"));
    assert!(serialized.contains("reidentifyExtensionDays"));
    assert!(serialized.contains("injectionExtensionDays"));
}

#[test]
fn test_forgetting_config_merge_agent_overrides_global() {
    let global = ForgettingConfig {
        initial_ttl_days: Some(90),
        reidentify_extension_days: Some(90),
        injection_extension_days: Some(7),
    };
    let agent = ForgettingConfig {
        initial_ttl_days: Some(30),
        reidentify_extension_days: Some(60),
        injection_extension_days: None,
    };
    let merged = global.merge_overrides(&agent);
    assert_eq!(merged.initial_ttl_days, Some(30));
    assert_eq!(merged.reidentify_extension_days, Some(60));
    assert_eq!(merged.injection_extension_days, Some(7));
}

#[test]
fn test_forgetting_config_merge_both_none() {
    let global = ForgettingConfig::default();
    let agent = ForgettingConfig::default();
    let merged = global.merge_overrides(&agent);
    assert!(merged.initial_ttl_days.is_none());
    assert!(merged.injection_extension_days.is_none());
}

#[test]
fn test_memory_config_deserialize_with_forgetting() {
    let json = r#"{
        "forgetting": {
            "initialTtlDays": 30
        }
    }"#;
    let config: MemoryConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.forgetting.initial_ttl_days, Some(30));
    assert!(config.forgetting.injection_extension_days.is_none());
}

#[test]
fn test_memory_config_deserialize_without_forgetting() {
    let json = r#"{"mining": {"enabled": true}}"#;
    let config: MemoryConfig = serde_json::from_str(json).unwrap();
    assert!(config.forgetting.initial_ttl_days.is_none());
    assert!(config.forgetting.injection_extension_days.is_none());
}

#[test]
fn test_memory_config_merge_includes_forgetting() {
    let global = MemoryConfig {
        forgetting: ForgettingConfig {
            initial_ttl_days: Some(90),
            reidentify_extension_days: None,
            injection_extension_days: Some(7),
        },
        ..Default::default()
    };
    let agent = MemoryConfig {
        forgetting: ForgettingConfig {
            initial_ttl_days: Some(30),
            reidentify_extension_days: None,
            injection_extension_days: None,
        },
        ..Default::default()
    };
    let merged = global.merge_overrides(&agent);
    assert_eq!(merged.forgetting.initial_ttl_days, Some(30));
    assert_eq!(merged.forgetting.injection_extension_days, Some(7));
}

#[test]
fn test_default_forgetting_values() {
    assert_eq!(default_forgetting_initial_ttl_days(), 90);
    assert_eq!(default_forgetting_reidentify_extension_days(), 90);
    assert_eq!(default_forgetting_injection_extension_days(), 7);
}

// ── MemoryStorageConfig tests ───────────────────────────────────────

#[test]
fn test_memory_storage_config_camel_case_roundtrip() {
    let json = r#"{
        "dbPath": "memory/agent-a.db",
        "memoryMdPath": "memory/MEMORY.md"
    }"#;
    let config: MemoryStorageConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.db_path.as_deref(), Some("memory/agent-a.db"));
    assert_eq!(config.memory_md_path.as_deref(), Some("memory/MEMORY.md"));
    let serialized = serde_json::to_string(&config).unwrap();
    assert!(serialized.contains("\"dbPath\""));
    assert!(serialized.contains("\"memoryMdPath\""));
}

// ── MemoryConfig::merge_overrides field-level semantics ────────────

#[test]
fn test_memory_config_merge_overrides_field_level() {
    // Agent-declared fields override global; undeclared (None) fields
    // inherit the global value — across every subsystem at once.
    let global = MemoryConfig {
        storage: MemoryStorageConfig {
            db_path: Some("memory/global.db".to_string()),
            memory_md_path: Some("memory/MEMORY.md".to_string()),
        },
        mining: MiningConfig {
            enabled: Some(true),
            max_events_per_session: Some(10),
            ..Default::default()
        },
        dreaming: DreamingConfig {
            schedule: Some("0 3 * * *".to_string()),
            threshold: DreamingThresholdConfig {
                absolute: Some(2.0),
                relative: None,
            },
            ..Default::default()
        },
        search: SearchConfig {
            timeout_ms: Some(3000),
            top_k_events: Some(3),
            ..Default::default()
        },
        forgetting: ForgettingConfig {
            initial_ttl_days: Some(90),
            ..Default::default()
        },
    };
    let agent = MemoryConfig {
        storage: MemoryStorageConfig {
            db_path: Some("memory/agent-a.db".to_string()),
            memory_md_path: None,
        },
        mining: MiningConfig {
            enabled: Some(false),
            max_events_per_session: None,
            ..Default::default()
        },
        dreaming: DreamingConfig {
            schedule: None,
            threshold: DreamingThresholdConfig {
                absolute: None,
                relative: Some(0.3),
            },
            ..Default::default()
        },
        search: SearchConfig {
            timeout_ms: None,
            top_k_events: Some(5),
            ..Default::default()
        },
        forgetting: ForgettingConfig::default(),
    };
    let merged = global.merge_overrides(&agent);
    // Agent overrides win where declared…
    assert_eq!(merged.storage.db_path.as_deref(), Some("memory/agent-a.db"));
    assert_eq!(merged.mining.enabled, Some(false));
    assert_eq!(merged.dreaming.threshold.relative, Some(0.3));
    assert_eq!(merged.search.top_k_events, Some(5));
    // …global values survive where the agent left the field undeclared.
    assert_eq!(merged.mining.max_events_per_session, Some(10));
    assert_eq!(merged.dreaming.schedule.as_deref(), Some("0 3 * * *"));
    assert_eq!(merged.dreaming.threshold.absolute, Some(2.0));
    assert_eq!(merged.search.timeout_ms, Some(3000));
    assert_eq!(merged.forgetting.initial_ttl_days, Some(90));
}
