//! Step 1.5 unit tests for gaps 1–4 in agent spawn.
//!
//! Covers tool-level interception (Gap 1), `SpawnTree::get_parent`
//! (Gap 2), `count_active_children` semantics (Gap 3), and
//! `kill_child` for completed sessions (Gap 4).

use super::spawn::SpawnMode;
use super::tests::{clear_global_prompt_state, make_test_mgr};
use super::SessionManager;
use closeclaw_config::agents::{ConfigSource, MemoryConfig, ResolvedAgentConfig};
use closeclaw_llm::session::ChatSession;
use closeclaw_session::recovery::SpawnTree;

use serial_test::serial;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

fn test_resolved_config(id: &str) -> ResolvedAgentConfig {
    ResolvedAgentConfig {
        id: id.to_string(),
        name: id.to_string(),
        parent_id: None,
        model: Some("test-model".to_string()),
        workspace: None,
        agent_dir: None,
        bootstrap_mode: closeclaw_session::bootstrap::BootstrapMode::Full,
        skills: vec![],
        tools: vec![],
        disallowed_tools: vec![],
        subagents: closeclaw_config::agents::SubagentsConfig::default(),
        memory: MemoryConfig::default(),
        source: ConfigSource::Merged,
    }
}

/// Register a parent `ConversationSession` so child creation can
/// derive the cancel token from the parent's token tree.
async fn register_parent(mgr: &SessionManager, parent_id: &str, workdir: PathBuf) {
    use closeclaw_llm::session::ConversationSession;
    use tokio::sync::RwLock;

    let cs = ConversationSession::new(parent_id.to_string(), "test-model".to_string(), workdir);
    let arc = Arc::new(RwLock::new(cs));
    mgr.conversation_sessions
        .write()
        .await
        .insert(parent_id.to_string(), arc);
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

// ── Mock tool for tool-level interception test ────────────────────────────

struct MockSpawnTool;

impl closeclaw_common::tool_registry::Tool for MockSpawnTool {
    fn name(&self) -> &str {
        "sessions_spawn"
    }
    fn group(&self) -> &str {
        "sessions"
    }
    fn summary(&self) -> String {
        "Spawn a sub-agent for a sub-task".into()
    }
    fn detail(&self) -> String {
        "Create a child session for a spawned sub-agent.".into()
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }
    fn flags(&self) -> closeclaw_common::tool_registry::ToolFlags {
        closeclaw_common::tool_registry::ToolFlags::default()
    }
}

/// Create a `SessionManager` with a `ToolRegistry` containing a mock
/// `sessions_spawn` tool so the system prompt renders tool definitions.
async fn make_mgr_with_spawn_tool(workspace: Option<&std::path::Path>) -> SessionManager {
    use closeclaw_common::tool_registry::ToolRegistry;

    let mgr = make_test_mgr(workspace);
    let registry = Arc::new(ToolRegistry::new());
    registry.register(MockSpawnTool).await.unwrap();
    mgr.set_tool_registry(registry).await;
    mgr
}

// ── Gap 1 tests ─────────────────────────────────────────────────────────

/// Verify that `max_spawn_depth == 0` strips `sessions_spawn` from the
/// child's tool whitelist so the LLM cannot call it.
#[tokio::test]
#[serial]
async fn test_create_child_session_removes_spawn_tool_when_budget_zero() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_mgr_with_spawn_tool(Some(tmp.path())).await;
    // Config must list sessions_spawn in tools so whitelist filtering
    // is active (empty list = wildcard = no filtering).  Include other
    // tools so the filtered list is non-empty (proper whitelist).
    let config = ResolvedAgentConfig {
        tools: vec!["sessions_spawn".to_string(), "read".to_string()],
        ..test_resolved_config("child-agent")
    };

    register_parent(&mgr, "parent-budget-0", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-budget-0",
            1,
            "task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            0, // max_spawn_depth == 0
        )
        .await
        .expect("create_child_session should succeed");

    let cs = mgr
        .get_conversation_session(&child_id)
        .await
        .expect("child session should exist");
    let guard = cs.read().await;
    let prompt = guard.system_prompt().expect("system prompt should be set");

    // The tool definition for sessions_spawn must NOT appear when the
    // budget is zero.
    assert!(
        !prompt.contains("**sessions_spawn**"),
        "system prompt must NOT contain sessions_spawn tool definition \
         when max_spawn_depth == 0, but found it in: {:?}",
        prompt
    );
}

