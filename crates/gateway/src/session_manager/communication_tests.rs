//! Tests for communication permission checks (Step 1.4).
//!
//! Covers:
//! 1. Default config (parent-only) → parent↔child allowed, child↔sibling denied
//! 2. `outbound: ["*"]` → allows sending to any agent
//! 3. `inbound: ["*"]` → allows receiving from any agent
//! 4. Both parties restricted → only parent↔child allowed
//! 5. steer denied by communication check returns correct error
//! 6. announce denied by communication check is handled gracefully

use super::communication::{
    check_communication_allowed, CommunicationCheckResult, CommunicationConfig, CommunicationError,
};
use super::spawn::{ChildSessionInfo, SpawnMode};
use super::tests::{clear_global_prompt_state, make_test_mgr};
use super::SessionManager;
use closeclaw_llm::session::ConversationSession;
use closeclaw_session::bootstrap::BootstrapMode;
use serial_test::serial;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

// ── Helper to build ResolvedAgentConfig ────────────────────────────────────

#[allow(dead_code)]
fn test_resolved_config(
    id: &str,
    workspace: Option<PathBuf>,
) -> closeclaw_config::agents::ResolvedAgentConfig {
    use closeclaw_common::agent_lookup::config::SubagentsConfig;
    use closeclaw_config::agents::ConfigSource;

    closeclaw_config::agents::ResolvedAgentConfig {
        id: id.to_string(),
        name: id.to_string(),
        parent_id: None,
        model: Some("test-model".to_string()),
        workspace,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills: vec![],
        tools: vec![],
        disallowed_tools: vec![],
        subagents: SubagentsConfig::default(),
        memory: None,
        source: ConfigSource::Merged,
    }
}

