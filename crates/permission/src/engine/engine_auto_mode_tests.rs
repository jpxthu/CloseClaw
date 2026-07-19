//! Auto Mode runtime review tests.
//!
//! Separate from engine_plan_mode_tests.rs due to the 1000-line file limit.
//! Covers all behavioral dimensions of `check_auto_mode_filter` as specified
//! in the auto-mode-review design doc.

use super::engine_eval::PermissionEngine;
use super::engine_types::{
    Caller, Effect, MessageDirection, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use crate::rules::RuleSetBuilder;
use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::session_mode_query::SessionModeQuery;
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Mock SessionModeQuery (duplicated here for file independence)
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
    let permissive = super::engine_types::Defaults {
        file_read: Effect::Allow,
        file_write: Effect::Allow,
        command: Effect::Allow,
        network: Effect::Allow,
        inter_agent: Effect::Allow,
        config: Effect::Allow,
        tool_call: Effect::Allow,
        message: Effect::Allow,
    };
    let ruleset = RuleSetBuilder::new()
        .defaults(permissive.clone())
        .user_defaults(permissive)
        .build()
        .unwrap();
    PermissionEngine::new_with_default_data_root(ruleset).with_session_mode_query(query)
}

// ---------------------------------------------------------------------------
// Normal path: Auto Mode + low risk → Allowed
// ---------------------------------------------------------------------------

#[test]
fn test_auto_low_risk_file_write_allowed() {
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
        "Auto mode + low risk file write should be Allowed, got: {:?}",
        resp
    );
}

#[test]
fn test_auto_low_risk_command_exec_allowed() {
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
        "Auto mode + low risk command exec should be Allowed, got: {:?}",
        resp
    );
}

#[test]
fn test_auto_low_risk_file_read_allowed() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Auto));
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
        "Auto mode + low risk file read should be Allowed, got: {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// Dangerous path: Auto Mode + High/Critical risk → Denied
// ---------------------------------------------------------------------------

#[test]
fn test_auto_high_risk_git_path_denied() {
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
        PermissionResponse::Denied { reason, rule, .. } => {
            assert!(reason.contains("Auto Mode"), "reason: {}", reason);
            assert_eq!(rule, "<auto_mode_filter>");
        }
        other => panic!(
            "expected Denied for Auto mode + .git path write, got: {:?}",
            other
        ),
    }
}

#[test]
fn test_auto_high_risk_git_path_read_denied() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Auto));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/repo/.git/HEAD".to_string(),
            op: "read".to_string(),
        }),
        None,
    );
    match resp {
        PermissionResponse::Denied { reason, rule, .. } => {
            assert!(reason.contains("Auto Mode"), "reason: {}", reason);
            assert_eq!(rule, "<auto_mode_filter>");
        }
        other => panic!(
            "expected Denied for Auto mode + .git path read, got: {:?}",
            other
        ),
    }
}

#[test]
fn test_auto_high_risk_rm_rf_denied() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Auto));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "a".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string(), "/tmp/foo".to_string()],
        }),
        None,
    );
    match resp {
        PermissionResponse::Denied { reason, rule, .. } => {
            assert!(reason.contains("Auto Mode"), "reason: {}", reason);
            assert_eq!(rule, "<auto_mode_filter>");
        }
        other => panic!("expected Denied for Auto mode + rm -rf, got: {:?}", other),
    }
}

#[test]
fn test_auto_critical_risk_permissions_json_denied() {
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
        PermissionResponse::Denied { reason, rule, .. } => {
            assert!(reason.contains("Auto Mode"), "reason: {}", reason);
            assert_eq!(rule, "<auto_mode_filter>");
        }
        other => panic!(
            "expected Denied for Auto mode + permissions.json, got: {:?}",
            other
        ),
    }
}

#[test]
fn test_auto_critical_risk_config_write_denied() {
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
        PermissionResponse::Denied { reason, rule, .. } => {
            assert!(
                reason.contains("Auto Mode") || reason.contains("config write"),
                "reason: {}",
                reason
            );
            assert!(
                rule == "<auto_mode_filter>" || rule == "<config_write_default_guard>",
                "rule: {}",
                rule
            );
        }
        other => panic!(
            "expected Denied for Auto mode + ConfigWrite, got: {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// MessageSend: Auto Mode + Send → Denied; Receive → not blocked
// ---------------------------------------------------------------------------

#[test]
fn test_auto_message_send_denied() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Auto));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::MessageSend {
            agent: "a".to_string(),
            direction: MessageDirection::Send,
            target: "chat-id-123".to_string(),
        }),
        None,
    );
    match resp {
        PermissionResponse::Denied { reason, rule, .. } => {
            assert!(reason.contains("Auto Mode"), "reason: {}", reason);
            assert!(reason.contains("message"), "reason: {}", reason);
            assert_eq!(rule, "<auto_mode_filter>");
        }
        other => panic!(
            "expected Denied for Auto mode + MessageSend(Send), got: {:?}",
            other
        ),
    }
}

