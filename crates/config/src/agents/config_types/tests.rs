use super::*;

fn make_perms(agent_id: &str, allowed_dims: &[&str]) -> AgentPermissions {
    let dimensions = [
        "command",
        "file_read",
        "file_write",
        "network",
        "spawn",
        "tool_call",
        "config_write",
        "message",
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

// ── HookConfig / AgentConfig.hooks tests ───────────────────────

#[test]
fn test_agent_config_hooks_default_empty() {
    let config = AgentConfig::default();
    assert!(config.hooks.is_empty());
}

#[test]
fn test_agent_config_deserialize_old_config_without_hooks() {
    let json = r#"{"id": "test-agent", "name": "Test"}"#;
    let config: AgentConfig = serde_json::from_str(json).unwrap();
    assert!(config.hooks.is_empty());
}

#[test]
fn test_agent_config_deserialize_with_hooks() {
    let json = r#"{
        "id": "test-agent",
        "hooks": [
            {"hookType": "planCheck", "enabled": true},
            {"hookType": "loopCheck", "enabled": false}
        ]
    }"#;
    let config: AgentConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.hooks.len(), 2);
    assert_eq!(
        config.hooks[0].hook_type,
        closeclaw_common::HookType::PlanCheck
    );
    assert!(config.hooks[0].enabled);
    assert_eq!(
        config.hooks[1].hook_type,
        closeclaw_common::HookType::LoopCheck
    );
    assert!(!config.hooks[1].enabled);
}

#[test]
fn test_agent_config_serialize_empty_hooks_skipped() {
    let config = AgentConfig {
        id: "test".to_string(),
        hooks: Vec::new(),
        ..Default::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(!json.contains("hooks"));
}

#[test]
fn test_agent_config_serialize_nonempty_hooks() {
    let config = AgentConfig {
        id: "test".to_string(),
        hooks: vec![closeclaw_common::HookConfig {
            hook_type: closeclaw_common::HookType::PlanCheck,
            enabled: true,
        }],
        ..Default::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("hooks"));
    assert!(json.contains("planCheck"));
}

#[test]
fn test_hook_config_default_enabled() {
    let config = closeclaw_common::HookConfig::default();
    assert!(config.enabled);
    assert_eq!(config.hook_type, closeclaw_common::HookType::PlanCheck);
}

// --- intersect: normal path ---

#[test]
fn intersect_both_allow_preserves() {
    let child = make_perms("child", &["command", "file_read"]);
    let parent = make_perms("parent", &["command", "file_read"]);
    let result = child.intersect(&parent);
    assert!(result.permissions["command"].allowed);
    assert!(result.permissions["file_read"].allowed);
}

#[test]
fn intersect_limits_commands_set_intersection() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "command".to_string(),
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
            "command".to_string(),
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
    assert_eq!(result.permissions["command"].limits.commands, vec!["git"]);
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
            "command".to_string(),
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
            "command".to_string(),
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
    assert_eq!(
        result.permissions["command"].limits.timeout_ms,
        Some(30_000)
    );
}

// --- intersect: error path ---

#[test]
fn intersect_child_deny_overrides() {
    let child = make_perms("child", &["file_read"]); // exec denied
    let parent = make_perms("parent", &["command", "file_read"]);
    let result = child.intersect(&parent);
    assert!(!result.permissions["command"].allowed);
    assert!(result.permissions["file_read"].allowed);
}

#[test]
fn intersect_parent_deny_overrides() {
    let child = make_perms("child", &["command", "file_read"]);
    let parent = make_perms("parent", &["command"]); // file_read denied
    let result = child.intersect(&parent);
    assert!(result.permissions["command"].allowed);
    assert!(!result.permissions["file_read"].allowed);
}

#[test]
fn intersect_absent_dimension_is_deny() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::new(),
        inherited_from: None,
    };
    let parent = make_perms("parent", &["command"]);
    let result = child.intersect(&parent);
    assert!(!result.permissions["command"].allowed);
}

#[test]
fn intersect_deny_gets_default_limits() {
    let child = make_perms("child", &[]); // all denied
    let parent = make_perms("parent", &["command"]);
    let result = child.intersect(&parent);
    let exec = &result.permissions["command"].limits;
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
            "command".to_string(),
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
            "command".to_string(),
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
    assert_eq!(result.permissions["command"].limits.timeout_ms, None);
}

