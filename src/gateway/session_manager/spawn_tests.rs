//! Tests for `SessionManager::create_child_session` and the children tracking table.
//!
//! These tests live in a separate module to keep `session_manager.rs` and
//! `tests.rs` under the project's 500-line file limit. Shared helpers
//! (`make_test_mgr`, `clear_global_prompt_state`) are re-exported by
//! `super::tests` at `pub(super)` visibility.

use super::spawn::SpawnMode;
use super::tests::{clear_global_prompt_state, make_test_mgr, test_config};
use super::SessionManager;
use crate::agent::config::SubagentsConfig;
use crate::config::agents::{ConfigSource, ResolvedAgentConfig};
use crate::llm::session::ConversationSession;
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::{PersistenceService, SessionCheckpoint};
use crate::session::ReasoningLevel;
use serial_test::serial;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

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
        permissions: None,
        source: ConfigSource::Merged,
    }
}

/// Pre-populate a parent `ConversationSession` in the manager's
/// `conversation_sessions` map.
///
/// Step 1.5 of the cascade-stop plan requires the parent session to
/// be present in `conversation_sessions` so `create_child_session`
/// can derive the child's cancel token from the parent's token tree
/// and register the child handle for cascade stopping. In production
/// the parent is registered by `find_or_create`; tests exercise
/// `create_child_session` in isolation and must do this setup
/// themselves.
async fn register_parent_session(mgr: &SessionManager, parent_id: &str, workdir: PathBuf) {
    let cs = ConversationSession::new(parent_id.to_string(), "test-model".to_string(), workdir);
    let arc = Arc::new(RwLock::new(cs));
    let mut conv = mgr.conversation_sessions.write().await;
    conv.insert(parent_id.to_string(), arc);
    // Also register in `sessions` so `get_chat_id` can resolve the
    // parent's agent_id for communication config generation.
    mgr.sessions.write().await.insert(
        parent_id.to_string(),
        crate::gateway::Session {
            id: parent_id.to_string(),
            agent_id: "parent-agent".to_string(),
            channel: "spawn".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
}

#[tokio::test]
#[serial]
async fn test_create_child_session_basic() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("child-agent", None);

    // Step 1.5 requires the parent to live in conversation_sessions
    // so the child can be wired into the parent's cancel token tree.
    register_parent_session(&mgr, "parent-session-1", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-session-1",
            1,
            "do something useful",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3, // remaining_spawn_depth
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
    // Spawn context is now injected into the first user message
    let first_msg = &cs_guard.get_pending_messages()[0].content;
    assert!(
        first_msg.contains("do something useful"),
        "pending message should contain the task"
    );
    assert!(
        first_msg.contains("Spawn Context"),
        "pending message should contain spawn context"
    );
    assert!(
        first_msg.contains("sub-agent"),
        "pending message should contain sub-agent role declaration"
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

    // Step 1.5: pre-populate parent so child can inherit its cancel
    // token tree.
    register_parent_session(&mgr, "parent-x", explicit.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-x",
            2,
            "task body",
            true,
            None,
            SpawnMode::Session,
            false,
            None,
            None,
            None,
            3, // remaining_spawn_depth
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
            false,
            None,
            None,
            None,
            3, // remaining_spawn_depth
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

    // Step 1.5: pre-populate parent so child inherits the parent's
    // cancel token tree and is registered in the parent's
    // child_handles.
    let parent_workdir = std::env::temp_dir();
    register_parent_session(&mgr, "parent-7", parent_workdir).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-7",
            1,
            "do work",
            false,
            None,
            SpawnMode::Session,
            false,
            None,
            None,
            None,
            3, // remaining_spawn_depth
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

#[tokio::test]
#[serial]
async fn test_steer_child_injects_pending_message() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("steer-child", None);

    register_parent_session(&mgr, "parent-steer", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-steer",
            1,
            "initial task",
            false,
            None,
            SpawnMode::Session,
            false,
            None,
            None,
            None,
            3, // remaining_spawn_depth
        )
        .await
        .expect("create_child_session should succeed");

    // Steer the child with a new task
    mgr.steer_child(&child_id, "new task")
        .await
        .expect("steer_child should succeed");

    // Verify the pending message was injected
    let cs = mgr
        .get_conversation_session(&child_id)
        .await
        .expect("conversation session should exist");
    let cs_guard = cs.read().await;
    let pending = cs_guard.get_pending_messages();
    // There should be 2 pending messages: the original task + the steer message
    assert!(
        pending.len() >= 2,
        "expected at least 2 pending messages, got {}",
        pending.len()
    );
    let steer_msg = pending
        .iter()
        .find(|m| m.content == "new task")
        .expect("pending messages should contain 'new task'");
    assert_eq!(steer_msg.content, "new task");
}

#[tokio::test]
#[serial]
async fn test_kill_child_removes_from_all_tables() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("kill-child", None);

    register_parent_session(&mgr, "parent-kill", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-kill",
            1,
            "doomed task",
            false,
            None,
            SpawnMode::Session,
            false,
            None,
            None,
            None,
            3, // remaining_spawn_depth
        )
        .await
        .expect("create_child_session should succeed");

    // Confirm child exists before kill
    assert!(mgr.has_session(&child_id).await);
    assert!(mgr.get_conversation_session(&child_id).await.is_some());
    assert_eq!(mgr.count_active_children("parent-kill").await, 1);

    // Kill the child
    mgr.kill_child("parent-kill", &child_id)
        .await
        .expect("kill_child should succeed");

    // Verify child is removed from sessions
    assert!(
        !mgr.has_session(&child_id).await,
        "has_session should return false after kill"
    );

    // Verify child is removed from conversation_sessions
    assert!(
        mgr.get_conversation_session(&child_id).await.is_none(),
        "get_conversation_session should return None after kill"
    );

    // Verify child is removed from children tracking table
    assert_eq!(
        mgr.count_active_children("parent-kill").await,
        0,
        "children table should be empty after kill"
    );
    let children = mgr.children.read().await;
    assert!(
        children.get("parent-kill").is_none(),
        "parent entry should be removed from children table when list is empty"
    );
}

#[tokio::test]
#[serial]
async fn test_validate_child_ownership_returns_none_for_run_mode() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("run-child", None);

    register_parent_session(&mgr, "parent-validate-run", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-validate-run",
            1,
            "run task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3, // remaining_spawn_depth
        )
        .await
        .expect("create_child_session should succeed");

    // validate_child_ownership should return None for Run mode children
    let result = mgr
        .validate_child_ownership("parent-validate-run", &child_id)
        .await;
    assert!(
        result.is_none(),
        "validate_child_ownership should return None for Run mode children"
    );
}

#[tokio::test]
#[serial]
async fn test_validate_child_ownership_returns_info_for_session_mode() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("session-child", None);

    register_parent_session(&mgr, "parent-validate-session", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-validate-session",
            1,
            "session task",
            false,
            None,
            SpawnMode::Session,
            false,
            None,
            None,
            None,
            3, // remaining_spawn_depth
        )
        .await
        .expect("create_child_session should succeed");

    // validate_child_ownership should return Some for Session mode children
    let result = mgr
        .validate_child_ownership("parent-validate-session", &child_id)
        .await;
    let info =
        result.expect("validate_child_ownership should return Some for Session mode children");
    assert_eq!(info.session_id, child_id);
    assert_eq!(info.mode, SpawnMode::Session);
    assert_eq!(info.parent_session_id, "parent-validate-session");
}

#[tokio::test]
#[serial]
async fn test_create_child_session_allowed_tools_override() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));

    // Config with some tools listed
    let config = ResolvedAgentConfig {
        id: "tools-agent".to_string(),
        name: "tools-agent".to_string(),
        parent_id: None,
        model: Some("test-model".to_string()),
        workspace: None,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills: vec![],
        tools: vec!["ToolA".into(), "ToolB".into(), "ToolC".into()],
        disallowed_tools: vec![],
        subagents: SubagentsConfig::default(),
        permissions: None,
        source: ConfigSource::Merged,
    };

    register_parent_session(&mgr, "parent-tools", tmp.path().to_path_buf()).await;

    // Create child with allowed_tools override
    let allowed = vec!["ToolA".to_string(), "ToolC".to_string()];
    let child_id = mgr
        .create_child_session(
            &config,
            "parent-tools",
            1,
            "restricted task",
            false,
            None,
            SpawnMode::Run,
            false,
            Some(allowed),
            None,
            None,
            3, // remaining_spawn_depth
        )
        .await
        .expect("create_child_session with allowed_tools should succeed");

    // Child should be created successfully
    assert!(mgr.has_session(&child_id).await);
    assert_eq!(mgr.get_session_depth(&child_id).await, Some(1));
}

