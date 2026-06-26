//! Unit tests for spawn depth budget propagation, kill all-mode, and
//! cascade termination (Step 1.5).

use std::sync::Arc;

use crate::agent::config::SubagentsConfig;
use crate::agent::spawn::{SpawnController, SpawnError};
use crate::config::agents::{ConfigSource, ResolvedAgentConfig};
use crate::config::ConfigManager;
use crate::gateway::session_manager::{ChildSessionInfo, SpawnMode};
use crate::gateway::{DmScope, GatewayConfig, SessionManager};
use crate::permission::engine::engine_eval::PermissionEngine;
use crate::permission::rules::RuleSetBuilder;
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::{PersistenceService, SessionCheckpoint};
use crate::session::storage::memory::MemoryStorage;
use crate::session::ReasoningLevel;

// ---------------------------------------------------------------------------
// Helpers (duplicated from spawn_tests.rs to keep this file self-contained)
// ---------------------------------------------------------------------------

fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
        ..Default::default()
    }
}

fn make_config_manager() -> ConfigManager {
    let tmp = tempfile::tempdir().expect("tempdir should be created");
    ConfigManager::new(tmp.path().to_path_buf()).expect("ConfigManager::new should succeed")
}

fn make_agent(id: &str, subagents: SubagentsConfig) -> ResolvedAgentConfig {
    ResolvedAgentConfig {
        id: id.to_string(),
        name: id.to_string(),
        parent_id: None,
        model: Some("test-model".to_string()),
        workspace: None,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills: vec![],
        tools: vec![],
        disallowed_tools: vec![],
        subagents,
        memory: None,
        source: ConfigSource::User,
    }
}

async fn setup_parent_session(mgr: &SessionManager, agent_id: &str) -> String {
    let msg = crate::gateway::Message {
        id: format!("msg-{}", agent_id),
        from: "user".to_string(),
        to: agent_id.to_string(),
        content: "hi".to_string(),
        channel: "test-channel".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
    };
    mgr.find_or_create("test-channel", &msg, None)
        .await
        .expect("find_or_create should succeed")
}

fn inject_agents(cm: &ConfigManager, agents: Vec<(&str, ResolvedAgentConfig)>) {
    let mut map = cm.agents.write().expect("agents RwLock poisoned");
    for (id, cfg) in agents {
        map.insert(id.to_string(), cfg);
    }
}

