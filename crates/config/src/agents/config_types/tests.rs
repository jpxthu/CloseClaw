use super::*;

fn make_perms(agent_id: &str, allowed_dims: &[&str]) -> AgentPermissions {
    let dimensions = [
        "exec",
        "file_read",
        "file_write",
        "network",
        "spawn",
        "tool_call",
        "config_write",
    ];
    let permissions = dimensions
        .iter()
        .map(|&dim| {
            (
                dim.to_string(),
                ActionPermission {
                    allowed: allowed_dims.contains(&dim),
                    limits: PermissionLimits::default(),
                },
            )
        })
        .collect();
    AgentPermissions {
        agent_id: agent_id.to_string(),
        permissions,
        inherited_from: None,
    }
}

// --- intersect: normal path ---

#[test]
fn intersect_both_allow_preserves() {
    let child = make_perms("child", &["exec", "file_read"]);
    let parent = make_perms("parent", &["exec", "file_read"]);
    let result = child.intersect(&parent);
    assert!(result.permissions["exec"].allowed);
    assert!(result.permissions["file_read"].allowed);
}

#[test]
fn intersect_limits_commands_set_intersection() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    commands: vec!["git".into(), "ls".into()],
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let parent = AgentPermissions {
        agent_id: "parent".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    commands: vec!["git".into(), "cat".into()],
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    assert_eq!(result.permissions["exec"].limits.commands, vec!["git"]);
}