#[tokio::test]
#[serial]
async fn test_create_child_session_no_allowed_tools_preserves_config() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));

    let config = test_resolved_config("no-restrict", None);

    register_parent_session(&mgr, "parent-no-restrict", tmp.path().to_path_buf()).await;

    // Create child without allowed_tools (None) — should use config's tools as-is
    let child_id = mgr
        .create_child_session(
            &config,
            "parent-no-restrict",
            1,
            "normal task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3, // remaining_spawn_depth
        )
        .await
        .expect("create_child_session without allowed_tools should succeed");

    assert!(mgr.has_session(&child_id).await);
}

#[tokio::test]
#[serial]
async fn test_create_child_session_workspace_fallback_to_parent() {
    clear_global_prompt_state();

    // Set up manager with a workspace root.
    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));

    // Parent workspace: {tmp}/workspaces/parent-agent/default/
    let parent_workspace = tmp
        .path()
        .join("workspaces")
        .join("parent-agent")
        .join("default");
    std::fs::create_dir_all(&parent_workspace).unwrap();

    let config = test_resolved_config("child-agent", None);

    // Register parent session with the parent workspace as its workdir.
    register_parent_session(&mgr, "parent-ws", parent_workspace.clone()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-ws",
            1,
            "test workspace fallback",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3, // remaining_spawn_depth
        )
        .await
        .expect("create_child_session should succeed");

    // Verify child session exists
    assert!(mgr.has_session(&child_id).await);

    // Verify child workspace is a subdirectory of parent workspace.
    let child_cs = mgr
        .get_conversation_session(&child_id)
        .await
        .expect("child conversation session should exist");
    let child_workdir = child_cs.read().await.workdir().to_path_buf();
    assert!(
        child_workdir.starts_with(&parent_workspace),
        "child workdir {:?} should be under parent workspace {:?}",
        child_workdir,
        parent_workspace
    );
    // Verify the child agent_id appears in the path
    assert!(
        child_workdir.to_string_lossy().contains("child-agent"),
        "child workdir {:?} should contain child agent id",
        child_workdir
    );
}