fn make_session_manager_with_memory_storage() -> (Arc<SessionManager>, Arc<MemoryStorage>) {
    let storage = Arc::new(MemoryStorage::new());
    let mgr = Arc::new(SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    (mgr, storage)
}

async fn save_checkpoint_with_budget(
    storage: &MemoryStorage,
    session_id: &str,
    depth: u32,
    effective_budget: Option<u32>,
    parent_session_id: Option<&str>,
) {
    let mut cp = SessionCheckpoint::new(session_id.to_string())
        .with_depth(depth)
        .with_effective_max_spawn_depth(effective_budget);
    if let Some(pid) = parent_session_id {
        cp = cp.with_parent_session_id(pid.to_string());
    }
    storage.save_checkpoint(&cp).await.unwrap();
}

// ══════════════════════════════════════════════════════════════════════
// Step 1.5: Depth budget propagation tests
// ══════════════════════════════════════════════════════════════════════

/// Verify effective budget read from checkpoint vs config fallback.
#[tokio::test]
async fn test_depth_budget_checkpoint_vs_config_fallback() {
    let cm = Arc::new(make_config_manager());
    let (sm, mem_storage) = make_session_manager_with_memory_storage();
    let controller = SpawnController::new(
        cm.clone(),
        sm.clone(),
        Arc::new(PermissionEngine::new_with_default_data_root(
            RuleSetBuilder::new().build().unwrap(),
        )),
    );

    // Root: maxSpawnDepth=3
    let mut root_sub = SubagentsConfig::default();
    root_sub.max_spawn_depth = 3;
    let root = make_agent("root", root_sub);
    let root_id = setup_parent_session(&sm, "root").await;
    // NO checkpoint saved → config fallback (3) is used

    let mut child_sub = SubagentsConfig::default();
    child_sub.max_spawn_depth = 1;
    let child = make_agent("child", child_sub);
    inject_agents(&cm, vec![("root", root), ("child", child)]);

    // Fallback: effective = min(1, 3-1) = 1
    let result = controller
        .validate(&root_id, Some("child"))
        .await
        .expect("should pass with config fallback");
    assert_eq!(result.effective_max_spawn_depth, 1);

    // Now save a checkpoint with effective budget = 1
    save_checkpoint_with_budget(&mem_storage, &root_id, 0, Some(1), None).await;

    // effective = min(1, 1-1) = 0, parent budget=1 > 0 → can create
    // Per design doc: effective=0 means child exists but cannot spawn further.
    let result2 = controller
        .validate(&root_id, Some("child"))
        .await
        .expect("should pass: parent budget > 0, child created with effective=0");
    assert_eq!(result2.effective_max_spawn_depth, 0);
}

/// Spawn allowed when parent effective budget > 0 (child effective budget
/// may be 0). Root(maxSpawnDepth=3) → child1(effective=1) →
/// child2(effective=0, exists but cannot spawn further).
#[tokio::test]
async fn test_depth_budget_allowed_when_effective_zero() {
    let cm = Arc::new(make_config_manager());
    let (sm, mem_storage) = make_session_manager_with_memory_storage();
    let controller = SpawnController::new(
        cm.clone(),
        sm.clone(),
        Arc::new(PermissionEngine::new_with_default_data_root(
            RuleSetBuilder::new().build().unwrap(),
        )),
    );

    let mut root_sub = SubagentsConfig::default();
    root_sub.max_spawn_depth = 3;
    let root = make_agent("root", root_sub);
    let root_id = setup_parent_session(&sm, "root").await;
    save_checkpoint_with_budget(&mem_storage, &root_id, 0, Some(3), None).await;

    // child1 at depth=1, effective budget=1 (only allows one more level)
    let mut child1_sub = SubagentsConfig::default();
    child1_sub.max_spawn_depth = 1;
    let child1 = make_agent("child1", child1_sub);
    inject_agents(&cm, vec![("root", root), ("child1", child1)]);

    // First spawn succeeds
    let result = controller
        .validate(&root_id, Some("child1"))
        .await
        .expect("should pass: effective=1, child_depth=1");
    assert_eq!(result.effective_max_spawn_depth, 1);

    // Simulate child1 created with effective budget = 1
    let child1_session_id = "child1-budget-zero";
    sm.sessions.write().await.insert(
        child1_session_id.to_string(),
        crate::gateway::Session {
            id: child1_session_id.to_string(),
            agent_id: "child1".to_string(),
            channel: "test".to_string(),
            created_at: 0,
            depth: 1,
        },
    );
    sm.register_child(
        &root_id,
        ChildSessionInfo {
            session_id: child1_session_id.to_string(),
            parent_session_id: root_id.to_string(),
            agent_id: "child1".to_string(),
            depth: 1,
            mode: SpawnMode::Run,
        },
    )
    .await;
    save_checkpoint_with_budget(&mem_storage, child1_session_id, 1, Some(1), Some(&root_id)).await;

    // child2 attempt: effective = min(1, 1-1) = 0
    // Per design doc: child with effective=0 can be created (exists in tree)
    // but cannot spawn further children.
    let mut child2_sub = SubagentsConfig::default();
    child2_sub.max_spawn_depth = 1;
    let child2 = make_agent("child2", child2_sub);
    inject_agents(&cm, vec![("child2", child2)]);

    let result2 = controller
        .validate(child1_session_id, Some("child2"))
        .await
        .expect("should pass: parent budget > 0, child created with effective=0");
    assert_eq!(result2.effective_max_spawn_depth, 0);
}

/// Child maxSpawnDepth narrows via min: parent has large budget but
/// child's config limits it.
#[tokio::test]
async fn test_depth_budget_child_narrows_via_min() {
    let cm = Arc::new(make_config_manager());
    let (sm, mem_storage) = make_session_manager_with_memory_storage();
    let controller = SpawnController::new(
        cm.clone(),
        sm.clone(),
        Arc::new(PermissionEngine::new_with_default_data_root(
            RuleSetBuilder::new().build().unwrap(),
        )),
    );

    let mut root_sub = SubagentsConfig::default();
    root_sub.max_spawn_depth = 5;
    let root = make_agent("root", root_sub);
    let root_id = setup_parent_session(&sm, "root").await;
    save_checkpoint_with_budget(&mem_storage, &root_id, 0, Some(5), None).await;

    // child: maxSpawnDepth=2 — effective = min(2, 5-1) = 2
    let mut child_sub = SubagentsConfig::default();
    child_sub.max_spawn_depth = 2;
    let child = make_agent("narrow-child", child_sub);
    inject_agents(&cm, vec![("root", root), ("narrow-child", child)]);

    let result = controller
        .validate(&root_id, Some("narrow-child"))
        .await
        .expect("should pass: effective=2, child_depth=1");
    assert_eq!(result.effective_max_spawn_depth, 2);
    assert_eq!(result.config.id, "narrow-child");

    // Simulate child created with effective budget = 2
    let child_session_id = "narrow-child-session";
    sm.sessions.write().await.insert(
        child_session_id.to_string(),
        crate::gateway::Session {
            id: child_session_id.to_string(),
            agent_id: "narrow-child".to_string(),
            channel: "test".to_string(),
            created_at: 0,
            depth: 1,
        },
    );
    sm.register_child(
        &root_id,
        ChildSessionInfo {
            session_id: child_session_id.to_string(),
            parent_session_id: root_id.to_string(),
            agent_id: "narrow-child".to_string(),
            depth: 1,
            mode: SpawnMode::Session,
        },
    )
    .await;
    save_checkpoint_with_budget(&mem_storage, child_session_id, 1, Some(2), Some(&root_id)).await;

    // grandchild: maxSpawnDepth=5 — effective = min(5, 2-1) = 1
    // Per design doc: child with effective=1 can be created and exists in tree.
    let mut grandchild_sub = SubagentsConfig::default();
    grandchild_sub.max_spawn_depth = 5;
    let grandchild = make_agent("grandchild", grandchild_sub);
    inject_agents(&cm, vec![("grandchild", grandchild)]);

    let result2 = controller
        .validate(child_session_id, Some("grandchild"))
        .await
        .expect("should pass: parent budget > 0, child created with effective=1");
    assert_eq!(result2.effective_max_spawn_depth, 1);
}

/// Full multi-level spawn tree from design doc:
/// root(3) → child1(5,eff=2) → child2(5,eff=1) → child3(1,eff=0)
#[tokio::test]
async fn test_depth_budget_full_multilevel_tree() {
    let cm = Arc::new(make_config_manager());
    let (sm, mem_storage) = make_session_manager_with_memory_storage();
    let controller = SpawnController::new(
        cm.clone(),
        sm.clone(),
        Arc::new(PermissionEngine::new_with_default_data_root(
            RuleSetBuilder::new().build().unwrap(),
        )),
    );

    // root: maxSpawnDepth=3
    let mut root_sub = SubagentsConfig::default();
    root_sub.max_spawn_depth = 3;
    let root = make_agent("root", root_sub);
    let root_id = setup_parent_session(&sm, "root").await;
    save_checkpoint_with_budget(&mem_storage, &root_id, 0, Some(3), None).await;

    // child1: maxSpawnDepth=5 — effective = min(5, 3-1) = 2
    let mut child1_sub = SubagentsConfig::default();
    child1_sub.max_spawn_depth = 5;
    let child1 = make_agent("child1", child1_sub);
    inject_agents(&cm, vec![("root", root), ("child1", child1)]);

    let result1 = controller
        .validate(&root_id, Some("child1"))
        .await
        .expect("root → child1: should pass, effective=2");
    assert_eq!(result1.effective_max_spawn_depth, 2);

    // Simulate child1 created with effective budget = 2
    let child1_sid = "tree-child1";
    sm.sessions.write().await.insert(
        child1_sid.to_string(),
        crate::gateway::Session {
            id: child1_sid.to_string(),
            agent_id: "child1".to_string(),
            channel: "test".to_string(),
            created_at: 0,
            depth: 1,
        },
    );
    sm.register_child(
        &root_id,
        ChildSessionInfo {
            session_id: child1_sid.to_string(),
            parent_session_id: root_id.to_string(),
            agent_id: "child1".to_string(),
            depth: 1,
            mode: SpawnMode::Session,
        },
    )
    .await;
    save_checkpoint_with_budget(&mem_storage, child1_sid, 1, Some(2), Some(&root_id)).await;

    // child2: maxSpawnDepth=5 — effective = min(5, 2-1) = 1
    let mut child2_sub = SubagentsConfig::default();
    child2_sub.max_spawn_depth = 5;
    let child2 = make_agent("child2", child2_sub);
    inject_agents(&cm, vec![("child2", child2)]);

    let result2 = controller
        .validate(child1_sid, Some("child2"))
        .await
        .expect("child1 → child2: should pass, effective=1");
    assert_eq!(result2.effective_max_spawn_depth, 1);

    // Simulate child2 created with effective budget = 1
    let child2_sid = "tree-child2";
    sm.sessions.write().await.insert(
        child2_sid.to_string(),
        crate::gateway::Session {
            id: child2_sid.to_string(),
            agent_id: "child2".to_string(),
            channel: "test".to_string(),
            created_at: 0,
            depth: 2,
        },
    );
    sm.register_child(
        child1_sid,
        ChildSessionInfo {
            session_id: child2_sid.to_string(),
            parent_session_id: child1_sid.to_string(),
            agent_id: "child2".to_string(),
            depth: 2,
            mode: SpawnMode::Session,
        },
    )
    .await;
    save_checkpoint_with_budget(&mem_storage, child2_sid, 2, Some(1), Some(child1_sid)).await;

    // child3: maxSpawnDepth=1 — effective = min(1, 1-1) = 0
    // Per design doc: child3 exists in tree but cannot spawn further.
    let mut child3_sub = SubagentsConfig::default();
    child3_sub.max_spawn_depth = 1;
    let child3 = make_agent("child3", child3_sub);
    inject_agents(&cm, vec![("child3", child3)]);

    let result3 = controller
        .validate(child2_sid, Some("child3"))
        .await
        .expect("child2 → child3: should pass, effective=0 (leaf node)");
    assert_eq!(result3.effective_max_spawn_depth, 0);

    // Simulate child3 created with effective budget = 0
    let child3_sid = "tree-child3";
    sm.sessions.write().await.insert(
        child3_sid.to_string(),
        crate::gateway::Session {
            id: child3_sid.to_string(),
            agent_id: "child3".to_string(),
            channel: "test".to_string(),
            created_at: 0,
            depth: 3,
        },
    );
    sm.register_child(
        child2_sid,
        ChildSessionInfo {
            session_id: child3_sid.to_string(),
            parent_session_id: child2_sid.to_string(),
            agent_id: "child3".to_string(),
            depth: 3,
            mode: SpawnMode::Run,
        },
    )
    .await;
    save_checkpoint_with_budget(&mem_storage, child3_sid, 3, Some(0), Some(child2_sid)).await;

    // child3 has effective budget = 0 → cannot spawn further
    let mut child4_sub = SubagentsConfig::default();
    child4_sub.max_spawn_depth = 5;
    let child4 = make_agent("child4", child4_sub);
    inject_agents(&cm, vec![("child4", child4)]);

    let err = controller
        .validate(child3_sid, Some("child4"))
        .await
        .expect_err("child3 → child4: should reject, parent effective budget = 0");
    match err {
        SpawnError::DepthExceeded { current, max } => {
            assert_eq!(current, 1);
            assert_eq!(max, 0);
        }
        other => panic!("expected DepthExceeded, got {:?}", other),
    }
}

// ══════════════════════════════════════════════════════════════════════
// Step 1.5: Kill all-mode tests
// ══════════════════════════════════════════════════════════════════════

/// Kill a run-mode child session — should succeed and clean up all tables.
#[tokio::test]
async fn test_kill_run_mode_child_succeeds() {
    let tmp = tempfile::TempDir::new().unwrap();
    let storage = Arc::new(MemoryStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage),
        Some(tmp.path().to_path_buf()),
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );

    let parent_id = "parent-run-kill";
    let parent_cs = crate::llm::session::ConversationSession::new(
        parent_id.to_string(),
        "test-model".to_string(),
        tmp.path().to_path_buf(),
    );
    let parent_arc = std::sync::Arc::new(tokio::sync::RwLock::new(parent_cs));
    mgr.conversation_sessions
        .write()
        .await
        .insert(parent_id.to_string(), parent_arc);
    mgr.sessions.write().await.insert(
        parent_id.to_string(),
        crate::gateway::Session {
            id: parent_id.to_string(),
            agent_id: "parent-agent".to_string(),
            channel: "test".to_string(),
            created_at: 0,
            depth: 0,
        },
    );

    let child_id = "run-child-to-kill";
    let child_cs = crate::llm::session::ConversationSession::new(
        child_id.to_string(),
        "test-model".to_string(),
        tmp.path().to_path_buf(),
    );
    let child_arc = std::sync::Arc::new(tokio::sync::RwLock::new(child_cs));
    mgr.conversation_sessions
        .write()
        .await
        .insert(child_id.to_string(), child_arc);
    mgr.sessions.write().await.insert(
        child_id.to_string(),
        crate::gateway::Session {
            id: child_id.to_string(),
            agent_id: "child-agent".to_string(),
            channel: "spawn".to_string(),
            created_at: 0,
            depth: 1,
        },
    );
    mgr.register_child(
        parent_id,
        ChildSessionInfo {
            session_id: child_id.to_string(),
            parent_session_id: parent_id.to_string(),
            agent_id: "child-agent".to_string(),
            depth: 1,
            mode: SpawnMode::Run,
        },
    )
    .await;

    mgr.kill_child(parent_id, child_id)
        .await
        .expect("kill_child should succeed for run-mode child");

    assert!(!mgr.has_session(child_id).await);
    assert!(mgr.get_conversation_session(child_id).await.is_none());
    assert_eq!(mgr.count_active_children(parent_id).await, 0);
}

