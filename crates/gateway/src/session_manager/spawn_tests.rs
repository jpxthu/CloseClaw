//! Tests for `SessionManager::create_child_session` and the children tracking table.
//!
//! These tests live in a separate module to keep `session_manager.rs` and
//! `tests.rs` under the project's 500-line file limit. Shared helpers
//! (`make_test_mgr`, `clear_global_prompt_state`) are re-exported by
//! `super::tests` at `pub(super)` visibility.

use super::spawn::SpawnMode;
use super::tests::{clear_global_prompt_state, make_test_mgr, test_config};
use super::SessionManager;
use closeclaw_common::BootstrapMode;
use closeclaw_config::agents::SubagentsConfig;
use closeclaw_config::agents::{ConfigSource, MemoryConfig, ModelSpec, ResolvedAgentConfig};
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::{PersistenceService, ReasoningLevel, SessionCheckpoint};
use serial_test::serial;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

fn test_resolved_config(id: &str, workspace: Option<PathBuf>) -> ResolvedAgentConfig {
    ResolvedAgentConfig {
        id: id.to_string(),
        name: id.to_string(),
        parent_id: None,
        model: Some(ModelSpec::single("test-model")),
        workspace,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills: vec![],
        tools: vec![],
        disallowed_tools: vec![],
        subagents: SubagentsConfig::default(),
        memory: MemoryConfig::default(),
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
        crate::Session {
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
            3,
            None, // spawn_timeout,
            None, // label
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
            3,
            None, // spawn_timeout,
            None, // label
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
            3,
            None, // spawn_timeout,
            None, // label
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
            3,
            None, // spawn_timeout,
            None, // label
        )
        .await
        .expect("create_child_session should succeed");

    // Direct lookup via children table
    let children = mgr.children.read().await;
    let list = children.list_children("parent-7");
    assert!(!list.is_empty(), "parent-7 should have a children entry");
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
            3,
            None, // spawn_timeout,
            None, // label
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
            3,
            None, // spawn_timeout,
            None, // label
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
        children.list_children("parent-kill").is_empty(),
        "parent entry should be removed from children table when list is empty"
    );
}

#[tokio::test]
#[serial]
async fn test_validate_child_ownership_by_mode() {
    clear_global_prompt_state();

    // --- Run mode: should return Some with correct info ---
    {
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
                3,
                None, // spawn_timeout,
                None, // label
            )
            .await
            .expect("create_child_session should succeed");
        let result = mgr
            .validate_child_ownership("parent-validate-run", &child_id)
            .await;
        let info =
            result.expect("validate_child_ownership should return Some for Run mode children");
        assert_eq!(info.session_id, child_id);
        assert_eq!(info.mode, SpawnMode::Run);
        assert_eq!(info.parent_session_id, "parent-validate-run");
    }

    // --- Session mode: should still return Some with correct info ---
    {
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
                3,
                None, // spawn_timeout,
                None, // label
            )
            .await
            .expect("create_child_session should succeed");
        let result = mgr
            .validate_child_ownership("parent-validate-session", &child_id)
            .await;
        let info =
            result.expect("validate_child_ownership should return Some for Session mode children");
        assert_eq!(info.session_id, child_id);
        assert_eq!(info.mode, SpawnMode::Session);
        assert_eq!(info.parent_session_id, "parent-validate-session");
    }
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
        model: Some(ModelSpec::single("test-model")),
        workspace: None,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills: vec![],
        tools: vec!["ToolA".into(), "ToolB".into(), "ToolC".into()],
        disallowed_tools: vec![],
        subagents: SubagentsConfig::default(),
        memory: MemoryConfig::default(),
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
            3,
            None, // spawn_timeout,
            None, // label
        )
        .await
        .expect("create_child_session with allowed_tools should succeed");
    assert!(mgr.has_session(&child_id).await);
    assert_eq!(mgr.get_session_depth(&child_id).await, Some(1));

    // Create child without allowed_tools (None) — should use config's tools as-is
    let child_id_2 = mgr
        .create_child_session(
            &config,
            "parent-tools",
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
            None, // spawn_timeout,
            None, // label
        )
        .await
        .expect("create_child_session without allowed_tools should succeed");
    assert!(mgr.has_session(&child_id_2).await);
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
            3,
            None, // spawn_timeout,
            None, // label
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
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());
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
            3,
            None, // spawn_timeout,
            None, // label
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
/// depth=1, max_spawn_depth=3 (allows further spawning).
#[test]
fn test_build_spawn_context_allows_spawning() {
    let ctx = SessionManager::build_spawn_context(1, 3, "parent-abc", &SpawnMode::Run, false);
    assert!(ctx.contains("sub-agent"), "should declare sub-agent role");
    assert!(ctx.contains("**depth**: 1 / **max_spawn_depth**: 3"));
    assert!(ctx.contains("results are automatically pushed back"));
    assert!(ctx.contains("Do not poll for status"));
    assert!(ctx.contains("Your effective maximum depth for children is 2"));
    assert!(ctx.contains("**parent_session_id**: parent-abc"));
    assert!(ctx.contains("**spawn_mode**: run"));
    assert!(ctx.contains("**fork**: false"));
}