#[tokio::test]
#[serial]
async fn test_create_child_session_workspace_uses_actual_user_id() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();

    // Set up MemoryStorage with a parent checkpoint that has a sender_id.
    let storage = Arc::new(crate::session::storage::memory::MemoryStorage::new());
    let parent_session_id = "parent-with-user";
    let parent_agent_id = "parent-agent";
    let actual_user_id = "ou_actual_user_123";
    let mut cp = SessionCheckpoint::new(parent_session_id.to_string());
    cp.sender_id = Some(actual_user_id.to_string());
    cp.agent_id = Some(parent_agent_id.to_string());
    storage.save_checkpoint(&cp).await.unwrap();

    let mgr = SessionManager::new(
        &test_config(),
        Some(storage),
        Some(tmp.path().to_path_buf()),
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );

    // Parent workspace: {tmp}/workspaces/parent-agent/default/
    let parent_workspace = tmp
        .path()
        .join("workspaces")
        .join(parent_agent_id)
        .join("default");
    std::fs::create_dir_all(&parent_workspace).unwrap();

    let config = test_resolved_config("child-agent", None);

    // Register parent session with the parent workspace as its workdir.
    register_parent_session(&mgr, parent_session_id, parent_workspace.clone()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            parent_session_id,
            1,
            "test user_id passing",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3, // remaining_spawn_depth
        )
        .await
        .expect("create_child_session should succeed");

    // Verify child workspace path contains the actual user_id.
    let child_cs = mgr
        .get_conversation_session(&child_id)
        .await
        .expect("child conversation session should exist");
    let child_workdir = child_cs.read().await.workdir().to_path_buf();
    assert!(
        child_workdir.to_string_lossy().contains(actual_user_id),
        "child workdir {:?} should contain actual user_id '{}'",
        child_workdir,
        actual_user_id
    );
    // Verify it does NOT contain the hardcoded "default" user_id.
    // The path should be: <parent_workspace>/<child_agent_id>/<actual_user_id>/
    let expected_suffix = format!("{}/{}", "child-agent", actual_user_id);
    assert!(
        child_workdir.to_string_lossy().ends_with(&expected_suffix),
        "child workdir {:?} should end with '{}/{}'",
        child_workdir,
        "child-agent",
        actual_user_id
    );
}

