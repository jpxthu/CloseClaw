use super::engine_eval::PermissionEngine;
use super::engine_risk::RiskLevel;
use super::engine_types::{
    Caller, Effect, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use crate::rules::RuleSetBuilder;
use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::session_mode_query::SessionModeQuery;
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Mock SessionModeQuery
// ---------------------------------------------------------------------------

struct MockModeQuery {
    modes: HashMap<String, SessionMode>,
}

impl MockModeQuery {
    fn new() -> Self {
        Self {
            modes: HashMap::new(),
        }
    }

    fn with_mode(mut self, agent_id: &str, mode: SessionMode) -> Self {
        self.modes.insert(agent_id.to_string(), mode);
        self
    }
}

impl SessionModeQuery for MockModeQuery {
    fn get_session_mode(&self, agent_id: &str) -> Option<SessionMode> {
        self.modes.get(agent_id).copied()
    }
}

fn make_permissive_engine(query: Arc<dyn SessionModeQuery>) -> PermissionEngine {
    let ruleset = RuleSetBuilder::new()
        .default_file(Effect::Allow)
        .default_command(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_config(Effect::Allow)
        .build()
        .unwrap();
    PermissionEngine::new_with_default_data_root(ruleset).with_session_mode_query(query)
}

// ---------------------------------------------------------------------------
// Normal mode: no filtering
// ---------------------------------------------------------------------------

#[test]
fn test_normal_mode_file_write_allowed() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Normal));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Normal mode should allow file write, got: {:?}",
        resp
    );
}

#[test]
fn test_normal_mode_command_exec_allowed() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Normal));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "a".to_string(),
            cmd: "ls".to_string(),
            args: vec![],
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Normal mode should allow command exec, got: {:?}",
        resp
    );
}

#[test]
fn test_normal_mode_config_write_allowed() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Normal));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::ConfigWrite {
            agent: "a".to_string(),
            config_file: "config.json".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Normal mode should allow config write, got: {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// Plan mode: write operations denied
// ---------------------------------------------------------------------------

#[test]
fn test_plan_mode_file_write_denied() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Plan));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    match resp {
        PermissionResponse::Denied { reason, rule, .. } => {
            assert!(reason.contains("Plan mode"), "reason: {}", reason);
            assert_eq!(rule, "<plan_mode_filter>");
        }
        other => panic!("expected Denied for Plan mode file write, got: {:?}", other),
    }
}

#[test]
fn test_plan_mode_command_exec_denied() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Plan));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "a".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["build".to_string()],
        }),
        None,
    );
    match resp {
        PermissionResponse::Denied { reason, rule, .. } => {
            assert!(reason.contains("Plan mode"), "reason: {}", reason);
            assert_eq!(rule, "<plan_mode_filter>");
        }
        other => panic!(
            "expected Denied for Plan mode command exec, got: {:?}",
            other
        ),
    }
}

#[test]
fn test_plan_mode_config_write_denied() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Plan));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::ConfigWrite {
            agent: "a".to_string(),
            config_file: "config.json".to_string(),
        }),
        None,
    );
    match resp {
        PermissionResponse::Denied { reason, rule, .. } => {
            assert!(reason.contains("Plan mode"), "reason: {}", reason);
            assert_eq!(rule, "<plan_mode_filter>");
        }
        other => panic!(
            "expected Denied for Plan mode config write, got: {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Plan mode: plans/ directory write allowed
// ---------------------------------------------------------------------------

#[test]
fn test_plan_mode_file_write_plans_dir_allowed() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Plan));
    let engine = make_permissive_engine(query);

    // Path starts with "plans/"
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "plans/2026-07-08-feature.md".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Plan mode should allow write to plans/ dir, got: {:?}",
        resp
    );
}

#[test]
fn test_plan_mode_file_write_nested_plans_dir_allowed() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Plan));
    let engine = make_permissive_engine(query);

    // Path contains "/plans/"
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/workspace/plans/feature.md".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Plan mode should allow write to nested plans/ dir, got: {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// Plan mode: read operations allowed
// ---------------------------------------------------------------------------

#[test]
fn test_plan_mode_file_read_allowed() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Plan));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/src/main.rs".to_string(),
            op: "read".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Plan mode should allow file read, got: {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// Plan mode: other request types not affected
// ---------------------------------------------------------------------------

#[test]
fn test_plan_mode_net_op_affected() {
    // NetOp is NOT one of the filtered types (only FileOp write, CommandExec, ConfigWrite)
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Plan));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::NetOp {
            agent: "a".to_string(),
            host: "example.com".to_string(),
            port: 443,
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Plan mode should not filter NetOp, got: {:?}",
        resp
    );
}

#[test]
fn test_plan_mode_inter_agent_msg_affected() {
    // InterAgentMsg is NOT one of the filtered types
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Plan));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::InterAgentMsg {
            from: "a".to_string(),
            to: "b".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Plan mode should not filter InterAgentMsg, got: {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// Auto mode: low risk → direct pass
// ---------------------------------------------------------------------------

#[test]
fn test_auto_mode_low_risk_file_write_allowed() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Auto));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Auto mode + low risk should allow file write, got: {:?}",
        resp
    );
}

#[test]
fn test_auto_mode_low_risk_command_exec_allowed() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Auto));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "a".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["build".to_string()],
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Auto mode + low risk should allow command exec, got: {:?}",
        resp
    );
}