/// Verify `build_spawn_context` omits spawn guidance when depth == max_spawn_depth.
#[test]
fn test_build_spawn_context_no_spawning_at_limit() {
    let ctx = SessionManager::build_spawn_context(3, 3, "parent-abc", &SpawnMode::Run, false);
    assert!(ctx.contains("sub-agent"));
    assert!(
        ctx.contains("**depth**: 3 / **max_spawn_depth**: 3"),
        "should contain depth info"
    );
    assert!(
        !ctx.contains("effective maximum depth"),
        "should NOT include spawn guidance at limit"
    );
}

/// Verify `build_spawn_context` at depth=0, max_spawn_depth=1 (allows spawning).
#[test]
fn test_build_spawn_context_depth_zero() {
    let ctx = SessionManager::build_spawn_context(0, 1, "parent-abc", &SpawnMode::Run, false);
    assert!(ctx.contains("**depth**: 0 / **max_spawn_depth**: 1"));
    assert!(
        ctx.contains("Your effective maximum depth for children is 1"),
        "should show effective depth (1-0=1)"
    );
}

/// Verify the behavioral constraints section is always present.
#[test]
fn test_build_spawn_context_behavioral_constraints() {
    let ctx = SessionManager::build_spawn_context(0, 1, "parent-abc", &SpawnMode::Session, true);
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
    assert!(
        ctx.contains("**spawn_mode**: session"),
        "should reflect session spawn mode"
    );
    assert!(ctx.contains("**fork**: true"), "should reflect fork=true");
}

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
            4,
            None, // spawn_timeout,
            None, // label
        )
        .await
        .expect("create_child_session should succeed");
    let cs = mgr
        .get_conversation_session(&child_id)
        .await
        .expect("child session should exist");
    let guard = cs.read().await;
    let prompt = guard.system_prompt().expect("system prompt should be set");
    assert!(
        prompt.contains("sub-agent"),
        "system prompt should contain sub-agent role declaration"
    );
    assert!(
        prompt.contains("**depth**: 2 / **max_spawn_depth**: 4"),
        "system prompt should contain depth info"
    );
    assert!(
        prompt.contains("results are automatically pushed back"),
        "system prompt should describe push-based communication"
    );
}
// ── Step 1.2: structured output guidance tests ───────────────────────────

/// Verify `build_spawn_context` includes the structured output guidance
/// section (per design doc §结构化输出) when remaining_depth > 0.
#[test]
fn test_build_spawn_context_structured_output_guidance() {
    let ctx = SessionManager::build_spawn_context(1, 2, "parent-abc", &SpawnMode::Run, false);
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
    let ctx = SessionManager::build_spawn_context(0, 1, "parent-abc", &SpawnMode::Run, false);
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
    let ctx = SessionManager::build_spawn_context(3, 0, "parent-abc", &SpawnMode::Run, false);
    assert!(
        ctx.contains("Structured output (optional)"),
        "structured output should be present even at spawn depth limit"
    );
    assert!(
        ctx.contains("Task scope"),
        "Task scope should be present at depth limit"
    );
}

