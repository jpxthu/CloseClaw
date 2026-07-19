//! Integration Tests for Plan Mode and Auto Mode
//!
//! Cross-module integration scenarios verifying end-to-end behavior of:
//! - Plan mode tool filtering (write denied → read allowed → plans/ allowed)
//! - Auto mode dangerous operation interception (high risk → approval, low risk → pass)
//!
//! All tests use `tempfile::TempDir` — no hardcoded paths, no external dependencies.

use std::collections::HashMap;
use std::sync::Arc;

use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::session_mode_query::SessionModeQuery;
use closeclaw_permission::engine::{
    Caller, Defaults, Effect, PermissionEngine, PermissionRequest, PermissionRequestBody,
    PermissionResponse,
};
use closeclaw_permission::rules::RuleSetBuilder;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Mock session mode query — maps agent IDs to modes.
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

/// Build an allow-all engine (no mode query, no rejection logger).
/// Both Agent phase defaults and User phase defaults are set to Allow,
/// so that non-Owner users with no matching rules still get Allow.
fn allow_all_engine() -> PermissionEngine {
    let permissive = Defaults {
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
    PermissionEngine::new_with_default_data_root(ruleset)
}

/// Build an allow-all engine with mode query injected.
fn allow_all_engine_with_mode(mode: SessionMode) -> PermissionEngine {
    let query: Arc<dyn SessionModeQuery> =
        Arc::new(MockModeQuery::new().with_mode("test-agent", mode));
    allow_all_engine().with_session_mode_query(query)
}

// ============================================================================
// 1. Plan mode end-to-end tool filtering
// ============================================================================

/// Plan mode: write tool to non-plans/ path → denied.
#[test]
fn test_plan_mode_e2e_write_tool_denied() {
    let engine = allow_all_engine_with_mode(SessionMode::Plan);

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );

    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "Plan mode should deny file write outside plans/, got: {:?}",
        resp
    );
}

/// Plan mode: command execution → denied.
#[test]
fn test_plan_mode_e2e_command_exec_denied() {
    let engine = allow_all_engine_with_mode(SessionMode::Plan);

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["build".to_string()],
        }),
        None,
    );

    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "Plan mode should deny command exec, got: {:?}",
        resp
    );
}

/// Plan mode: config write → denied.
#[test]
fn test_plan_mode_e2e_config_write_denied() {
    let engine = allow_all_engine_with_mode(SessionMode::Plan);

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::ConfigWrite {
            agent: "test-agent".to_string(),
            config_file: "daemon.json".to_string(),
        }),
        None,
    );

    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "Plan mode should deny config write, got: {:?}",
        resp
    );
}

/// Plan mode: file read → allowed.
#[test]
fn test_plan_mode_e2e_read_tool_allowed() {
    let engine = allow_all_engine_with_mode(SessionMode::Plan);

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/repo/src/main.rs".to_string(),
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

/// Plan mode: plans/ directory write → allowed.
#[test]
fn test_plan_mode_e2e_plans_dir_write_allowed() {
    let engine = allow_all_engine_with_mode(SessionMode::Plan);

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
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

/// Plan mode: nested plans/ directory write → allowed.
#[test]
fn test_plan_mode_e2e_nested_plans_dir_write_allowed() {
    let engine = allow_all_engine_with_mode(SessionMode::Plan);

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/workspace/plans/design.md".to_string(),
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

/// Plan mode: full end-to-end flow — agent sets Plan mode, experiences
/// write rejection, read pass, plans/ write pass, all in sequence.
#[test]
fn test_plan_mode_e2e_full_flow() {
    let engine = allow_all_engine_with_mode(SessionMode::Plan);
    let agent_id = "test-agent";

    // Step 1: Write tool to non-plans path → denied
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: agent_id.to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(matches!(resp, PermissionResponse::Denied { .. }));

    // Step 2: Read tool → allowed
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: agent_id.to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "read".to_string(),
        }),
        None,
    );
    assert!(matches!(resp, PermissionResponse::Allowed { .. }));

    // Step 3: Command exec → denied
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: agent_id.to_string(),
            cmd: "ls".to_string(),
            args: vec![],
        }),
        None,
    );
    assert!(matches!(resp, PermissionResponse::Denied { .. }));

    // Step 4: Plans/ directory write → allowed
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: agent_id.to_string(),
            path: "plans/my-plan.md".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(matches!(resp, PermissionResponse::Allowed { .. }));
}

/// Plan mode with caller metadata: full e2e flow still works.
#[test]
fn test_plan_mode_e2e_with_caller() {
    let engine = allow_all_engine_with_mode(SessionMode::Plan);

    // Denied: write outside plans/
    let resp = engine.evaluate(
        PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "alice".to_string(),
                agent: "test-agent".to_string(),
                creator_id: String::new(),
            },
            request: PermissionRequestBody::FileOp {
                agent: "test-agent".to_string(),
                path: "/src/main.rs".to_string(),
                op: "write".to_string(),
            },
        },
        None,
    );
    assert!(matches!(resp, PermissionResponse::Denied { .. }));

    // Allowed: plans/ write
    let resp = engine.evaluate(
        PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "alice".to_string(),
                agent: "test-agent".to_string(),
                creator_id: String::new(),
            },
            request: PermissionRequestBody::FileOp {
                agent: "test-agent".to_string(),
                path: "plans/design.md".to_string(),
                op: "write".to_string(),
            },
        },
        None,
    );
    assert!(matches!(resp, PermissionResponse::Allowed { .. }));

    // Allowed: read
    let resp = engine.evaluate(
        PermissionRequest::WithCaller {
            caller: Caller {
                user_id: "alice".to_string(),
                agent: "test-agent".to_string(),
                creator_id: String::new(),
            },
            request: PermissionRequestBody::FileOp {
                agent: "test-agent".to_string(),
                path: "/src/main.rs".to_string(),
                op: "read".to_string(),
            },
        },
        None,
    );
    assert!(matches!(resp, PermissionResponse::Allowed { .. }));
}