/// Kill a session-mode child session — should succeed (regression test).
#[tokio::test]
async fn test_kill_session_mode_child_succeeds() {
    let tmp = tempfile::TempDir::new().unwrap();
    let storage = Arc::new(MemoryStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage),
        Some(tmp.path().to_path_buf()),
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );

    let parent_id = "parent-session-kill";
    let parent_cs = crate::llm::session::ConversationSession::new(
        parent_id.to_string(),
        "test-model".to_string(),
        tmp.path().to_path_buf(),
    );
    let parent_arc = std::sync::Arc::new(tokio::sync::RwLock::new(parent_cs));
    mgr.conversation_sessions
        .write()
        .await
        .insert(parent_id.to_string(), parent_arc);
    mgr.sessions.write().await.insert(
        parent_id.to_string(),
        crate::gateway::Session {
            id: parent_id.to_string(),
            agent_id: "parent-agent".to_string(),
            channel: "test".to_string(),
            created_at: 0,
            depth: 0,
        },
    );

    let child_id = "session-child-to-kill";
    let child_cs = crate::llm::session::ConversationSession::new(
        child_id.to_string(),
        "test-model".to_string(),
        tmp.path().to_path_buf(),
    );
    let child_arc = std::sync::Arc::new(tokio::sync::RwLock::new(child_cs));
    mgr.conversation_sessions
        .write()
        .await
        .insert(child_id.to_string(), child_arc);
    mgr.sessions.write().await.insert(
        child_id.to_string(),
        crate::gateway::Session {
            id: child_id.to_string(),
            agent_id: "child-agent".to_string(),
            channel: "spawn".to_string(),
            created_at: 0,
            depth: 1,
        },
    );
    mgr.register_child(
        parent_id,
        ChildSessionInfo {
            session_id: child_id.to_string(),
            parent_session_id: parent_id.to_string(),
            agent_id: "child-agent".to_string(),
            depth: 1,
            mode: SpawnMode::Session,
        },
    )
    .await;

    mgr.kill_child(parent_id, child_id)
        .await
        .expect("kill_child should succeed for session-mode child");

    assert!(!mgr.has_session(child_id).await);
    assert!(mgr.get_conversation_session(child_id).await.is_none());
    assert_eq!(mgr.count_active_children(parent_id).await, 0);
}