#[test]
fn intersect_limits_paths_set_intersection() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "file_read".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    paths: vec!["/data/**".into(), "/home/**".into()],
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let parent = AgentPermissions {
        agent_id: "parent".to_string(),
        permissions: HashMap::from([(
            "file_read".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    paths: vec!["/data/**".into(), "/etc/**".into()],
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    assert_eq!(
        result.permissions["file_read"].limits.paths,
        vec!["/data/**"]
    );
}

#[test]
fn intersect_limits_timeout_min() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    timeout_ms: Some(60_000),
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let parent = AgentPermissions {
        agent_id: "parent".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    timeout_ms: Some(30_000),
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    assert_eq!(result.permissions["exec"].limits.timeout_ms, Some(30_000));
}

// --- intersect: error path ---

#[test]
fn intersect_child_deny_overrides() {
    let child = make_perms("child", &["file_read"]); // exec denied
    let parent = make_perms("parent", &["exec", "file_read"]);
    let result = child.intersect(&parent);
    assert!(!result.permissions["exec"].allowed);
    assert!(result.permissions["file_read"].allowed);
}

#[test]
fn intersect_parent_deny_overrides() {
    let child = make_perms("child", &["exec", "file_read"]);
    let parent = make_perms("parent", &["exec"]); // file_read denied
    let result = child.intersect(&parent);
    assert!(result.permissions["exec"].allowed);
    assert!(!result.permissions["file_read"].allowed);
}

#[test]
fn intersect_absent_dimension_is_deny() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::new(),
        inherited_from: None,
    };
    let parent = make_perms("parent", &["exec"]);
    let result = child.intersect(&parent);
    assert!(!result.permissions["exec"].allowed);
}

#[test]
fn intersect_deny_gets_default_limits() {
    let child = make_perms("child", &[]); // all denied
    let parent = make_perms("parent", &["exec"]);
    let result = child.intersect(&parent);
    let exec = &result.permissions["exec"].limits;
    assert!(exec.commands.is_empty());
    assert!(exec.paths.is_empty());
    assert_eq!(exec.timeout_ms, None);
}

// --- intersect: boundary values ---

#[test]
fn intersect_limits_timeout_none_vs_none() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    timeout_ms: None,
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let parent = AgentPermissions {
        agent_id: "parent".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    timeout_ms: None,
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    assert_eq!(result.permissions["exec"].limits.timeout_ms, None);
}

#[test]
fn intersect_limits_timeout_none_vs_some() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    timeout_ms: None,
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let parent = AgentPermissions {
        agent_id: "parent".to_string(),
        permissions: HashMap::from([(
            "exec".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    timeout_ms: Some(5_000),
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    assert_eq!(result.permissions["exec"].limits.timeout_ms, Some(5_000));
}

#[test]
fn intersect_vec_none_none_returns_empty() {
    let result = intersect_vec::<String>(None, None);
    assert!(result.is_empty());
}

#[test]
fn intersect_vec_some_none_returns_some() {
    let a = vec!["x".to_string(), "y".to_string()];
    let result = intersect_vec(Some(&a), None);
    assert_eq!(result, vec!["x", "y"]);
}

#[test]
fn intersect_vec_none_some_returns_some() {
    let b = vec!["x".to_string(), "z".to_string()];
    let result = intersect_vec(None, Some(&b));
    assert_eq!(result, vec!["x", "z"]);
}

// --- state transition: is_fully_denied ---

#[test]
fn is_fully_denied_all_absent() {
    let perms = AgentPermissions {
        agent_id: "a".to_string(),
        permissions: HashMap::new(),
        inherited_from: None,
    };
    assert!(perms.is_fully_denied());
}

#[test]
fn is_fully_denied_all_explicit_deny() {
    let perms = make_perms("a", &[]);
    assert!(perms.is_fully_denied());
}

#[test]
fn is_fully_denied_one_allow() {
    let perms = make_perms("a", &["exec"]);
    assert!(!perms.is_fully_denied());
}

// --- intersect: result identity ---

#[test]
fn intersect_result_has_correct_ids() {
    let child = make_perms("child", &["exec"]);
    let parent = make_perms("parent", &["exec"]);
    let result = child.intersect(&parent);
    assert_eq!(result.agent_id, "child");
    assert_eq!(result.inherited_from, Some("parent".into()));
}

// --- intersect: all seven dimensions ---

#[test]
fn intersect_all_seven_dimensions_checked() {
    let child = make_perms("child", &["exec"]);
    let parent = make_perms("parent", &["exec"]);
    let result = child.intersect(&parent);
    assert_eq!(result.permissions.len(), 7);
    for dim in [
        "exec",
        "file_read",
        "file_write",
        "network",
        "spawn",
        "tool_call",
        "config_write",
    ] {
        assert!(
            result.permissions.contains_key(dim),
            "missing dimension: {dim}"
        );
    }
}

// ── MiningConfig tests ──────────────────────────────────────────────

#[test]
fn test_mining_config_defaults() {
    let config = MiningConfig::default();
    assert!(!config.enabled);
    assert!(config.model.is_none());
    assert_eq!(config.max_events_per_session, 10);
    assert_eq!(config.dedup_window_days, 30);
    assert_eq!(config.transcript_clean_rules.min_turns, 5);
    assert_eq!(config.transcript_clean_rules.min_owner_msgs, 5);
    assert_eq!(config.transcript_clean_rules.format, "md");
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
    assert!(config.enabled);
    assert_eq!(config.model.as_deref(), Some("gpt-4o"));
    assert_eq!(config.max_events_per_session, 20);
    assert_eq!(config.dedup_window_days, 14);
    assert_eq!(config.transcript_clean_rules.min_turns, 3);
    assert_eq!(config.transcript_clean_rules.min_owner_msgs, 2);
    assert_eq!(config.transcript_clean_rules.format, "json");
}

#[test]
fn test_mining_config_deserialize_minimal() {
    let json = r#"{"enabled": true}"#;
    let config: MiningConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled);
    assert!(config.model.is_none());
    assert_eq!(config.max_events_per_session, 10);
    assert_eq!(config.dedup_window_days, 30);
}

#[test]
fn test_mining_config_camel_case_roundtrip() {
    let json = r#"{
        "enabled": true,
        "maxEventsPerSession": 5,
        "dedupWindowDays": 7
    }"#;
    let config: MiningConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.max_events_per_session, 5);
    assert_eq!(config.dedup_window_days, 7);
    let serialized = serde_json::to_string(&config).unwrap();
    assert!(serialized.contains("maxEventsPerSession"));
    assert!(serialized.contains("dedupWindowDays"));
}

// ── DreamingConfig tests ───────────────────────────────────────────

#[test]
fn test_dreaming_config_defaults() {
    let config = DreamingConfig::default();
    assert!(!config.enabled);
    assert!(config.model.is_none());
    assert_eq!(config.schedule, "0 3 * * *");
    assert_eq!(config.scoring.frequency_weight, 1.0);
    assert_eq!(config.scoring.recency_weight, 0.5);
    assert_eq!(config.scoring.explicitness_weight, 1.5);
    assert_eq!(config.scoring.cross_agent_weight, 1.3);
    assert_eq!(config.scoring.negative_signal_weight, -0.5);
    assert_eq!(config.threshold.absolute, 2.0);
    assert_eq!(config.threshold.relative, 0.3);
    assert_eq!(config.capacity.max_rules, 20);
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
    assert!(config.enabled);
    assert_eq!(config.model.as_deref(), Some("claude-3"));
    assert_eq!(config.schedule, "0 4 * * *");
    assert_eq!(config.scoring.frequency_weight, 2.0);
    assert_eq!(config.scoring.recency_weight, 1.0);
    assert_eq!(config.scoring.explicitness_weight, 3.0);
    assert_eq!(config.scoring.cross_agent_weight, 2.5);
    assert_eq!(config.scoring.negative_signal_weight, -1.0);
    assert_eq!(config.threshold.absolute, 3.0);
    assert_eq!(config.threshold.relative, 0.5);
    assert_eq!(config.capacity.max_rules, 50);
}

#[test]
fn test_dreaming_config_deserialize_minimal() {
    let json = r#"{"enabled": true}"#;
    let config: DreamingConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled);
    assert_eq!(config.scoring.frequency_weight, 1.0);
    assert_eq!(config.threshold.absolute, 2.0);
    assert_eq!(config.capacity.max_rules, 20);
}

