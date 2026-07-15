//! Tests for `SessionManager::kill_child` cascade behavior.
//!
//! Validates the design doc §级联 Kill ordering: deepest descendants
//! are removed first, completed/terminated sessions skip cancel but
//! are still cleaned up, and single-child scenarios work correctly.

use super::spawn::{ChildSessionInfo, SpawnMode};
use super::test_helpers::test_resolved_config;
use super::tests::{clear_global_prompt_state, make_test_mgr};
use super::SessionManager;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_session::llm_session::ConversationSession;
use serial_test::serial;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Helper: register a `ConversationSession` in the manager's
/// `conversation_sessions` map and in `sessions`.
async fn register_session(mgr: &SessionManager, id: &str, agent_id: &str, depth: u32) {
    let cs = ConversationSession::new(id.to_string(), "test-model".into(), PathBuf::from("/tmp"));
    mgr.conversation_sessions
        .write()
        .await
        .insert(id.to_string(), Arc::new(RwLock::new(cs)));
    mgr.sessions.write().await.insert(
        id.to_string(),
        crate::Session {
            id: id.to_string(),
            agent_id: agent_id.to_string(),
            channel: "spawn".into(),
            created_at: 0,
            depth,
        },
    );
}

/// Helper: register a parent-child entry in the `children` table.
async fn register_tree_entry(
    mgr: &SessionManager,
    parent_id: &str,
    child_id: &str,
    agent_id: &str,
    depth: u32,
) {
    mgr.register_child(
        parent_id,
        ChildSessionInfo {
            session_id: child_id.to_string(),
            parent_session_id: parent_id.to_string(),
            agent_id: agent_id.to_string(),
            depth,
            mode: SpawnMode::Run,
        },
    )
    .await;
}

/// Verify `kill_child` removes descendants deepest-first when
/// the tree has 3+ layers.
///
/// Tree:
/// ```text
///   parent (root, not killed)
///     └─ child (killed)
///          └─ grandchild
///               ├─ great_grandchild (deepest leaf)
///               └─ great_grandchild2 (deepest leaf)
/// ```
///
/// `list_descendants` returns deepest-first (reversed BFS), so
/// `kill_child` should clean up in order:
/// great_grandchild, great_grandchild2, grandchild, then child.
#[tokio::test]
#[serial]
async fn test_kill_child_deep_nesting_removes_leaves_first() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));

    // Register parent with a ConversationSession.
    {
        let cs = ConversationSession::new(
            "parent".into(),
            "test-model".into(),
            tmp.path().to_path_buf(),
        );
        mgr.conversation_sessions
            .write()
            .await
            .insert("parent".into(), Arc::new(RwLock::new(cs)));
        mgr.sessions.write().await.insert(
            "parent".into(),
            crate::Session {
                id: "parent".into(),
                agent_id: "root-agent".into(),
                channel: "spawn".into(),
                created_at: 0,
                depth: 0,
            },
        );
    }

    // Register all descendant sessions.
    register_session(&mgr, "child", "child-agent", 1).await;
    register_session(&mgr, "grandchild", "gc-agent", 2).await;
    register_session(&mgr, "great_grandchild", "ggc-agent", 3).await;
    register_session(&mgr, "great_grandchild2", "ggc2-agent", 3).await;

    // Build the spawn tree: parent→child→grandchild→{great, great2}.
    register_tree_entry(&mgr, "parent", "child", "child-agent", 1).await;
    register_tree_entry(&mgr, "child", "grandchild", "gc-agent", 2).await;
    register_tree_entry(&mgr, "grandchild", "great_grandchild", "ggc-agent", 3).await;
    register_tree_entry(&mgr, "grandchild", "great_grandchild2", "ggc2-agent", 3).await;

    // Confirm pre-conditions: all sessions exist.
    assert!(mgr.has_session("child").await);
    assert!(mgr.has_session("grandchild").await);
    assert!(mgr.has_session("great_grandchild").await);
    assert!(mgr.has_session("great_grandchild2").await);

    // Kill the child and all its descendants.
    mgr.kill_child("parent", "child")
        .await
        .expect("kill_child should succeed");

    // All descendants and the child itself should be removed.
    assert!(!mgr.has_session("child").await, "child should be removed");
    assert!(
        !mgr.has_session("grandchild").await,
        "grandchild should be removed"
    );
    assert!(
        !mgr.has_session("great_grandchild").await,
        "great_grandchild should be removed"
    );
    assert!(
        !mgr.has_session("great_grandchild2").await,
        "great_grandchild2 should be removed"
    );

    // conversation_sessions entries are gone.
    assert!(mgr.get_conversation_session("child").await.is_none());
    assert!(mgr.get_conversation_session("grandchild").await.is_none());
    assert!(mgr
        .get_conversation_session("great_grandchild")
        .await
        .is_none());
    assert!(mgr
        .get_conversation_session("great_grandchild2")
        .await
        .is_none());

    // children table is empty for the killed subtree.
    assert_eq!(mgr.count_active_children("child").await, 0);
    assert_eq!(mgr.count_active_children("grandchild").await, 0);
}