/// Register a parent session with a ConversationSession so child
/// creation can wire into the cancel token tree.
async fn register_parent_session(
    mgr: &SessionManager,
    parent_id: &str,
    agent_id: &str,
    workdir: &std::path::Path,
) {
    use crate::Session;

    mgr.sessions.write().await.insert(
        parent_id.to_string(),
        Session {
            id: parent_id.to_string(),
            agent_id: agent_id.to_string(),
            channel: "spawn".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
    let cs = Arc::new(RwLock::new(ConversationSession::new(
        parent_id.to_string(),
        "test-model".to_string(),
        workdir.to_path_buf(),
    )));
    mgr.conversation_sessions
        .write()
        .await
        .insert(parent_id.to_string(), cs);
}

/// Register a child session directly (no full create_child_session) with a
/// custom CommunicationConfig for targeted unit tests.
async fn register_child_session(
    mgr: &SessionManager,
    session_id: &str,
    agent_id: &str,
    config: CommunicationConfig,
) {
    use crate::Session;

    let mut cs = ConversationSession::new(
        session_id.to_string(),
        "test-model".to_string(),
        PathBuf::from("/tmp"),
    );
    cs = cs.with_communication_config(config);
    let arc = Arc::new(RwLock::new(cs));

    mgr.sessions.write().await.insert(
        session_id.to_string(),
        Session {
            id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            channel: "spawn".to_string(),
            created_at: 0,
            depth: 1,
        },
    );
    mgr.conversation_sessions
        .write()
        .await
        .insert(session_id.to_string(), arc);
}

// ── Pure function tests ────────────────────────────────────────────────────

/// 1. Default config (parent-only): parent (allow-all) ↔ child allowed,
///    child → sibling denied.
#[test]
fn test_check_communication_allowed_default_parent_only() {
    // In production, parent/orchestrator sessions have no CommunicationConfig,
    // which defaults to allow-all. Child sessions get default_with_parent.
    let parent_config = CommunicationConfig {
        outbound: vec!["*".to_string()],
        inbound: vec!["*".to_string()],
    };
    let child_config = CommunicationConfig::default_with_parent(Some("parent-agent"));

    // Parent → child: parent outbound is "*" → allowed.
    let result =
        check_communication_allowed(&parent_config, "parent-agent", &child_config, "child-agent");
    assert_eq!(result, CommunicationCheckResult::Allowed);

    // Child → parent: child outbound includes "parent-agent" → allowed.
    let result =
        check_communication_allowed(&child_config, "child-agent", &parent_config, "parent-agent");
    assert_eq!(result, CommunicationCheckResult::Allowed);

    // Child → sibling: child outbound is ["parent-agent"], doesn't include
    // "sibling-agent" → denied.
    let sibling_config = CommunicationConfig::default_with_parent(Some("parent-agent"));
    let result = check_communication_allowed(
        &child_config,
        "child-agent",
        &sibling_config,
        "sibling-agent",
    );
    assert_eq!(result, CommunicationCheckResult::TargetNotInSourceOutbound);
}

/// 2. outbound: ["*"] allows sending to any agent.
#[test]
fn test_check_communication_allowed_wildcard_outbound() {
    let config = CommunicationConfig {
        outbound: vec!["*".to_string()],
        inbound: vec!["any-agent".to_string()],
    };
    let target_config = CommunicationConfig {
        outbound: vec![],
        inbound: vec!["wildcard-agent".to_string()],
    };

    // wildcard-agent → any-agent: outbound is "*" → allowed.
    let result =
        check_communication_allowed(&config, "wildcard-agent", &target_config, "any-agent");
    assert_eq!(result, CommunicationCheckResult::Allowed);

    // wildcard-agent → unknown-agent: outbound is "*" → allowed.
    let result =
        check_communication_allowed(&config, "wildcard-agent", &target_config, "unknown-agent");
    assert_eq!(result, CommunicationCheckResult::Allowed);
}

/// 3. inbound: ["*"] allows receiving from any agent.
#[test]
fn test_check_communication_allowed_wildcard_inbound() {
    let source_config = CommunicationConfig {
        outbound: vec!["wildcard-target".to_string()],
        inbound: vec!["*".to_string()],
    };
    let target_config = CommunicationConfig {
        outbound: vec![],
        inbound: vec!["*".to_string()],
    };

    // source → target: source outbound has "wildcard-target" → allowed.
    let result = check_communication_allowed(
        &source_config,
        "source-agent",
        &target_config,
        "wildcard-target",
    );
    assert_eq!(result, CommunicationCheckResult::Allowed);
}

/// 4. Both parties restricted: only parent↔child allowed.
#[test]
fn test_check_communication_allowed_both_restricted() {
    let parent_config = CommunicationConfig {
        outbound: vec!["child-1".to_string()],
        inbound: vec!["child-1".to_string()],
    };
    let child_config = CommunicationConfig {
        outbound: vec!["parent-agent".to_string()],
        inbound: vec!["parent-agent".to_string()],
    };
    let other_config = CommunicationConfig {
        outbound: vec!["someone-else".to_string()],
        inbound: vec!["someone-else".to_string()],
    };

    // parent → child: parent outbound has "child-1", child inbound has
    // "parent-agent" → allowed.
    let result =
        check_communication_allowed(&parent_config, "parent-agent", &child_config, "child-1");
    assert_eq!(result, CommunicationCheckResult::Allowed);

    // child → parent: child outbound has "parent-agent", parent inbound has
    // "child-1" → allowed.
    let result =
        check_communication_allowed(&child_config, "child-1", &parent_config, "parent-agent");
    assert_eq!(result, CommunicationCheckResult::Allowed);

    // child → other: child outbound has "parent-agent", doesn't include
    // "other-agent" → denied.
    let result =
        check_communication_allowed(&child_config, "child-1", &other_config, "other-agent");
    assert_eq!(result, CommunicationCheckResult::TargetNotInSourceOutbound);

    // other → child: other outbound has "someone-else", doesn't include
    // "child-1" → denied.
    let result =
        check_communication_allowed(&other_config, "other-agent", &child_config, "child-1");
    assert_eq!(result, CommunicationCheckResult::TargetNotInSourceOutbound);
}

// ── SessionManager integration tests ───────────────────────────────────────

/// 5. steer_child returns correct error when communication check denies.
#[tokio::test]
#[serial]
async fn test_steer_child_denied_by_communication_check() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));

    // Register parent session.
    register_parent_session(&mgr, "parent-steer-deny", "parent-agent", tmp.path()).await;

    // Register parent session (with communication_config restricted to
    // "other-child" only — NOT our child).
    {
        let parent_cs = mgr
            .get_conversation_session("parent-steer-deny")
            .await
            .unwrap();
        let mut guard = parent_cs.write().await;
        *guard = guard
            .clone()
            .with_communication_config(CommunicationConfig {
                outbound: vec!["other-child".to_string()],
                inbound: vec!["other-child".to_string()],
            });
    }

    // Register a child session with config restricted to parent only.
    register_child_session(
        &mgr,
        "child-steer-deny",
        "child-agent",
        CommunicationConfig::default_with_parent(Some("parent-agent")),
    )
    .await;

    // Also register the child in the children table so get_parent_of works.
    mgr.register_child(
        "parent-steer-deny",
        ChildSessionInfo {
            session_id: "child-steer-deny".to_string(),
            parent_session_id: "parent-steer-deny".to_string(),
            agent_id: "child-agent".to_string(),
            depth: 1,
            mode: SpawnMode::Session,
        },
    )
    .await;

    // Attempt to steer — should be denied because parent outbound doesn't
    // include "child-agent".
    let result = mgr.steer_child("child-steer-deny", "new task").await;
    assert!(result.is_err(), "steer_child should fail when denied");
    let err_msg = result.unwrap_err();
    assert!(
        err_msg.contains("steer blocked by communication policy"),
        "error should indicate communication policy denial, got: {}",
        err_msg
    );
}