// ══════════════════════════════════════════════════════════════════════
// Step 1.5: Cascade termination on parent finish_llm
// ══════════════════════════════════════════════════════════════════════

/// Simulate the cascade termination pattern from finish_llm:
/// list_active_child_ids → kill_child for each → all removed.
#[tokio::test]
async fn test_cascade_terminate_all_children_simulation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let storage = Arc::new(MemoryStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage),
        Some(tmp.path().to_path_buf()),
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );

    let parent_id = "parent-cascade";
    let parent_cs = crate::llm::session::ConversationSession::new(
        parent_id.to_string(),
        "test-model".to_string(),
        tmp.path().to_path_buf(),
    );
    let parent_arc = std::sync::Arc::new(tokio::sync::RwLock::new(parent_cs));
    mgr.conversation_sessions
        .write()
        .await
        .insert(parent_id.to_string(), parent_arc);
    mgr.sessions.write().await.insert(
        parent_id.to_string(),
        crate::gateway::Session {
            id: parent_id.to_string(),
            agent_id: "parent-agent".to_string(),
            channel: "test".to_string(),
            created_at: 0,
            depth: 0,
        },
    );

    // Create 3 children (2 run, 1 session)
    let children: Vec<(&str, SpawnMode)> = vec![
        ("child-a", SpawnMode::Run),
        ("child-b", SpawnMode::Session),
        ("child-c", SpawnMode::Run),
    ];
    for (child_id, mode) in &children {
        let cs = crate::llm::session::ConversationSession::new(
            child_id.to_string(),
            "test-model".to_string(),
            tmp.path().to_path_buf(),
        );
        let arc = std::sync::Arc::new(tokio::sync::RwLock::new(cs));
        mgr.conversation_sessions
            .write()
            .await
            .insert(child_id.to_string(), arc);
        mgr.sessions.write().await.insert(
            child_id.to_string(),
            crate::gateway::Session {
                id: child_id.to_string(),
                agent_id: "child-agent".to_string(),
                channel: "spawn".to_string(),
                created_at: 0,
                depth: 1,
            },
        );
        mgr.register_child(
            parent_id,
            ChildSessionInfo {
                session_id: child_id.to_string(),
                parent_session_id: parent_id.to_string(),
                agent_id: "child-agent".to_string(),
                depth: 1,
                mode: mode.clone(),
            },
        )
        .await;
    }

    assert_eq!(mgr.count_active_children(parent_id).await, 3);

    // Simulate finish_llm cascade pattern
    let child_ids = mgr.list_active_child_ids(parent_id).await;
    assert_eq!(child_ids.len(), 3);
    for child_id in &child_ids {
        mgr.kill_child(parent_id, child_id)
            .await
            .expect("kill_child should succeed in cascade");
    }

    assert_eq!(mgr.count_active_children(parent_id).await, 0);
    for (child_id, _) in &children {
        assert!(!mgr.has_session(child_id).await);
        assert!(mgr.get_conversation_session(child_id).await.is_none());
    }
}