// ============================================================================
// 2. Auto mode dangerous operation interception
// ============================================================================

/// Auto mode: low risk file write → allowed (direct pass).
#[test]
fn test_auto_mode_e2e_low_risk_allowed() {
    let engine = allow_all_engine_with_mode(SessionMode::Auto);

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );

    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Auto mode + low risk should pass directly, got: {:?}",
        resp
    );
}

/// Auto mode: low risk command exec → allowed.
#[test]
fn test_auto_mode_e2e_low_risk_command_allowed() {
    let engine = allow_all_engine_with_mode(SessionMode::Auto);

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "cargo".to_string(),
            args: vec!["build".to_string()],
        }),
        None,
    );

    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Auto mode + low risk command should pass, got: {:?}",
        resp
    );
}

/// Auto mode: high risk (git path write) → denied.
/// High risk operations in Auto Mode trigger Deny to start the approval
/// workflow, consistent with design doc: "dangerous operations still
/// require user approval".
#[test]
fn test_auto_mode_e2e_high_risk_git_path_denied() {
    let engine = allow_all_engine_with_mode(SessionMode::Auto);

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/repo/.git/config".to_string(),
            op: "write".to_string(),
        }),
        None,
    );

    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "Auto mode + high risk (git path) should be denied, got: {:?}",
        resp
    );
}

/// Auto mode: high risk (bare rm -rf) → denied.
/// High risk operations in Auto Mode trigger Deny to start the approval
/// workflow, consistent with design doc: "dangerous operations still
/// require user approval".
#[test]
fn test_auto_mode_e2e_high_risk_bare_rm_rf_denied() {
    let engine = allow_all_engine_with_mode(SessionMode::Auto);

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "test-agent".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string()],
        }),
        None,
    );

    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "Auto mode + high risk (rm -rf) should be denied, got: {:?}",
        resp
    );
}

/// Auto mode: critical risk (permissions.json write) → denied.
/// Critical risk operations in Auto Mode trigger Deny to start the approval
/// workflow, consistent with design doc: "dangerous operations still
/// require user approval".
#[test]
fn test_auto_mode_e2e_critical_risk_permissions_json_denied() {
    let engine = allow_all_engine_with_mode(SessionMode::Auto);

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/repo/permissions.json".to_string(),
            op: "write".to_string(),
        }),
        None,
    );

    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "Auto mode + critical risk (permissions.json) should be denied, got: {:?}",
        resp
    );
}

/// Auto mode: critical risk (daemon config write) → allowed
/// (risk gate removed; only Deny triggers approval per design doc).
#[test]
fn test_auto_mode_e2e_critical_risk_daemon_config_allowed() {
    let engine = allow_all_engine_with_mode(SessionMode::Auto);

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::ConfigWrite {
            agent: "test-agent".to_string(),
            config_file: "daemon.json".to_string(),
        }),
        None,
    );

    // ConfigWrite is always high-risk (design doc: single approval only, no whitelist).
    // Even auto mode + allow defaults must be intercepted by the config_write guard.
    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "Auto mode + critical risk (daemon config) should be denied (config write guard), got: {:?}",
        resp
    );
}

/// Auto mode: full e2e flow — low risk passes, high/critical risk denied
/// to start approval workflow, consistent with design doc.
#[test]
fn test_auto_mode_e2e_full_flow() {
    let engine = allow_all_engine_with_mode(SessionMode::Auto);
    let agent_id = "test-agent";

    // Step 1: Low risk write → allowed
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: agent_id.to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(matches!(resp, PermissionResponse::Allowed { .. }));

    // Step 2: High risk (git path read) → denied (.git = high risk)
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: agent_id.to_string(),
            path: "/repo/.git/HEAD".to_string(),
            op: "read".to_string(),
        }),
        None,
    );
    assert!(matches!(resp, PermissionResponse::Denied { .. }));

    // Step 3: Critical risk (permissions.json write) → denied
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: agent_id.to_string(),
            path: "/etc/permissions.json".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(matches!(resp, PermissionResponse::Denied { .. }));

    // Step 4: Low risk command → allowed again
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: agent_id.to_string(),
            cmd: "git".to_string(),
            args: vec!["status".to_string()],
        }),
        None,
    );
    assert!(matches!(resp, PermissionResponse::Allowed { .. }));
}

/// Normal mode: high risk operations NOT gated (no approval needed).
#[test]
fn test_normal_mode_e2e_high_risk_not_gated() {
    let engine = allow_all_engine_with_mode(SessionMode::Normal);

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
            path: "/repo/.git/config".to_string(),
            op: "write".to_string(),
        }),
        None,
    );

    assert!(
        matches!(resp, PermissionResponse::Allowed { .. }),
        "Normal mode should not gate high risk operations, got: {:?}",
        resp
    );
}

/// Plan mode: Plan mode filtering denies write operations.
#[test]
fn test_plan_mode_e2e_write_denied() {
    let engine = allow_all_engine_with_mode(SessionMode::Plan);

    // .git write is denied by Plan mode filter
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "test-agent".to_string(),
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
            "Plan mode should deny .git write via plan mode filter, got: {:?}",
            other
        ),
    }
}