/// 6. try_push_announce returns early (no panic) when communication check
///    denies the announce from child to parent.
#[tokio::test]
#[serial]
async fn test_announce_denied_by_communication_check() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));

    // Register parent session with config that does NOT allow receiving from
    // the child's agent.
    register_parent_session(&mgr, "parent-announce-deny", "parent-agent", tmp.path()).await;
    {
        let parent_cs = mgr
            .get_conversation_session("parent-announce-deny")
            .await
            .unwrap();
        let mut guard = parent_cs.write().await;
        *guard = guard
            .clone()
            .with_communication_config(CommunicationConfig {
                outbound: vec!["unrelated-agent".to_string()],
                inbound: vec!["unrelated-agent".to_string()],
            });
    }

    // Register a run-mode child under the parent.
    register_child_session(
        &mgr,
        "child-announce-deny",
        "child-agent",
        CommunicationConfig::default_with_parent(Some("parent-agent")),
    )
    .await;

    // Register the child in the children table as run mode.
    mgr.register_child(
        "parent-announce-deny",
        ChildSessionInfo {
            session_id: "child-announce-deny".to_string(),
            parent_session_id: "parent-announce-deny".to_string(),
            agent_id: "child-agent".to_string(),
            depth: 1,
            mode: SpawnMode::Run,
        },
    )
    .await;

    // try_push_announce should return without panicking even though the
    // communication check denies. The announce is silently dropped.
    mgr.try_push_announce("child-announce-deny").await;

    // Verify no announce was pushed to the parent.
    let parent_cs = mgr
        .get_conversation_session("parent-announce-deny")
        .await
        .unwrap();
    let mut guard = parent_cs.write().await;
    let queue = guard.drain_announce_queue();
    assert!(
        queue.is_empty(),
        "announce queue should be empty when communication is denied"
    );
}

// ── Wildcard integration via SessionManager ─────────────────────────────────

/// Integration test: child with outbound: ["*"] can communicate with any agent
/// through check_session_communication.
#[tokio::test]
#[serial]
async fn test_wildcard_outbound_integration() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));

    // Register two parent sessions with different agents.
    register_parent_session(&mgr, "parent-a", "agent-a", tmp.path()).await;
    register_parent_session(&mgr, "parent-b", "agent-b", tmp.path()).await;

    // Register a child under parent-a with wildcard outbound.
    register_child_session(
        &mgr,
        "wildcard-child",
        "child-agent",
        CommunicationConfig {
            outbound: vec!["*".to_string()],
            inbound: vec!["agent-a".to_string()],
        },
    )
    .await;

    // wildcard-child → parent-b: outbound is "*" → allowed.
    let result = mgr
        .check_session_communication("wildcard-child", "parent-b")
        .await;
    assert!(
        result.is_ok(),
        "wildcard outbound should allow communication with any agent"
    );
}

/// Integration test: child with inbound: ["*"] can receive from any agent.
#[tokio::test]
#[serial]
async fn test_wildcard_inbound_integration() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));

    // Register two sessions.
    register_parent_session(&mgr, "source-session", "source-agent", tmp.path()).await;
    register_child_session(
        &mgr,
        "wildcard-receiver",
        "receiver-agent",
        CommunicationConfig {
            outbound: vec!["source-agent".to_string()],
            inbound: vec!["*".to_string()],
        },
    )
    .await;

    // source → wildcard-receiver: inbound is "*" → allowed.
    let result = mgr
        .check_session_communication("source-session", "wildcard-receiver")
        .await;
    assert!(
        result.is_ok(),
        "wildcard inbound should allow receiving from any agent"
    );
}

/// Integration test: denied communication returns CommunicationError::Denied.
#[tokio::test]
#[serial]
async fn test_session_communication_denied_returns_error() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));

    register_parent_session(&mgr, "parent-deny", "parent-agent", tmp.path()).await;

    // Child restricted to "other-parent" only, not "parent-agent".
    register_child_session(
        &mgr,
        "restricted-child",
        "child-agent",
        CommunicationConfig {
            outbound: vec!["other-parent".to_string()],
            inbound: vec!["other-parent".to_string()],
        },
    )
    .await;

    let result = mgr
        .check_session_communication("restricted-child", "parent-deny")
        .await;
    assert!(result.is_err(), "communication should be denied");
    match result.unwrap_err() {
        CommunicationError::Denied { reason } => {
            assert!(
                reason.contains("outbound"),
                "reason should mention outbound, got: {}",
                reason
            );
        }
        other => panic!("expected CommunicationError::Denied, got: {:?}", other),
    }
}

/// Integration test: session not found returns CommunicationError::SessionNotFound.
#[tokio::test]
#[serial]
async fn test_session_communication_not_found() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));

    register_parent_session(&mgr, "existing-session", "agent-x", tmp.path()).await;

    let result = mgr
        .check_session_communication("existing-session", "nonexistent-session")
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        CommunicationError::SessionNotFound(id) => {
            assert_eq!(id, "nonexistent-session");
        }
        other => panic!(
            "expected CommunicationError::SessionNotFound, got: {:?}",
            other
        ),
    }
}