// ── SearchConfig tests ─────────────────────────────────────────────

#[test]
fn test_search_config_defaults() {
    let config = SearchConfig::default();
    assert!(!config.enabled);
    assert!(config.model.is_none());
    assert_eq!(config.context_turns, 5);
    assert_eq!(config.timeout_ms, 3000);
    assert_eq!(config.max_summary_chars, 500);
    assert_eq!(config.min_entity_hits, 1);
    assert_eq!(config.top_k_events, 3);
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
    assert!(config.enabled);
    assert_eq!(config.model.as_deref(), Some("search-model"));
    assert_eq!(config.context_turns, 8);
    assert_eq!(config.timeout_ms, 5000);
    assert_eq!(config.max_summary_chars, 1000);
    assert_eq!(config.min_entity_hits, 2);
    assert_eq!(config.top_k_events, 10);
}

#[test]
fn test_search_config_deserialize_minimal() {
    let json = r#"{"enabled": true}"#;
    let config: SearchConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled);
    assert_eq!(config.context_turns, 5);
    assert_eq!(config.timeout_ms, 3000);
    assert_eq!(config.max_summary_chars, 500);
    assert_eq!(config.min_entity_hits, 1);
    assert_eq!(config.top_k_events, 3);
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
    assert_eq!(config.context_turns, 7);
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
    assert_eq!(rules.min_turns, 5);
    assert_eq!(rules.min_owner_msgs, 5);
    assert_eq!(rules.format, "md");
}

#[test]
fn test_transcript_clean_rules_camel_case() {
    let json = r#"{
        "minTurns": 3,
        "minOwnerMsgs": 2,
        "format": "json"
    }"#;
    let rules: TranscriptCleanRules = serde_json::from_str(json).unwrap();
    assert_eq!(rules.min_turns, 3);
    assert_eq!(rules.min_owner_msgs, 2);
    assert_eq!(rules.format, "json");
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
    assert!(config.mining.enabled);
    assert_eq!(config.mining.max_events_per_session, 15);
    assert!(config.dreaming.enabled);
    assert_eq!(config.dreaming.threshold.absolute, 1.0);
    assert!(config.search.enabled);
    assert_eq!(config.search.timeout_ms, 6000);
}