/// When a parent session has no children, finish_llm cascade is a no-op.
#[tokio::test]
async fn test_cascade_terminate_no_children_noop() {
    let tmp = tempfile::TempDir::new().unwrap();
    let storage = Arc::new(MemoryStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage),
        Some(tmp.path().to_path_buf()),
        BootstrapMode::Full,
        ReasoningLevel::default(),
    );

    let parent_id = "parent-no-children";
    let parent_cs = crate::llm::session::ConversationSession::new(
        parent_id.to_string(),
        "test-model".to_string(),
        tmp.path().to_path_buf(),
    );
    let parent_arc = std::sync::Arc::new(tokio::sync::RwLock::new(parent_cs));
    mgr.conversation_sessions
        .write()
        .await
        .insert(parent_id.to_string(), parent_arc);

    let child_ids = mgr.list_active_child_ids(parent_id).await;
    assert_eq!(child_ids.len(), 0);
    for child_id in &child_ids {
        mgr.kill_child(parent_id, child_id)
            .await
            .expect("should not reach here");
    }
    assert!(mgr.get_conversation_session(parent_id).await.is_some());
}

// ══════════════════════════════════════════════════════════════════════
// Step 1.5: effective_max_spawn_depth persistence roundtrip
// ══════════════════════════════════════════════════════════════════════