/// Verify that a session which is already stopped (completed /
/// terminated) skips the `stop()` call but is still removed from
/// `conversation_sessions`, `sessions`, and the `children` table.
///
/// Simulates the scenario where a run-mode child completed its turn
/// before the parent calls `kill_child`.
#[tokio::test]
#[serial]
async fn test_kill_child_completed_session_skips_stop() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("done-child", None);

    // Register parent with a ConversationSession.
    {
        let cs = ConversationSession::new(
            "parent-done".into(),
            "test-model".into(),
            tmp.path().to_path_buf(),
        );
        mgr.conversation_sessions
            .write()
            .await
            .insert("parent-done".into(), Arc::new(RwLock::new(cs)));
        mgr.sessions.write().await.insert(
            "parent-done".into(),
            crate::Session {
                id: "parent-done".into(),
                agent_id: "root-agent".into(),
                channel: "spawn".into(),
                created_at: 0,
                depth: 0,
            },
        );
    }

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-done",
            1,
            "task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None, // spawn_timeout,
            None, // label
        )
        .await
        .expect("create_child_session should succeed");

    // Simulate the child having completed (stop already called).
    {
        let cs = mgr
            .get_conversation_session(&child_id)
            .await
            .expect("conversation session should exist");
        cs.read().await.stop(true, ShutdownMode::Forceful).await; // mark as stopped
    }
    // Verify the stopped flag is now set.
    {
        let cs = mgr.get_conversation_session(&child_id).await.unwrap();
        assert!(cs.read().await.is_stopped(), "session should be stopped");
    }

    // Kill should succeed and remove from all tables.
    mgr.kill_child("parent-done", &child_id)
        .await
        .expect("kill_child should succeed");

    assert!(
        !mgr.has_session(&child_id).await,
        "session should be removed from sessions"
    );
    assert!(
        mgr.get_conversation_session(&child_id).await.is_none(),
        "session should be removed from conversation_sessions"
    );
    assert_eq!(
        mgr.count_active_children("parent-done").await,
        0,
        "children table should be empty"
    );
}

/// Verify `kill_child` works correctly with a single child node
/// (no deeper descendants).
#[tokio::test]
#[serial]
async fn test_kill_child_single_child() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("only-child", None);

    // Register parent with a ConversationSession.
    {
        let cs = ConversationSession::new(
            "parent-single".into(),
            "test-model".into(),
            tmp.path().to_path_buf(),
        );
        mgr.conversation_sessions
            .write()
            .await
            .insert("parent-single".into(), Arc::new(RwLock::new(cs)));
        mgr.sessions.write().await.insert(
            "parent-single".into(),
            crate::Session {
                id: "parent-single".into(),
                agent_id: "root-agent".into(),
                channel: "spawn".into(),
                created_at: 0,
                depth: 0,
            },
        );
    }

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-single",
            1,
            "lone task",
            false,
            None,
            SpawnMode::Session,
            false,
            None,
            None,
            None,
            3,
            None, // spawn_timeout,
            None, // label
        )
        .await
        .expect("create_child_session should succeed");

    assert!(mgr.has_session(&child_id).await);
    assert!(mgr.get_conversation_session(&child_id).await.is_some());
    assert_eq!(mgr.count_active_children("parent-single").await, 1);

    // Kill the single child.
    mgr.kill_child("parent-single", &child_id)
        .await
        .expect("kill_child should succeed");

    assert!(
        !mgr.has_session(&child_id).await,
        "child removed from sessions"
    );
    assert!(
        mgr.get_conversation_session(&child_id).await.is_none(),
        "child removed from conversation_sessions"
    );
    assert_eq!(
        mgr.count_active_children("parent-single").await,
        0,
        "children table should be empty"
    );
    let children = mgr.children.read().await;
    assert!(
        children.list_children("parent-single").is_empty(),
        "parent entry should be removed when child list is empty"
    );
}