/// Verify that `max_spawn_depth > 0` keeps `sessions_spawn` in the
/// child's tool whitelist.
#[tokio::test]
#[serial]
async fn test_create_child_session_keeps_spawn_tool_when_budget_positive() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_mgr_with_spawn_tool(Some(tmp.path())).await;
    let config = ResolvedAgentConfig {
        tools: vec!["sessions_spawn".to_string(), "read".to_string()],
        ..test_resolved_config("child-agent")
    };

    register_parent(&mgr, "parent-budget-pos", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-budget-pos",
            1,
            "task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3, // max_spawn_depth > 0
        )
        .await
        .expect("create_child_session should succeed");

    let cs = mgr
        .get_conversation_session(&child_id)
        .await
        .expect("child session should exist");
    let guard = cs.read().await;
    let prompt = guard.system_prompt().expect("system prompt should be set");

    // The tool definition for sessions_spawn must appear when the
    // budget is positive.
    assert!(
        prompt.contains("**sessions_spawn**"),
        "system prompt must contain sessions_spawn tool definition \
         when max_spawn_depth > 0, but it was missing in: {:?}",
        prompt
    );
}

// ── Gap 2 tests (SpawnTree::get_parent) ─────────────────────────────────

/// Verify that `get_parent` returns `None` for a root session.
#[test]
fn test_spawn_tree_get_parent_root() {
    let tree = SpawnTree {
        roots: vec!["root-1".to_string(), "root-2".to_string()],
        children: HashMap::new(),
    };
    assert_eq!(tree.get_parent("root-1"), None);
    assert_eq!(tree.get_parent("root-2"), None);
}

/// Verify that `get_parent` returns the correct parent for a child.
#[test]
fn test_spawn_tree_get_parent_child() {
    let mut children = HashMap::new();
    children.insert(
        "parent-1".to_string(),
        vec!["child-a".to_string(), "child-b".to_string()],
    );
    let tree = SpawnTree {
        roots: vec!["parent-1".to_string()],
        children,
    };
    assert_eq!(
        tree.get_parent("child-a").map(|s| s.as_str()),
        Some("parent-1")
    );
    assert_eq!(
        tree.get_parent("child-b").map(|s| s.as_str()),
        Some("parent-1")
    );
    // Unknown session returns None.
    assert_eq!(tree.get_parent("unknown-id"), None);
}

// ── Gap 3 tests (count_active_children) ──────────────────────────────────

/// Verify that completed run-mode children are excluded from the
/// active count.
#[tokio::test]
#[serial]
async fn test_count_active_children_excludes_completed() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("worker");

    register_parent(&mgr, "parent-count", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-count",
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
        )
        .await
        .expect("create_child_session should succeed");

    // Initially active.
    assert_eq!(mgr.count_active_children("parent-count").await, 1);

    // Simulate completion: remove from conversation_sessions.
    mgr.conversation_sessions.write().await.remove(&child_id);

    // Completed child must NOT be counted.
    assert_eq!(
        mgr.count_active_children("parent-count").await,
        0,
        "completed run-mode child should not be counted"
    );
}

/// Verify that active session-mode children are correctly counted.
#[tokio::test]
#[serial]
async fn test_count_active_children_includes_active() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("worker");

    register_parent(&mgr, "parent-active", tmp.path().to_path_buf()).await;

    // Spawn 2 active session-mode children.
    let id1 = mgr
        .create_child_session(
            &config,
            "parent-active",
            1,
            "task1",
            false,
            None,
            SpawnMode::Session,
            false,
            None,
            None,
            None,
            3,
        )
        .await
        .unwrap();
    let id2 = mgr
        .create_child_session(
            &config,
            "parent-active",
            1,
            "task2",
            false,
            None,
            SpawnMode::Session,
            false,
            None,
            None,
            None,
            3,
        )
        .await
        .unwrap();

    assert_eq!(mgr.count_active_children("parent-active").await, 2);

    // Complete one child, verify count drops.
    mgr.conversation_sessions.write().await.remove(&id1);
    assert_eq!(
        mgr.count_active_children("parent-active").await,
        1,
        "only one active child should remain"
    );

    // Complete the other, count should be 0.
    mgr.conversation_sessions.write().await.remove(&id2);
    assert_eq!(
        mgr.count_active_children("parent-active").await,
        0,
        "no active children should remain"
    );
}