/// Verify the child session's communication config restricts
/// communication to the parent agent only, and persists to checkpoint.
#[tokio::test]
#[serial]
async fn test_child_session_communication_config_has_parent() {
    clear_global_prompt_state();
    let tmp = tempfile::TempDir::new().unwrap();
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        Some(tmp.path().to_path_buf()),
        ReasoningLevel::default(),
    );
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
            None, // spawn_timeout,
            None, // label
        )
        .await
        .expect("create_child_session should succeed");
    // Verify in-memory session has communication_config.
    let cs = mgr
        .get_conversation_session(&child_id)
        .await
        .expect("child session should exist");
    let guard = cs.read().await;
    let comm = guard
        .communication_config()
        .expect("communication_config should be set");
    assert_eq!(comm.outbound, vec!["parent-agent".to_string()]);
    assert_eq!(comm.inbound, vec!["parent-agent".to_string()]);
    drop(guard);
    // Verify checkpoint also has communication_config.
    let child_cp = storage
        .load_checkpoint(&child_id)
        .await
        .expect("storage should be accessible")
        .expect("child checkpoint should exist");
    let cp_comm = child_cp
        .communication_config
        .as_ref()
        .expect("checkpoint should have communication_config");
    assert_eq!(cp_comm.outbound, vec!["parent-agent".to_string()]);
    assert_eq!(cp_comm.inbound, vec!["parent-agent".to_string()]);
}

/// Verify a non-spawn session does NOT have communication config.
#[test]
fn test_non_spawn_session_no_communication_config() {
    use std::path::PathBuf;
    let cs = closeclaw_session::llm_session::ConversationSession::new(
        "regular-session".to_string(),
        "test-model".to_string(),
        PathBuf::from("/tmp"),
    );
    assert!(
        cs.communication_config().is_none(),
        "non-spawn session should not have communication_config"
    );
}

/// Verify `CommunicationConfig` construction and permission checks.
#[test]
fn test_communication_config_construction_and_permissions() {
    use crate::session_manager::communication::CommunicationConfig;

    // With valid parent
    let config = CommunicationConfig::default_with_parent(Some("agent-abc"));
    assert_eq!(config.outbound, vec!["agent-abc".to_string()]);
    assert_eq!(config.inbound, vec!["agent-abc".to_string()]);

    // With None parent
    let config = CommunicationConfig::default_with_parent(None);
    assert!(config.outbound.is_empty());
    assert!(config.inbound.is_empty());

    // Permission checks
    let config = CommunicationConfig::default_with_parent(Some("parent-1"));
    assert!(config.can_send_to("parent-1"));
    assert!(!config.can_send_to("other-agent"));
    assert!(config.can_receive_from("parent-1"));
    assert!(!config.can_receive_from("other-agent"));
}
// ── Step 1.3: spawn checkpoint parent_session_id + depth persistence ──

/// Verify that `create_child_session` persists a checkpoint with the
/// correct `parent_session_id` and `depth` values.
#[tokio::test]
#[serial]
async fn test_spawn_checkpoint_persists_parent_session_id_and_depth() {
    clear_global_prompt_state();
    let tmp = tempfile::TempDir::new().unwrap();
    // Use MemoryStorage so we can read back the checkpoint.
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());

    // Register a parent checkpoint in storage so the parent can be found.
    let parent_session_id = "parent-cp-check";
    let mut parent_cp = SessionCheckpoint::new(parent_session_id.to_string());
    parent_cp.depth = 0;
    parent_cp.parent_session_id = None;
    storage.save_checkpoint(&parent_cp).await.unwrap();
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        Some(tmp.path().to_path_buf()),
        ReasoningLevel::default(),
    );
    let config = test_resolved_config("cp-check-agent", None);
    register_parent_session(&mgr, parent_session_id, tmp.path().to_path_buf()).await;
    let child_id = mgr
        .create_child_session(
            &config,
            parent_session_id,
            2, // depth
            "checkpoint test task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            4,
            None, // spawn_timeout,
            None, // label
        )
        .await
        .expect("create_child_session should succeed");

    // Load the child checkpoint from storage and verify fields.
    let child_cp = storage
        .load_checkpoint(&child_id)
        .await
        .expect("storage should be accessible")
        .expect("child checkpoint should exist in storage");
    assert_eq!(
        child_cp.parent_session_id.as_deref(),
        Some(parent_session_id),
        "checkpoint parent_session_id should match the parent"
    );
    assert_eq!(
        child_cp.depth, 2,
        "checkpoint depth should match the depth passed to create_child_session"
    );
    assert_eq!(
        child_cp.agent_id.as_deref(),
        Some("cp-check-agent"),
        "checkpoint agent_id should match the config"
    );
}
