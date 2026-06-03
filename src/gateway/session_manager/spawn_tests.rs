//! Tests for `SessionManager::create_child_session` and the children tracking table.
//!
//! These tests live in a separate module to keep `session_manager.rs` and
//! `tests.rs` under the project's 500-line file limit. Shared helpers
//! (`make_test_mgr`, `clear_global_prompt_state`) are re-exported by
//! `super::tests` at `pub(super)` visibility.

use super::spawn::SpawnMode;
use super::tests::{clear_global_prompt_state, make_test_mgr};
use crate::agent::config::SubagentsConfig;
use crate::config::agents::{ConfigSource, ResolvedAgentConfig};
use crate::session::bootstrap::BootstrapMode;
use serial_test::serial;
use std::path::PathBuf;

fn test_resolved_config(id: &str, workspace: Option<PathBuf>) -> ResolvedAgentConfig {
    ResolvedAgentConfig {
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
        source: ConfigSource::Merged,
    }
}

#[tokio::test]
#[serial]
async fn test_create_child_session_basic() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("child-agent", None);

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-session-1",
            1,
            "do something useful",
            false,
            None,
            SpawnMode::Run,
        )
        .await
        .expect("create_child_session should succeed");

    // New child id is a UUID string
    assert_eq!(child_id.len(), 36, "child id should be a UUID string");

    // Child appears in sessions
    assert!(mgr.has_session(&child_id).await);

    // Depth is propagated
    assert_eq!(mgr.get_session_depth(&child_id).await, Some(1));

    // Children tracking table has the child
    assert_eq!(mgr.count_active_children("parent-session-1").await, 1);

    // ConversationSession exists with the task as first pending message
    let cs = mgr
        .get_conversation_session(&child_id)
        .await
        .expect("conversation session should exist");
    let cs_guard = cs.read().await;
    assert_eq!(cs_guard.get_pending_messages().len(), 1);
    assert_eq!(
        cs_guard.get_pending_messages()[0].content,
        "do something useful"
    );
}

#[tokio::test]
#[serial]
async fn test_create_child_session_workspace_fallback() {
    clear_global_prompt_state();

    // No manager-level workspace → falls back to config.workspace
    let mgr = make_test_mgr(None);
    let explicit = tempfile::TempDir::new().unwrap();
    let config = test_resolved_config("child-agent", Some(explicit.path().to_path_buf()));

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-x",
            2,
            "task body",
            true,
            None,
            SpawnMode::Session,
        )
        .await
        .expect("create_child_session should succeed");

    // depth=2 respected
    assert_eq!(mgr.get_session_depth(&child_id).await, Some(2));

    // explicit workspace arg overrides config.workspace
    let other = tempfile::TempDir::new().unwrap();
    let child_id_2 = mgr
        .create_child_session(
            &config,
            "parent-x",
            3,
            "task body 2",
            false,
            Some(other.path().to_str().unwrap()),
            SpawnMode::Run,
        )
        .await
        .expect("create_child_session with explicit workspace should succeed");

    // Both children are tracked under the same parent
    assert_eq!(mgr.count_active_children("parent-x").await, 2);

    // Different ids
    assert_ne!(child_id, child_id_2);
}

#[tokio::test]
#[serial]
async fn test_create_child_session_registers_child_info() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let config = test_resolved_config("worker-1", None);

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-7",
            1,
            "do work",
            false,
            None,
            SpawnMode::Session,
        )
        .await
        .expect("create_child_session should succeed");

    // Direct lookup via children table
    let children = mgr.children.read().await;
    let list = children
        .get("parent-7")
        .expect("parent-7 should have a children entry");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].session_id, child_id);
    assert_eq!(list[0].parent_session_id, "parent-7");
    assert_eq!(list[0].agent_id, "worker-1");
    assert_eq!(list[0].depth, 1);
    assert_eq!(list[0].mode, SpawnMode::Session);
}