// ── Gap 4 tests (kill completed child) ───────────────────────────────────

/// Verify that `kill_child` succeeds for a completed child — it must
/// skip the stop step but still clean up from the children table.
#[tokio::test]
#[serial]
async fn test_kill_completed_child_succeeds() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("worker");

    register_parent(&mgr, "parent-kill-completed", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-kill-completed",
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
        )
        .await
        .unwrap();

    // Simulate completion: remove from conversation_sessions.
    mgr.conversation_sessions.write().await.remove(&child_id);

    // Kill must NOT error on a completed child.
    let result = mgr.kill_child("parent-kill-completed", &child_id).await;
    assert!(
        result.is_ok(),
        "kill_child should succeed for completed child, got: {:?}",
        result
    );

    // Child must be removed from children table.
    assert_eq!(
        mgr.count_active_children("parent-kill-completed").await,
        0,
        "completed child should be removed from children table"
    );
}

/// Verify that `kill_child` on an active child stops and cleans up
/// correctly, including cascading to descendants.
#[tokio::test]
#[serial]
async fn test_kill_active_child_cascades() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config_child = test_resolved_config("child");
    let config_grandchild = test_resolved_config("grandchild");

    register_parent(&mgr, "parent-kill-active", tmp.path().to_path_buf()).await;

    // Spawn child.
    let child_id = mgr
        .create_child_session(
            &config_child,
            "parent-kill-active",
            1,
            "child task",
            false,
            None,
            SpawnMode::Session,
            false,
            None,
            None,
            None,
            3,
        )
        .await
        .unwrap();

    // Spawn grandchild under child.
    let grandchild_id = mgr
        .create_child_session(
            &config_grandchild,
            &child_id,
            2,
            "grandchild task",
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
        .unwrap();

    // Both active.
    assert!(mgr.has_session(&child_id).await);
    assert!(mgr.has_session(&grandchild_id).await);

    // Kill the child — should cascade to grandchild.
    mgr.kill_child("parent-kill-active", &child_id)
        .await
        .expect("kill_child should succeed");

    assert!(!mgr.has_session(&child_id).await);
    assert!(!mgr.has_session(&grandchild_id).await);
    assert_eq!(mgr.count_active_children("parent-kill-active").await, 0);
}

/// Verify cascade kill handles a mix of active and completed children
/// at different levels of the spawn tree.
#[tokio::test]
#[serial]
async fn test_cascade_kill_mixed_active_completed() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config_child = test_resolved_config("child");
    let config_active = test_resolved_config("active");
    let config_completed = test_resolved_config("completed");

    register_parent(&mgr, "parent-mixed", tmp.path().to_path_buf()).await;

    // Spawn child (active, session-mode).
    let child_id = mgr
        .create_child_session(
            &config_child,
            "parent-mixed",
            1,
            "child task",
            false,
            None,
            SpawnMode::Session,
            false,
            None,
            None,
            None,
            3,
        )
        .await
        .unwrap();

    // Spawn active grandchild under child.
    let active_gc = mgr
        .create_child_session(
            &config_active,
            &child_id,
            2,
            "active grandchild",
            false,
            None,
            SpawnMode::Session,
            false,
            None,
            None,
            None,
            3,
        )
        .await
        .unwrap();

    // Spawn completed grandchild under child, then simulate completion.
    let completed_gc = mgr
        .create_child_session(
            &config_completed,
            &child_id,
            2,
            "completed grandchild",
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
        .unwrap();
    mgr.conversation_sessions
        .write()
        .await
        .remove(&completed_gc);

    // Child has 2 descendants: 1 active, 1 completed.
    assert!(mgr.has_session(&child_id).await);
    assert!(mgr.has_session(&active_gc).await);
    assert_eq!(
        mgr.count_active_children(&child_id).await,
        1,
        "child should have exactly 1 active descendant"
    );

    // Kill the child — must handle both active and completed descendants.
    mgr.kill_child("parent-mixed", &child_id)
        .await
        .expect("kill_child should succeed with mixed descendants");

    assert!(!mgr.has_session(&child_id).await);
    assert!(!mgr.has_session(&active_gc).await);
    assert_eq!(mgr.count_active_children("parent-mixed").await, 0);
}