#[test]
fn intersect_limits_timeout_none_vs_some() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "command".to_string(),
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
            "command".to_string(),
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
    assert_eq!(result.permissions["command"].limits.timeout_ms, Some(5_000));
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
    let perms = make_perms("a", &["command"]);
    assert!(!perms.is_fully_denied());
}

// --- intersect: result identity ---

#[test]
fn intersect_result_has_correct_ids() {
    let child = make_perms("child", &["command"]);
    let parent = make_perms("parent", &["command"]);
    let result = child.intersect(&parent);
    assert_eq!(result.agent_id, "child");
    assert_eq!(result.inherited_from, Some("parent".into()));
}

// --- intersect: all eight dimensions ---

#[test]
fn intersect_all_eight_dimensions_checked() {
    let child = make_perms("child", &["command"]);
    let parent = make_perms("parent", &["command"]);
    let result = child.intersect(&parent);
    assert_eq!(result.permissions.len(), 8);
    for dim in [
        "command",
        "file_read",
        "file_write",
        "network",
        "spawn",
        "tool_call",
        "config_write",
        "message",
    ] {
        assert!(
            result.permissions.contains_key(dim),
            "missing dimension: {dim}"
        );
    }
}

// --- intersect: message dimension ---

#[test]
fn intersect_parent_deny_child_allow_message_is_deny() {
    let child = make_perms("child", &["command", "file_read", "message"]);
    let parent = make_perms("parent", &["command", "file_read"]); // message absent = deny
    let result = child.intersect(&parent);
    assert!(!result.permissions["message"].allowed);
}

#[test]
fn intersect_both_allow_message_is_allow() {
    let child = make_perms("child", &["command", "message"]);
    let parent = make_perms("parent", &["command", "message"]);
    let result = child.intersect(&parent);
    assert!(result.permissions["message"].allowed);
}

#[test]
fn intersect_child_allow_parent_absent_message_is_deny() {
    let child = make_perms("child", &["command", "message"]);
    // parent has no message dimension → absent treated as deny
    let parent = AgentPermissions {
        agent_id: "parent".to_string(),
        permissions: HashMap::from([(
            "command".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits::default(),
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    assert!(!result.permissions["message"].allowed);
}

// --- is_fully_denied: message dimension ---

#[test]
fn is_fully_denied_seven_deny_message_allow_is_false() {
    // All 7 non-message dimensions deny, message allow → not fully denied
    let perms = make_perms("a", &["message"]);
    assert!(!perms.is_fully_denied());
}

#[test]
fn is_fully_denied_all_eight_deny_is_true() {
    let perms = make_perms("a", &[]);
    assert!(perms.is_fully_denied());
}

// --- intersect: message dimension limits ---

#[test]
fn intersect_message_limits_commands_set_intersection() {
    let child = AgentPermissions {
        agent_id: "child".to_string(),
        permissions: HashMap::from([(
            "message".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    commands: vec!["send".into(), "edit".into()],
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let parent = AgentPermissions {
        agent_id: "parent".to_string(),
        permissions: HashMap::from([(
            "message".to_string(),
            ActionPermission {
                allowed: true,
                limits: PermissionLimits {
                    commands: vec!["send".into(), "delete".into()],
                    ..Default::default()
                },
            },
        )]),
        inherited_from: None,
    };
    let result = child.intersect(&parent);
    assert!(result.permissions["message"].allowed);
    assert_eq!(result.permissions["message"].limits.commands, vec!["send"]);
}

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

// ── SubagentsConfig timeout tests ─────────────────────────────────

#[test]
fn test_subagents_config_timeout_serialize() {
    let config = SubagentsConfig {
        timeout: Some(120),
        ..Default::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"timeout\""));
    assert!(json.contains("120"));
}

#[test]
fn test_subagents_config_timeout_deserialize() {
    let json = r#"{"timeout": 120}"#;
    let config: SubagentsConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.timeout, Some(120));
}

#[test]
fn test_subagents_config_default_timeout_is_none() {
    let config = SubagentsConfig::default();
    assert!(config.timeout.is_none());
}

#[test]
fn test_subagents_config_timeout_none_skip() {
    let config = SubagentsConfig {
        timeout: None,
        ..Default::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(!json.contains("\"timeout\""));
}
