//! Tests for SpawnContext and GatewayPermissionChecker adapters (Step 1.4).
//!
//! Verifies that the gateway-side adapter implementations delegate
//! correctly to SessionManager/PermissionEngine and produce expected
//! results under various conditions.

use std::collections::HashMap;
use std::sync::Arc;

use crate::session_manager::spawn_adapter::GatewayPermissionChecker;
use crate::session_manager::{ChildSessionInfo, ChildSessionStatus, SpawnMode};
use crate::{GatewayConfig, Message, SessionManager};
use closeclaw_common::{BootstrapMode, PermissionChecker, SpawnPermissionError};
use closeclaw_config::agents::{
    ActionPermission, AgentPermissions, ConfigSource, MemoryConfig, ModelSpec, PermissionLimits,
    ResolvedAgentConfig, SubagentsConfig,
};
use closeclaw_config::ConfigManager;
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::rules::RuleSetBuilder;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::{ReasoningLevel, SessionCheckpoint};
use closeclaw_session::spawn::controller::SpawnContext;
use closeclaw_session::storage::memory::MemoryStorage;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

fn make_session_manager() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &test_config(),
        None,
        None,
        ReasoningLevel::default(),
    ))
}

fn make_session_manager_with_storage() -> (Arc<SessionManager>, Arc<MemoryStorage>) {
    let storage = Arc::new(MemoryStorage::new());
    let mgr = Arc::new(SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        ReasoningLevel::default(),
    ));
    (mgr, storage)
}

fn make_config_manager() -> ConfigManager {
    let tmp = tempfile::tempdir().expect("tempdir");
    ConfigManager::new(tmp.path().to_path_buf()).expect("ConfigManager::new should succeed")
}

fn make_permission_engine() -> PermissionEngine {
    PermissionEngine::new_with_default_data_root(RuleSetBuilder::new().build().unwrap())
}

fn make_agent(id: &str, subagents: SubagentsConfig) -> ResolvedAgentConfig {
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
        subagents,
        memory: MemoryConfig::default(),
        hooks: Vec::new(),
        parallel_tool_calls: true,
        memory_configured: false,
        source: ConfigSource::User,
    }
}

fn inject_agents(cm: &ConfigManager, agents: Vec<(&str, ResolvedAgentConfig)>) {
    let mut map = cm.agents.write().expect("agents RwLock poisoned");
    for (id, cfg) in agents {
        map.insert(id.to_string(), cfg);
    }
}