#[test]
fn test_auto_message_receive_not_blocked() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Auto));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::MessageSend {
            agent: "a".to_string(),
            direction: MessageDirection::Receive,
            target: "chat-id-123".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Auto mode + MessageSend(Receive) should be Allowed, got: {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// Owner exemption: Owner + Auto Mode + high risk → Allowed
// ---------------------------------------------------------------------------

#[test]
fn test_owner_auto_mode_high_risk_allowed() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Auto));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "owner".to_string(),
                agent: "a".to_string(),
                creator_id: String::new(),
            },
            request: PermissionRequestBody::FileOp {
                agent: "a".to_string(),
                path: "/repo/.git/config".to_string(),
                op: "write".to_string(),
            },
        },
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Owner + Auto mode + high risk should be Allowed (owner exemption), got: {:?}",
        resp
    );
}

#[test]
fn test_owner_auto_mode_message_send_allowed() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Auto));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "owner".to_string(),
                agent: "a".to_string(),
                creator_id: String::new(),
            },
            request: PermissionRequestBody::MessageSend {
                agent: "a".to_string(),
                direction: MessageDirection::Send,
                target: "chat-id-123".to_string(),
            },
        },
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Owner + Auto mode + MessageSend should be Allowed (owner exemption), got: {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// Mode irrelevant: Normal Mode + high risk → not affected by auto mode filter
// ---------------------------------------------------------------------------

#[test]
fn test_normal_mode_high_risk_not_blocked() {
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
        "Normal mode + high risk should not be blocked by auto mode filter, got: {:?}",
        resp
    );
}

#[test]
fn test_normal_mode_message_send_not_blocked() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Normal));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::MessageSend {
            agent: "a".to_string(),
            direction: MessageDirection::Send,
            target: "chat-id-123".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Normal mode + MessageSend should not be blocked by auto mode filter, got: {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// No query: session_mode_query not set → no filtering
// ---------------------------------------------------------------------------

#[test]
fn test_no_query_high_risk_not_blocked() {
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Allow)
        .default_file_write(Effect::Allow)
        .default_command(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_config(Effect::Allow)
        .default_tool_call(Effect::Allow)
        .default_message(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);
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
        "Without session_mode_query, high risk should not be blocked, got: {:?}",
        resp
    );
}

#[test]
fn test_no_query_message_send_not_blocked() {
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Allow)
        .default_file_write(Effect::Allow)
        .default_command(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_config(Effect::Allow)
        .default_tool_call(Effect::Allow)
        .default_message(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::MessageSend {
            agent: "a".to_string(),
            direction: MessageDirection::Send,
            target: "chat-id-123".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Without session_mode_query, MessageSend should not be blocked, got: {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// Unknown agent: query returns None → no filtering
// ---------------------------------------------------------------------------

#[test]
fn test_auto_mode_unknown_agent_not_blocked() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Auto));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "unknown-agent".to_string(),
            path: "/repo/.git/config".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Unknown agent should not be blocked by auto mode filter, got: {:?}",
        resp
    );
}

// ---------------------------------------------------------------------------
// Plan Mode interaction: Plan mode filter takes priority over auto mode filter
// ---------------------------------------------------------------------------

#[test]
fn test_plan_mode_high_risk_denied_by_plan_filter() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Plan));
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
        PermissionResponse::Denied { rule, .. } => {
            assert_eq!(
                rule, "<plan_mode_filter>",
                "Plan mode should deny via plan_mode_filter, not auto_mode_filter"
            );
        }
        other => panic!("expected Denied by plan_mode_filter, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Denial reason content validation
// ---------------------------------------------------------------------------

#[test]
fn test_auto_denial_reason_contains_approval_hint() {
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
        PermissionResponse::Denied { reason, .. } => {
            assert!(
                reason.contains("approval") || reason.contains("requires"),
                "denial reason should hint at approval requirement, got: {}",
                reason
            );
        }
        other => panic!("expected Denied, got: {:?}", other),
    }
}

#[test]
fn test_auto_message_send_denial_reason_mentions_message() {
    let query = Arc::new(MockModeQuery::new().with_mode("a", SessionMode::Auto));
    let engine = make_permissive_engine(query);
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::MessageSend {
            agent: "a".to_string(),
            direction: MessageDirection::Send,
            target: "chat-id-123".to_string(),
        }),
        None,
    );
    match resp {
        PermissionResponse::Denied { reason, .. } => {
            assert!(
                reason.contains("message") || reason.contains("Message"),
                "denial reason should mention message sending, got: {}",
                reason
            );
        }
        other => panic!("expected Denied, got: {:?}", other),
    }
}
