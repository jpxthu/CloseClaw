//! Sub-agent session behavior tests for the permission port.
//!
//! Split from `permission_check_tests.rs` (which itself is compiled via
//! `#[path = ...]` inside `permission_check.rs`'s `#[cfg(test)] mod
//! tests`) to keep both files within the 1000-line limit.

use super::tests::{make_af, make_cm, make_ctx, make_engine_with_rules, make_port, make_sm};
use super::*;
use crate::{ToolCallError, ToolContext};
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_types::Rule;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

type ApprovalMutex = TokioMutex<ApprovalFlow>;

// ---------------------------------------------------------------------------
// is_sub_agent behavior tests
// ---------------------------------------------------------------------------

/// Insert a root session (depth=0) and a child session (depth=1) into the
/// SessionManager so that `is_session_sub_agent` can resolve depths.
async fn setup_sessions_with_depth(sm: &SessionManager, root_id: &str, child_id: &str) {
    use closeclaw_session::llm_session::ConversationSession;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    // Insert root session (depth=0)
    sm.sessions.write().await.insert(
        root_id.to_string(),
        closeclaw_gateway::Session {
            id: root_id.to_string(),
            agent_id: "agent-a".to_string(),
            channel: "test".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
    let cs_root = ConversationSession::new(
        root_id.to_string(),
        "test-model".to_string(),
        std::env::temp_dir(),
    );
    sm.conversation_sessions
        .write()
        .await
        .insert(root_id.to_string(), Arc::new(RwLock::new(cs_root)));

    // Insert child session (depth=1)
    sm.sessions.write().await.insert(
        child_id.to_string(),
        closeclaw_gateway::Session {
            id: child_id.to_string(),
            agent_id: "child-agent".to_string(),
            channel: "test".to_string(),
            created_at: 0,
            depth: 1,
        },
    );
    let cs_child = ConversationSession::new(
        child_id.to_string(),
        "test-model".to_string(),
        std::env::temp_dir(),
    );
    sm.conversation_sessions
        .write()
        .await
        .insert(child_id.to_string(), Arc::new(RwLock::new(cs_child)));
}

/// Build PermDeps using a shared SessionManager so that session state
/// set up via `setup_sessions_with_depth` is visible to the permission
/// check functions.
fn make_deps_with_sm(
    rules: Vec<Rule>,
    sm: Arc<SessionManager>,
    flow: Arc<ApprovalMutex>,
) -> PermDeps {
    make_port(make_engine_with_rules(rules), sm, make_cm(), flow)
}

/// Root session (depth=0) denial routes through approval flow → returns
/// a ToolResult with approval-pending status, NOT PermissionDenied.
#[tokio::test]
async fn test_root_session_denial_goes_through_approval_flow() {
    let sm = make_sm();
    setup_sessions_with_depth(&sm, "root-session", "child-session").await;
    let flow = make_af();
    let deps = make_deps_with_sm(vec![], sm, flow);

    let ctx = ToolContext {
        agent_id: "agent-a".to_string(),
        workdir: None,
        session_id: Some("root-session".to_string()),
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    };
    let result = check_tool_permission(&deps, &ctx, "bash", "call", None).await;
    // Root session → is_sub_agent=false → approval flow enqueues → Ok(Some(...))
    assert!(
        result.is_ok(),
        "Root session denial should route to approval flow"
    );
    assert!(
        result.unwrap().is_some(),
        "Root session should get approval-pending result"
    );
}

/// Child session (depth=1) denial is silent → returns
/// PermissionDenied (approval flow returns None for sub-agents).
#[tokio::test]
async fn test_child_session_denial_is_silent() {
    let sm = make_sm();
    setup_sessions_with_depth(&sm, "root-session", "child-session").await;
    let flow = make_af();
    let deps = make_deps_with_sm(vec![], sm, flow);

    let ctx = ToolContext {
        agent_id: "agent-a".to_string(),
        workdir: None,
        session_id: Some("child-session".to_string()),
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    };
    let result = check_tool_permission(&deps, &ctx, "bash", "call", None).await;
    // Child session → is_sub_agent=true → silent deny → PermissionDenied
    match result {
        Err(ToolCallError::PermissionDenied(reason)) => {
            assert!(!reason.is_empty());
        }
        other => panic!("Child session should get PermissionDenied, got {:?}", other),
    }
}

/// session_id=None → is_sub_agent=false → routes through approval flow.
#[tokio::test]
async fn test_none_session_id_not_sub_agent() {
    let sm = make_sm();
    let flow = make_af();
    let deps = make_deps_with_sm(vec![], sm, flow);
    let ctx = make_ctx("agent-a");
    // ctx.session_id is None by default from make_ctx
    let result = check_tool_permission(&deps, &ctx, "bash", "call", None).await;
    // None session → is_sub_agent=false → approval flow enqueues
    assert!(
        result.is_ok(),
        "None session_id should route to approval flow"
    );
    assert!(result.unwrap().is_some());
}

/// session_id="" → is_sub_agent=false → routes through approval flow.
#[tokio::test]
async fn test_empty_session_id_not_sub_agent() {
    let sm = make_sm();
    let flow = make_af();
    let deps = make_deps_with_sm(vec![], sm, flow);
    let ctx = ToolContext {
        agent_id: "agent-a".to_string(),
        workdir: None,
        session_id: Some(String::new()),
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    };
    let result = check_tool_permission(&deps, &ctx, "bash", "call", None).await;
    // Empty session_id → is_sub_agent=false → approval flow enqueues
    assert!(
        result.is_ok(),
        "Empty session_id should route to approval flow"
    );
    assert!(result.unwrap().is_some());
}

/// Child session (depth=1) denial for file op is also silent.
#[tokio::test]
async fn test_child_session_file_op_denial_is_silent() {
    let sm = make_sm();
    setup_sessions_with_depth(&sm, "root-session", "child-session").await;
    let flow = make_af();
    let deps = make_deps_with_sm(vec![], sm, flow);

    let ctx = ToolContext {
        agent_id: "agent-a".to_string(),
        workdir: None,
        session_id: Some("child-session".to_string()),
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    };
    let result = check_file_op_permission(&deps, &ctx, "/tmp/test.txt", "read", None).await;
    // Child session → silent deny → PermissionDenied
    assert!(
        result.is_err(),
        "Child session file op should be silently denied"
    );
}

/// Child session (depth=1) command denial is also silent.
#[tokio::test]
async fn test_child_session_command_denial_is_silent() {
    let sm = make_sm();
    setup_sessions_with_depth(&sm, "root-session", "child-session").await;
    let flow = make_af();
    let deps = make_deps_with_sm(vec![], sm, flow);

    let ctx = ToolContext {
        agent_id: "agent-a".to_string(),
        workdir: None,
        session_id: Some("child-session".to_string()),
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    };
    let result = check_command_permission(&deps, &ctx, "ls", &["-la".to_string()], None).await;
    // Child session → silent deny → Denied variant
    assert!(
        matches!(result, CommandPermissionResult::Denied(_)),
        "Child session command should be silently denied"
    );
}

// ---------------------------------------------------------------------------
// is_session_sub_agent direct regression tests
// ---------------------------------------------------------------------------

/// Build a port over the given session manager (engine/flow unused by
/// the sub-agent decision).
fn sub_agent_port(sm: Arc<SessionManager>) -> PermDeps {
    make_port(make_engine_with_rules(vec![]), sm, make_cm(), make_af())
}

/// is_session_sub_agent returns false for empty session_id.
/// (Non-regression: this behavior was already covered by
/// test_empty_session_id_not_sub_agent above.)
#[tokio::test]
async fn test_is_session_sub_agent_empty_session_id() {
    let port = sub_agent_port(make_sm());
    assert!(!port.is_session_sub_agent("").await);
}

/// is_session_sub_agent returns false for a non-existent session_id.
/// get_session_depth returns None → is_some_and evaluates to false.
#[tokio::test]
async fn test_is_session_sub_agent_nonexistent_session() {
    let port = sub_agent_port(make_sm());
    assert!(!port.is_session_sub_agent("does-not-exist").await);
}

/// is_session_sub_agent returns false for a root session (depth=0).
#[tokio::test]
async fn test_is_session_sub_agent_depth_zero() {
    let sm = make_sm();
    setup_sessions_with_depth(&sm, "root-0", "child-0").await;
    let port = sub_agent_port(sm);
    assert!(!port.is_session_sub_agent("root-0").await);
}

/// is_session_sub_agent returns true for a child session (depth=1).
#[tokio::test]
async fn test_is_session_sub_agent_depth_positive() {
    let sm = make_sm();
    setup_sessions_with_depth(&sm, "root-1", "child-1").await;
    let port = sub_agent_port(sm);
    assert!(port.is_session_sub_agent("child-1").await);
}