#[test]
fn test_auto_mode_low_risk_config_write_allowed() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Auto));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::ConfigWrite {
            agent: "a".to_string(),
            config_file: "config.json".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Auto mode + low risk should allow config write, got: {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// Auto mode: high risk → approval required
// ---------------------------------------------------------------------------

#[test]
fn test_auto_mode_high_risk_git_path_requires_approval() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Auto));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/repo/.git/config".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    match resp {
        PermissionResponse::ApprovalRequired {
            risk_level,
            rule,
            operation_desc,
        } => {
            assert_eq!(risk_level, RiskLevel::High);
            assert_eq!(rule, "<auto_mode_risk_gate>");
            assert!(operation_desc.contains(".git"));
        }
        other => panic!(
            "Auto mode + high risk should require approval, got: {:?}",
            other
        ),
    }
}

#[test]
fn test_auto_mode_high_risk_bare_rm_rf_requires_approval() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Auto));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "a".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string()],
        }),
        None,
    );
    match resp {
        PermissionResponse::ApprovalRequired {
            risk_level, rule, ..
        } => {
            assert_eq!(risk_level, RiskLevel::High);
            assert_eq!(rule, "<auto_mode_risk_gate>");
        }
        other => panic!(
            "Auto mode + high risk (rm -rf) should require approval, got: {:?}",
            other
        ),
    }
}

#[test]
fn test_auto_mode_critical_risk_permissions_json_requires_approval() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Auto));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/repo/permissions.json".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    match resp {
        PermissionResponse::ApprovalRequired {
            risk_level, rule, ..
        } => {
            assert_eq!(risk_level, RiskLevel::Critical);
            assert_eq!(rule, "<auto_mode_risk_gate>");
        }
        other => panic!(
            "Auto mode + critical risk should require approval, got: {:?}",
            other
        ),
    }
}

#[test]
fn test_auto_mode_critical_risk_daemon_config_requires_approval() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Auto));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::ConfigWrite {
            agent: "a".to_string(),
            config_file: "daemon.json".to_string(),
        }),
        None,
    );
    match resp {
        PermissionResponse::ApprovalRequired {
            risk_level, rule, ..
        } => {
            assert_eq!(risk_level, RiskLevel::Critical);
            assert_eq!(rule, "<auto_mode_risk_gate>");
        }
        other => panic!(
            "Auto mode + critical risk (daemon config) should require approval, got: {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Normal mode: not affected by risk gate
// ---------------------------------------------------------------------------

#[test]
fn test_normal_mode_high_risk_not_gated() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Normal));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/repo/.git/config".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Normal mode + high risk should not require approval, got: {:?}",
        resp
    );
}

#[test]
fn test_normal_mode_critical_risk_not_gated() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Normal));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/repo/permissions.json".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Normal mode + critical risk should not require approval, got: {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// Plan mode: not affected by risk gate (Plan mode filtering is prior)
// ---------------------------------------------------------------------------

#[test]
fn test_plan_mode_high_risk_not_gated() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Plan));
    let engine = make_permissive_engine(query);
    // .git write is denied by Plan mode filter (write not in plans/), not by risk gate
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/repo/.git/config".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    match resp {
        PermissionResponse::Denied { rule, .. } => {
            assert_eq!(rule, "<plan_mode_filter>");
        }
        other => panic!(
            "Plan mode write should be denied by plan mode filter, got: {:?}",
            other
        ),
    }
}

#[test]
fn test_plan_mode_critical_risk_read_not_gated() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Plan));
    let engine = make_permissive_engine(query);
    // Read in Plan mode is allowed — risk gate doesn't apply
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/repo/permissions.json".to_string(),
            op: "read".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Plan mode + critical risk read should not require approval, got: {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// No query set: no filtering
// ---------------------------------------------------------------------------

#[test]
fn test_no_query_file_write_allowed() {
    let ruleset = RuleSetBuilder::new()
        .default_file(Effect::Allow)
        .default_command(Effect::Allow)
        .default_config(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Without query, no filtering should occur, got: {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// Unknown agent: no filtering (query returns None)
// ---------------------------------------------------------------------------

#[test]
fn test_plan_mode_unknown_agent_no_filtering() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Plan));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "unknown".to_string(),
            path: "/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Unknown agent should not be filtered, got: {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// WithCaller: Plan mode filtering works through the full path
// ---------------------------------------------------------------------------

#[test]
fn test_plan_mode_with_caller_file_write_denied() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Plan));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "alice".to_string(),
                agent: "a".to_string(),
                creator_id: String::new(),
            },
            request: PermissionRequestBody::FileOp {
                agent: "a".to_string(),
                path: "/src/main.rs".to_string(),
                op: "write".to_string(),
            },
        },
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "Plan mode + WithCaller should deny file write, got: {:?}",
        resp
    );
}

#[test]
fn test_plan_mode_with_caller_plans_allowed() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Plan));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "alice".to_string(),
                agent: "a".to_string(),
                creator_id: String::new(),
            },
            request: PermissionRequestBody::FileOp {
                agent: "a".to_string(),
                path: "plans/my-plan.md".to_string(),
                op: "write".to_string(),
            },
        },
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Plan mode + WithCaller should allow plans/ write, got: {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// plans/ path edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_plan_mode_plans_path_not_prefix() {
    // "some-plans-dir" should NOT match (not "plans/" prefix and no "/plans/")
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Plan));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "some-plans-dir/file.md".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "Path without plans/ pattern should be denied, got: {:?}",
        resp
    );
}

#[test]
fn test_plan_mode_plans_path_absolute() {
    // Absolute path with /plans/ should match
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Plan));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/home/user/workspace/plans/design.md".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Absolute path with /plans/ should be allowed, got: {:?}",
        resp
    );
}