/// Verify effective_max_spawn_depth round-trips through checkpoint
/// serialization (JSON serde).
#[test]
fn test_effective_max_spawn_depth_roundtrip() {
    // With value
    let cp = SessionCheckpoint::new("rt1".to_string()).with_effective_max_spawn_depth(Some(3));
    let json = serde_json::to_string(&cp).unwrap();
    let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.effective_max_spawn_depth, Some(3));

    // Without value
    let cp2 = SessionCheckpoint::new("rt2".to_string());
    let json2 = serde_json::to_string(&cp2).unwrap();
    let parsed2: SessionCheckpoint = serde_json::from_str(&json2).unwrap();
    assert_eq!(parsed2.effective_max_spawn_depth, None);

    // Missing field in old JSON → defaults to None
    let mut json_val = serde_json::to_value(&cp).unwrap();
    json_val
        .as_object_mut()
        .unwrap()
        .remove("effective_max_spawn_depth");
    let old_json_str = serde_json::to_string(&json_val).unwrap();
    let parsed_old: SessionCheckpoint = serde_json::from_str(&old_json_str).unwrap();
    assert_eq!(parsed_old.effective_max_spawn_depth, None);
}

/// Verify get_effective_max_spawn_depth reads from checkpoint correctly.
#[tokio::test]
async fn test_get_effective_max_spawn_depth_from_checkpoint() {
    let (sm, mem_storage) = make_session_manager_with_memory_storage();

    // No checkpoint → returns None
    assert_eq!(sm.get_effective_max_spawn_depth("nonexistent").await, None);

    // Save checkpoint with budget
    save_checkpoint_with_budget(&mem_storage, "s1", 0, Some(5), None).await;
    assert_eq!(sm.get_effective_max_spawn_depth("s1").await, Some(5));

    // Save checkpoint without budget
    save_checkpoint_with_budget(&mem_storage, "s2", 0, None, None).await;
    assert_eq!(sm.get_effective_max_spawn_depth("s2").await, None);
}

