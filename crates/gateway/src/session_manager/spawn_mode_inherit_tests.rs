//! Tests for Step 1.2: child session inherits parent's session_mode.

use super::spawn::SpawnMode;
use super::tests::{clear_global_prompt_state, make_test_mgr};
use super::SessionManager;
use closeclaw_config::agents::{ConfigSource, MemoryConfig, ModelSpec, ResolvedAgentConfig};
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::SessionMode;
use serial_test::serial;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

fn test_resolved_config(id: &str) -> ResolvedAgentConfig {
    ResolvedAgentConfig {
        id: id.to_string(),
        name: id.to_string(),
        parent_id: None,
        model: Some(ModelSpec::single("test-model")),
        workspace: None,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills: vec![],
        tools: vec![],
        disallowed_tools: vec![],
        subagents: Default::default(),
        memory: MemoryConfig::default(),
        source: ConfigSource::Merged,
    }
}

/// Helper: register a parent session with a specific `SessionMode`.
async fn register_parent_session_with_mode(
    mgr: &SessionManager,
    parent_id: &str,
    workdir: PathBuf,
    mode: SessionMode,
) {
    let cs = ConversationSession::new(parent_id.to_string(), "test-model".to_string(), workdir)
        .with_session_mode(mode);
    let arc = Arc::new(RwLock::new(cs));
    let mut conv = mgr.conversation_sessions.write().await;
    conv.insert(parent_id.to_string(), arc);
    mgr.sessions.write().await.insert(
        parent_id.to_string(),
        crate::Session {
            id: parent_id.to_string(),
            agent_id: "parent-agent".to_string(),
            channel: "spawn".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
}

/// When parent is in Plan Mode, the child inherits Plan Mode.
#[tokio::test]
#[serial]
async fn test_child_inherits_plan_mode_from_parent() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("plan-child");

    register_parent_session_with_mode(
        &mgr,
        "parent-plan",
        tmp.path().to_path_buf(),
        SessionMode::Plan,
    )
    .await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-plan",
            1,
            "plan task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
        )
        .await
        .expect("create_child_session should succeed");

    let cs = mgr
        .get_conversation_session(&child_id)
        .await
        .expect("child session should exist");
    let guard = cs.read().await;
    assert_eq!(
        guard.session_mode(),
        SessionMode::Plan,
        "child should inherit Plan Mode from parent"
    );
}

/// When parent is in Normal Mode, the child stays in Normal Mode.
#[tokio::test]
#[serial]
async fn test_child_stays_normal_when_parent_is_normal() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("normal-child");

    register_parent_session_with_mode(
        &mgr,
        "parent-normal",
        tmp.path().to_path_buf(),
        SessionMode::Normal,
    )
    .await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-normal",
            1,
            "normal task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
        )
        .await
        .expect("create_child_session should succeed");

    let cs = mgr
        .get_conversation_session(&child_id)
        .await
        .expect("child session should exist");
    let guard = cs.read().await;
    assert_eq!(
        guard.session_mode(),
        SessionMode::Normal,
        "child should remain in Normal Mode when parent is Normal"
    );
}

/// When parent is in Auto Mode, the child stays in Normal Mode
/// (only Plan Mode is inherited, per design doc).
#[tokio::test]
#[serial]
async fn test_child_stays_normal_when_parent_is_auto() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("auto-child");

    register_parent_session_with_mode(
        &mgr,
        "parent-auto",
        tmp.path().to_path_buf(),
        SessionMode::Auto,
    )
    .await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-auto",
            1,
            "auto task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
        )
        .await
        .expect("create_child_session should succeed");

    let cs = mgr
        .get_conversation_session(&child_id)
        .await
        .expect("child session should exist");
    let guard = cs.read().await;
    assert_eq!(
        guard.session_mode(),
        SessionMode::Normal,
        "child should remain in Normal Mode when parent is Auto"
    );
}