// ── Step 1.4: spawn-context injection tests ─────────────────────────────

/// Verify `build_spawn_context` produces the expected paragraph for
/// depth=1, remaining_depth=3 (allows further spawning).
#[test]
fn test_build_spawn_context_allows_spawning() {
    let ctx = SessionManager::build_spawn_context("parent-test", 1, 3, &SpawnMode::Run, false);
    assert!(ctx.contains("sub-agent"), "should declare sub-agent role");
    assert!(
        ctx.contains("Current depth: 1. Remaining spawn depth: 3"),
        "should contain depth info"
    );
    assert!(
        ctx.contains("results are automatically pushed back"),
        "should describe push-based communication"
    );
    assert!(
        ctx.contains("Do not poll for status"),
        "should forbid polling"
    );
    assert!(
        ctx.contains("Your effective maximum depth for children is 3"),
        "should show remaining spawn depth"
    );
    assert!(
        ctx.contains("Parent session: parent-test"),
        "should contain parent session id"
    );
    assert!(ctx.contains("Spawn mode: run"), "should contain spawn mode");
    assert!(ctx.contains("Fork: false"), "should contain fork flag");
}

/// Verify `build_spawn_context` omits spawn guidance when remaining_depth == 0.
#[test]
fn test_build_spawn_context_no_spawning_at_limit() {
    let ctx = SessionManager::build_spawn_context("parent-test", 3, 0, &SpawnMode::Run, false);
    assert!(ctx.contains("sub-agent"));
    assert!(
        ctx.contains("Current depth: 3. Remaining spawn depth: 0"),
        "should contain depth info"
    );
    assert!(
        !ctx.contains("effective maximum depth"),
        "should NOT include spawn guidance when remaining is 0"
    );
}

/// Verify `build_spawn_context` at depth=0, remaining_depth=1 (allows spawning).
#[test]
fn test_build_spawn_context_depth_zero() {
    let ctx = SessionManager::build_spawn_context("parent-test", 0, 1, &SpawnMode::Run, false);
    assert!(ctx.contains("Current depth: 0. Remaining spawn depth: 1"));
    assert!(
        ctx.contains("Your effective maximum depth for children is 1"),
        "should show remaining depth"
    );
}

/// Verify the behavioral constraints section is always present.
#[test]
fn test_build_spawn_context_behavioral_constraints() {
    let ctx = SessionManager::build_spawn_context("parent-test", 0, 1, &SpawnMode::Run, false);
    assert!(
        ctx.contains("Behavioral constraints"),
        "should have behavioral constraints header"
    );
    assert!(
        ctx.contains("Trust push-based completion"),
        "should trust push-based notifications"
    );
    assert!(
        ctx.contains("do not ask for confirmation"),
        "should forbid confirmation-seeking"
    );
}

// ── Step 1.4: child session spawn-context integration test ───────────────

/// Integration test: `create_child_session` appends spawn context to
/// the system prompt so the child agent knows its role and limits.
#[tokio::test]
#[serial]
async fn test_child_session_system_prompt_contains_spawn_context() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("child-agent", None);

    register_parent_session(&mgr, "parent-prompt", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-prompt",
            2,
            "test task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            4, // max_spawn_depth
        )
        .await
        .expect("create_child_session should succeed");

    let cs = mgr
        .get_conversation_session(&child_id)
        .await
        .expect("child session should exist");
    let guard = cs.read().await;

    // Spawn context is now injected into the first user message
    // (not the system prompt) per the design doc.
    let first_msg = &guard.get_pending_messages()[0].content;
    assert!(
        first_msg.contains("sub-agent"),
        "first user message should contain sub-agent role declaration"
    );
    assert!(
        first_msg.contains("Current depth: 2. Remaining spawn depth: 4"),
        "first user message should contain depth info"
    );
    assert!(
        first_msg.contains("results are automatically pushed back"),
        "first user message should describe push-based communication"
    );
}

// ── Step 1.2: structured output guidance tests ─────────────────────────────