/// Verify validate_child_ownership works for both modes without
/// the old SpawnMode::Session filter.
#[tokio::test]
async fn test_validate_child_ownership_all_modes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (sm, _) = make_session_manager_with_memory_storage();

    let parent_id = "parent-ownership";
    let parent_cs = crate::llm::session::ConversationSession::new(
        parent_id.to_string(),
        "test-model".to_string(),
        tmp.path().to_path_buf(),
    );
    let parent_arc = std::sync::Arc::new(tokio::sync::RwLock::new(parent_cs));
    sm.conversation_sessions
        .write()
        .await
        .insert(parent_id.to_string(), parent_arc);

    // Register run-mode child
    let run_child = "run-ownership-child";
    sm.register_child(
        parent_id,
        ChildSessionInfo {
            session_id: run_child.to_string(),
            parent_session_id: parent_id.to_string(),
            agent_id: "child-agent".to_string(),
            depth: 1,
            mode: SpawnMode::Run,
        },
    )
    .await;

    // Register session-mode child
    let session_child = "session-ownership-child";
    sm.register_child(
        parent_id,
        ChildSessionInfo {
            session_id: session_child.to_string(),
            parent_session_id: parent_id.to_string(),
            agent_id: "child-agent".to_string(),
            depth: 1,
            mode: SpawnMode::Session,
        },
    )
    .await;

    // Both should be found by validate_child_ownership
    let run_info = sm.validate_child_ownership(parent_id, run_child).await;
    assert!(run_info.is_some());
    assert_eq!(run_info.unwrap().mode, SpawnMode::Run);

    let session_info = sm.validate_child_ownership(parent_id, session_child).await;
    assert!(session_info.is_some());
    assert_eq!(session_info.unwrap().mode, SpawnMode::Session);

    // Unknown child should return None
    let unknown = sm
        .validate_child_ownership(parent_id, "unknown-child")
        .await;
    assert!(unknown.is_none());
}