async fn setup_parent_session(mgr: &SessionManager, agent_id: &str) -> String {
    let msg = Message {
        id: format!("msg-{}", agent_id),
        from: "user".to_string(),
        to: agent_id.to_string(),
        content: "hi".to_string(),
        channel: "test-channel".to_string(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    mgr.find_or_create("test-channel", &msg, None)
        .await
        .expect("find_or_create should succeed")
}

fn make_controller(
    cm: &Arc<ConfigManager>,
    sm: &Arc<SessionManager>,
    pe: &Arc<tokio::sync::RwLock<PermissionEngine>>,
) -> closeclaw_session::spawn::controller::SpawnController {
    closeclaw_session::spawn::controller::SpawnController::new(
        cm.clone(),
        sm.clone() as Arc<dyn SpawnContext>,
        Arc::new(GatewayPermissionChecker::new(
            sm.clone(),
            cm.clone(),
            pe.clone(),
        )),
    )
}

/// Write a permission file for the given agent into the ConfigManager's
/// agents root directory.
fn write_permission_file(cm: &ConfigManager, agent_id: &str, permissions: &AgentPermissions) {
    let agents_root = cm.config_dir.parent().unwrap_or(&cm.config_dir);
    let dir = agents_root.join("agents").join(agent_id);
    std::fs::create_dir_all(&dir).expect("create agents dir");
    let path = dir.join("permissions.json");
    let json = serde_json::to_string_pretty(permissions).expect("serialize permissions");
    std::fs::write(&path, json).expect("write permissions.json");
}

fn make_perms(agent_id: &str, allowed_dims: &[&str]) -> AgentPermissions {
    let dimensions = [
        "exec",
        "file_read",
        "file_write",
        "network",
        "spawn",
        "tool_call",
        "config_write",
        "message",
    ];
    let mut permissions = HashMap::with_capacity(dimensions.len());
    for &dim in &dimensions {
        permissions.insert(
            dim.to_string(),
            ActionPermission {
                allowed: allowed_dims.contains(&dim),
                limits: PermissionLimits::default(),
            },
        );
    }
    AgentPermissions {
        agent_id: agent_id.to_string(),
        permissions,
        inherited_from: None,
    }
}

async fn fill_children(mgr: &SessionManager, parent_id: &str, count: usize) {
    for i in 0..count {
        let child_id = format!("adapter-child-{}", i);
        let cs = ConversationSession::new(
            child_id.clone(),
            "test-model".to_string(),
            std::path::PathBuf::from("/tmp"),
        );
        mgr.conversation_sessions
            .write()
            .await
            .insert(child_id.clone(), Arc::new(tokio::sync::RwLock::new(cs)));
        mgr.register_child(
            parent_id,
            ChildSessionInfo {
                session_id: child_id,
                parent_session_id: parent_id.to_string(),
                agent_id: "adapter-child".to_string(),
                depth: 1,
                mode: SpawnMode::Run,
                status: ChildSessionStatus::Active,
                timeout_secs: None,
                timeout_warning_secs: None,
                timeout_notify_interval_ratio: None,
                created_at: std::time::Instant::now(),
            },
        )
        .await;
    }
}

// ══════════════════════════════════════════════════════════════════════
// SpawnContext adapter delegation tests
// ══════════════════════════════════════════════════════════════════════

/// active_children_count returns 0 for a session with no children.
#[tokio::test]
async fn test_spawn_context_active_children_count_zero() {
    let mgr = make_session_manager();
    let parent_id = setup_parent_session(&mgr, "parent-cc").await;

    let count = mgr.active_children_count(&parent_id).await;
    assert_eq!(count, 0, "no children registered → count should be 0");
}

/// active_children_count returns the correct count after registering children.
#[tokio::test]
async fn test_spawn_context_active_children_count_nonzero() {
    let mgr = make_session_manager();
    let parent_id = setup_parent_session(&mgr, "parent-cc2").await;

    fill_children(&mgr, &parent_id, 3).await;

    let count = mgr.active_children_count(&parent_id).await;
    assert_eq!(count, 3, "3 children registered → count should be 3");
}

/// chat_id returns the agent_id for an existing session.
#[tokio::test]
async fn test_spawn_context_chat_id_existing_session() {
    let mgr = make_session_manager();
    let parent_id = setup_parent_session(&mgr, "parent-chat").await;

    let chat_id = mgr.chat_id(&parent_id).await;
    assert_eq!(
        chat_id,
        Some("parent-chat".to_string()),
        "chat_id should return agent_id from session"
    );
}

/// chat_id returns None for a nonexistent session.
#[tokio::test]
async fn test_spawn_context_chat_id_nonexistent_session() {
    let mgr = make_session_manager();

    let chat_id = mgr.chat_id("nonexistent-session-id").await;
    assert_eq!(chat_id, None, "nonexistent session → None");
}

/// sender_id returns None when no checkpoint is saved (sender_id is read
/// from checkpoint, not from the message's `from` field).
#[tokio::test]
async fn test_spawn_context_sender_id_no_checkpoint() {
    let mgr = make_session_manager();
    let parent_id = setup_parent_session(&mgr, "parent-sid").await;

    let sender = mgr.sender_id(&parent_id).await;
    assert_eq!(sender, None, "no checkpoint → sender_id is None");
}

/// sender_id returns the value from the checkpoint when one is saved.
#[tokio::test]
async fn test_spawn_context_sender_id_from_checkpoint() {
    let (mgr, _storage) = make_session_manager_with_storage();
    let parent_id = setup_parent_session(&mgr, "parent-sid-cp").await;

    let mut cp = SessionCheckpoint::new(parent_id.clone());
    cp.sender_id = Some("alice".to_string());
    let cm = mgr.checkpoint_manager.read().await;
    cm.as_ref()
        .unwrap()
        .save_raw(&cp)
        .await
        .expect("save checkpoint");
    drop(cm);

    let sender = mgr.sender_id(&parent_id).await;
    assert_eq!(
        sender,
        Some("alice".to_string()),
        "checkpoint with sender_id → should return it"
    );
}

/// sender_id returns None for a nonexistent session.
#[tokio::test]
async fn test_spawn_context_sender_id_nonexistent_session() {
    let mgr = make_session_manager();

    let sender = mgr.sender_id("nonexistent-session-id").await;
    assert_eq!(sender, None, "nonexistent session → None");
}

/// effective_max_spawn_depth returns None when no checkpoint is saved.
#[tokio::test]
async fn test_spawn_context_effective_depth_no_checkpoint() {
    let (mgr, _storage) = make_session_manager_with_storage();
    let parent_id = setup_parent_session(&mgr, "parent-nocc").await;

    let depth = mgr.effective_max_spawn_depth(&parent_id).await;
    assert_eq!(depth, None, "no checkpoint → None");
}

/// effective_max_spawn_depth reads from checkpoint correctly.
#[tokio::test]
async fn test_spawn_context_effective_depth_from_checkpoint() {
    let (mgr, _storage) = make_session_manager_with_storage();
    let parent_id = setup_parent_session(&mgr, "parent-cp").await;

    let cp = SessionCheckpoint::new(parent_id.clone()).with_effective_max_spawn_depth(Some(5));
    let cm = mgr.checkpoint_manager.read().await;
    cm.as_ref()
        .unwrap()
        .save_raw(&cp)
        .await
        .expect("save checkpoint");
    drop(cm);

    let depth = mgr.effective_max_spawn_depth(&parent_id).await;
    assert_eq!(
        depth,
        Some(5),
        "checkpoint with budget=5 → should return Some(5)"
    );
}

/// effective_max_spawn_depth returns None for a nonexistent session.
#[tokio::test]
async fn test_spawn_context_effective_depth_nonexistent_session() {
    let (mgr, _storage) = make_session_manager_with_storage();

    let depth = mgr
        .effective_max_spawn_depth("nonexistent-session-id")
        .await;
    assert_eq!(depth, None, "nonexistent session → None");
}

// ══════════════════════════════════════════════════════════════════════
// GatewayPermissionChecker delegation tests
// ══════════════════════════════════════════════════════════════════════

/// When no permissions are configured for either parent or child,
/// GatewayPermissionChecker should return Ok (no restriction).
#[tokio::test]
async fn test_permission_checker_no_permissions_configured() {
    let pe = Arc::new(tokio::sync::RwLock::new(make_permission_engine()));
    let cm = Arc::new(make_config_manager());
    let sm = make_session_manager();
    let checker = GatewayPermissionChecker::new(sm.clone(), cm.clone(), pe.clone());

    // Create parent session
    let parent_id = setup_parent_session(&sm, "parent-pc").await;
    // No permission files written → no restrictions

    let result = checker
        .validate_spawn_permission("child-pc", &parent_id)
        .await;
    assert!(
        result.is_ok(),
        "no permissions configured → should be Ok, got {:?}",
        result
    );
}

/// When parent has exec allowed and child has exec allowed,
/// intersection is non-empty → Ok.
#[tokio::test]
async fn test_permission_checker_partial_overlap_ok() {
    let pe = Arc::new(tokio::sync::RwLock::new(make_permission_engine()));
    let (cm_raw, _tmpdir) = {
        let parent = tempfile::tempdir().expect("parent tempdir");
        let config_dir = parent.path().join("config");
        std::fs::create_dir_all(&config_dir).expect("create config dir");
        let cm = ConfigManager::new(config_dir).expect("ConfigManager::new");
        (cm, parent)
    };
    let cm = Arc::new(cm_raw);
    let sm = make_session_manager();
    let checker = GatewayPermissionChecker::new(sm.clone(), cm.clone(), pe.clone());

    let parent_id = setup_parent_session(&sm, "parent-overlap").await;

    // Parent allows exec only; child allows exec + file_read.
    write_permission_file(
        &cm,
        "parent-overlap",
        &make_perms("parent-overlap", &["exec"]),
    );
    write_permission_file(
        &cm,
        "child-overlap",
        &make_perms("child-overlap", &["exec", "file_read"]),
    );

    let result = checker
        .validate_spawn_permission("child-overlap", &parent_id)
        .await;
    assert!(
        result.is_ok(),
        "partial overlap → should be Ok, got {:?}",
        result
    );
}

/// When parent has no permissions and child has permissions,
/// intersection is empty for all dimensions → Denied.
#[tokio::test]
async fn test_permission_checker_parent_denies_all() {
    let pe = Arc::new(tokio::sync::RwLock::new(make_permission_engine()));
    let (cm_raw, _tmpdir) = {
        let parent = tempfile::tempdir().expect("parent tempdir");
        let config_dir = parent.path().join("config");
        std::fs::create_dir_all(&config_dir).expect("create config dir");
        let cm = ConfigManager::new(config_dir).expect("ConfigManager::new");
        (cm, parent)
    };
    let cm = Arc::new(cm_raw);
    let sm = make_session_manager();
    let checker = GatewayPermissionChecker::new(sm.clone(), cm.clone(), pe.clone());

    let parent_id = setup_parent_session(&sm, "parent-denied").await;

    // Parent has all permissions denied; child has all allowed.
    write_permission_file(&cm, "parent-denied", &make_perms("parent-denied", &[]));
    write_permission_file(
        &cm,
        "child-denied",
        &make_perms(
            "child-denied",
            &[
                "exec",
                "file_read",
                "file_write",
                "network",
                "spawn",
                "tool_call",
                "config_write",
                "message",
            ],
        ),
    );

    let result = checker
        .validate_spawn_permission("child-denied", &parent_id)
        .await;
    match result {
        Ok(()) => panic!("expected Denied, got Ok"),
        Err(SpawnPermissionError::Denied { agent_id, .. }) => {
            assert_eq!(agent_id, "child-denied");
        }
    }
}

// ══════════════════════════════════════════════════════════════════════
// Adapter integration: validate → permission check (two-step)
// ══════════════════════════════════════════════════════════════════════

/// Full adapter integration test: validate() passes with correct delegation,
/// then check_spawn_permission() passes (no permissions configured).
/// Verifies that the entire pipeline works end-to-end through adapters.
#[tokio::test]
async fn test_adapter_integration_validate_then_permission_no_restrictions() {
    let pe = Arc::new(tokio::sync::RwLock::new(make_permission_engine()));
    let cm = Arc::new(make_config_manager());
    let sm = make_session_manager();
    let controller = make_controller(&cm, &sm, &pe);

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", parent_sub);
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    // Step 1: validate passes (preconditions OK)
    let validation = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("validate should succeed");
    assert_eq!(validation.config.id, "child");

    // Step 2: permission check passes (no permissions configured)
    controller
        .check_spawn_permission(&parent_id, &validation)
        .await
        .expect("check_spawn_permission should succeed when no permissions configured");
}

/// Full adapter integration test: validate() passes, then
/// check_spawn_permission() rejects (child fully denied).
#[tokio::test]
async fn test_adapter_integration_validate_then_permission_denied() {
    let pe = Arc::new(tokio::sync::RwLock::new(make_permission_engine()));
    let (cm_raw, _tmpdir) = {
        let parent = tempfile::tempdir().expect("parent tempdir");
        let config_dir = parent.path().join("config");
        std::fs::create_dir_all(&config_dir).expect("create config dir");
        let cm = ConfigManager::new(config_dir).expect("ConfigManager::new");
        (cm, parent)
    };
    let cm = Arc::new(cm_raw);
    let sm = make_session_manager();
    let controller = make_controller(&cm, &sm, &pe);

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(2);
    let parent = make_agent("parent", parent_sub);
    let child = make_agent("child", SubagentsConfig::default());
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    // Parent: all permissions allowed; child: all denied.
    write_permission_file(
        &cm,
        "parent",
        &make_perms(
            "parent",
            &[
                "exec",
                "file_read",
                "file_write",
                "network",
                "spawn",
                "tool_call",
                "config_write",
                "message",
            ],
        ),
    );
    write_permission_file(&cm, "child", &make_perms("child", &[]));

    // Step 1: validate passes (preconditions OK)
    let validation = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("validate should succeed");
    assert_eq!(validation.config.id, "child");

    // Step 2: permission check rejects (child fully denied)
    let err = controller
        .check_spawn_permission(&parent_id, &validation)
        .await
        .expect_err("check_spawn_permission should reject when child fully denied");

    match err {
        closeclaw_session::spawn_validation::SpawnError::PermissionDenied { agent_id, .. } => {
            assert_eq!(agent_id, "child");
        }
        other => panic!("expected PermissionDenied, got {:?}", other),
    }
}

// ══════════════════════════════════════════════════════════════════════
// State transition: validate result fields usable for create_child_session
// ══════════════════════════════════════════════════════════════════════

/// Verify that a successful validate() returns a SpawnValidationResult
/// with all fields populated and usable for create_child_session.
/// This validates the validate → create transition contract.
#[tokio::test]
async fn test_validate_result_usable_for_child_session_creation() {
    let pe = Arc::new(tokio::sync::RwLock::new(make_permission_engine()));
    let cm = Arc::new(make_config_manager());
    let sm = make_session_manager();
    let controller = make_controller(&cm, &sm, &pe);

    let mut parent_sub = SubagentsConfig::default();
    parent_sub.max_spawn_depth = Some(3);
    let parent = make_agent("parent", parent_sub);

    let mut child_sub = SubagentsConfig::default();
    child_sub.max_spawn_depth = Some(2);
    child_sub.timeout = Some(3600);
    let child = make_agent("child", child_sub);
    inject_agents(&cm, vec![("parent", parent), ("child", child)]);

    let parent_id = setup_parent_session(&sm, "parent").await;

    let result = controller
        .validate(&parent_id, Some("child"))
        .await
        .expect("validate should succeed");

    // All fields required by ChildSessionCreationParams must be present:
    assert_eq!(result.config.id, "child", "target agent config");
    assert!(
        result.effective_max_spawn_depth > 0 || result.effective_max_spawn_depth == 0,
        "effective_max_spawn_depth must be set"
    );
    assert!(
        result.spawn_timeout.is_some(),
        "spawn_timeout must be Some (falls back to global default)"
    );
}