/// Verify `build_spawn_context` includes the structured output guidance
/// section (per design doc §结构化输出) when remaining_depth > 0.
#[test]
fn test_build_spawn_context_structured_output_guidance() {
    let ctx = SessionManager::build_spawn_context("parent-test", 1, 2, &SpawnMode::Run, false);
    assert!(
        ctx.contains("Structured output (optional)"),
        "should contain structured output header"
    );
    assert!(
        ctx.contains("Task scope"),
        "should contain Task scope section"
    );
    assert!(
        ctx.contains("Execution results"),
        "should contain Execution results section"
    );
    assert!(
        ctx.contains("Files involved"),
        "should contain Files involved section"
    );
    assert!(
        ctx.contains("File changes"),
        "should contain File changes section"
    );
    assert!(
        ctx.contains("Issues found"),
        "should contain Issues found section"
    );
}

/// Verify the structured output guidance explicitly states it is optional
/// and that the child may reply freely.
#[test]
fn test_build_spawn_context_structured_output_is_optional() {
    let ctx = SessionManager::build_spawn_context("parent-test", 0, 1, &SpawnMode::Run, false);
    assert!(
        ctx.contains("suggestion"),
        "should indicate structured output is a suggestion"
    );
    assert!(
        ctx.contains("may reply freely"),
        "should state the child may reply freely"
    );
}

/// Verify structured output guidance is present even at remaining_depth == 0
/// (it is independent of spawn depth — it always applies).
#[test]
fn test_build_spawn_context_structured_output_at_depth_limit() {
    let ctx = SessionManager::build_spawn_context("parent-test", 3, 0, &SpawnMode::Run, false);
    assert!(
        ctx.contains("Structured output (optional)"),
        "structured output should be present even at spawn depth limit"
    );
    assert!(
        ctx.contains("Task scope"),
        "Task scope should be present at depth limit"
    );
}

// ── Step 1.4: communication config tests ─────────────────────────────────

/// Verify the child session's communication config restricts
/// communication to the parent agent only.
#[tokio::test]
#[serial]
async fn test_child_session_communication_config_has_parent() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("comm-child", None);

    register_parent_session(&mgr, "parent-comm", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-comm",
            1,
            "comm task",
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
    let comm = guard
        .communication_config()
        .expect("communication_config should be set");

    // Outbound should contain parent agent id
    assert_eq!(comm.outbound.len(), 1);
    // The parent agent id is looked up via get_chat_id which reads
    // from sessions. The parent session was registered with
    // agent_id="parent-agent" by the test setup, so the comm config
    // should reference that.
    assert!(
        comm.outbound.contains(&"parent-agent".to_string()),
        "outbound should contain parent agent id, got: {:?}",
        comm.outbound
    );

    // Inbound should also contain parent agent id
    assert_eq!(comm.inbound.len(), 1);
    assert!(
        comm.inbound.contains(&"parent-agent".to_string()),
        "inbound should contain parent agent id, got: {:?}",
        comm.inbound
    );
}

/// Verify a non-spawn session does NOT have communication config.
#[test]
fn test_non_spawn_session_no_communication_config() {
    use std::path::PathBuf;
    let cs = crate::llm::session::ConversationSession::new(
        "regular-session".to_string(),
        "test-model".to_string(),
        PathBuf::from("/tmp"),
    );
    assert!(
        cs.communication_config().is_none(),
        "non-spawn session should not have communication_config"
    );
}

/// Verify `CommunicationConfig::default_with_parent` with a valid parent id.
#[test]
fn test_communication_config_default_with_parent() {
    use crate::agent::communication::CommunicationConfig;

    let config = CommunicationConfig::default_with_parent(Some("agent-abc"));
    assert_eq!(config.outbound, vec!["agent-abc".to_string()]);
    assert_eq!(config.inbound, vec!["agent-abc".to_string()]);
}

/// Verify `CommunicationConfig::default_with_parent` with None parent.
#[test]
fn test_communication_config_default_with_parent_none() {
    use crate::agent::communication::CommunicationConfig;

    let config = CommunicationConfig::default_with_parent(None);
    assert!(config.outbound.is_empty());
    assert!(config.inbound.is_empty());
}

/// Verify `can_send_to` / `can_receive_from` respect the whitelist.
#[test]
fn test_communication_config_permission_checks() {
    use crate::agent::communication::CommunicationConfig;

    let config = CommunicationConfig::default_with_parent(Some("parent-1"));
    assert!(config.can_send_to("parent-1"));
    assert!(!config.can_send_to("other-agent"));
    assert!(config.can_receive_from("parent-1"));
    assert!(!config.can_receive_from("other-agent"));
}