/// Verify kill_child cascades token to grandchild sessions
/// (parent → child → grandchild, kill child cascade-stops grandchild token).
#[tokio::test]
async fn test_kill_child_cascades_to_grandchild() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (mgr, _) = make_session_manager_with_memory_storage();

    let parent_id = "parent-kill-cascade";
    let parent_cs = crate::llm::session::ConversationSession::new(
        parent_id.to_string(),
        "test-model".to_string(),
        tmp.path().to_path_buf(),
    );
    let parent_arc = std::sync::Arc::new(tokio::sync::RwLock::new(parent_cs));
    mgr.conversation_sessions
        .write()
        .await
        .insert(parent_id.to_string(), parent_arc);
    mgr.sessions.write().await.insert(
        parent_id.to_string(),
        crate::gateway::Session {
            id: parent_id.to_string(),
            agent_id: "parent-agent".to_string(),
            channel: "test".to_string(),
            created_at: 0,
            depth: 0,
        },
    );

    // Create child with a grandchild registered under it
    let child_id = "child-kill-cascade";
    let child_cs = crate::llm::session::ConversationSession::new(
        child_id.to_string(),
        "test-model".to_string(),
        tmp.path().to_path_buf(),
    );
    let child_arc = std::sync::Arc::new(tokio::sync::RwLock::new(child_cs));
    mgr.conversation_sessions
        .write()
        .await
        .insert(child_id.to_string(), child_arc.clone());
    mgr.sessions.write().await.insert(
        child_id.to_string(),
        crate::gateway::Session {
            id: child_id.to_string(),
            agent_id: "child-agent".to_string(),
            channel: "spawn".to_string(),
            created_at: 0,
            depth: 1,
        },
    );
    mgr.register_child(
        parent_id,
        ChildSessionInfo {
            session_id: child_id.to_string(),
            parent_session_id: parent_id.to_string(),
            agent_id: "child-agent".to_string(),
            depth: 1,
            mode: SpawnMode::Session,
        },
    )
    .await;

    // Register grandchild under child
    let grandchild_id = "grandchild-kill-cascade";
    let gc_cs = crate::llm::session::ConversationSession::new(
        grandchild_id.to_string(),
        "test-model".to_string(),
        tmp.path().to_path_buf(),
    );
    let gc_arc = std::sync::Arc::new(tokio::sync::RwLock::new(gc_cs));
    mgr.conversation_sessions
        .write()
        .await
        .insert(grandchild_id.to_string(), gc_arc.clone());
    mgr.sessions.write().await.insert(
        grandchild_id.to_string(),
        crate::gateway::Session {
            id: grandchild_id.to_string(),
            agent_id: "grandchild-agent".to_string(),
            channel: "spawn".to_string(),
            created_at: 0,
            depth: 2,
        },
    );
    mgr.register_child(
        child_id,
        ChildSessionInfo {
            session_id: grandchild_id.to_string(),
            parent_session_id: child_id.to_string(),
            agent_id: "grandchild-agent".to_string(),
            depth: 2,
            mode: SpawnMode::Run,
        },
    )
    .await;

    // Register grandchild handle in child's ConversationSession
    child_arc
        .read()
        .await
        .register_child_handle(grandchild_id, std::sync::Arc::downgrade(&gc_arc));

    // Verify initial state
    assert_eq!(mgr.count_active_children(parent_id).await, 1);
    assert_eq!(mgr.count_active_children(child_id).await, 1);
    assert!(mgr.has_session(grandchild_id).await);

    // Kill the child — grandchild token is cascade-stopped
    mgr.kill_child(parent_id, child_id)
        .await
        .expect("kill_child should succeed");

    // Child should be removed
    assert!(!mgr.has_session(child_id).await);
    assert!(mgr.get_conversation_session(child_id).await.is_none());
    assert_eq!(mgr.count_active_children(parent_id).await, 0);
    // Grandchild is recursively cleaned up — all tracking tables are
    // purged (per design doc §级联 Kill: from deepest to shallowest).
    assert!(
        !mgr.has_session(grandchild_id).await,
        "grandchild session should be recursively removed after parent kill"
    );
    assert!(
        mgr.get_conversation_session(grandchild_id).await.is_none(),
        "grandchild conversation_session should be removed after parent kill"
    );
    assert_eq!(
        mgr.count_active_children(child_id).await,
        0,
        "grandchild entry should be removed from children table"
    );
}
