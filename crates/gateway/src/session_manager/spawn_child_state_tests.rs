//! Tests for Step 1.3: child_states registration and checkpoint persistence.
//!
//! Verifies that:
//! - `register_child_state` is called during `create_child_session`
//! - The parent checkpoint contains SubSessionSpawn in `pending_operations`
//! - `deregister_child_state` removes the entry on child completion

use super::spawn::SpawnMode;
use super::test_helpers::{
    append_assistant_to_child, setup_parent_with_conv, test_resolved_config,
};
use super::tests::{clear_global_prompt_state, make_test_mgr, test_config};
use super::SessionManager;
use closeclaw_common::tool_session::ToolSession;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::{PersistenceService, ReasoningLevel, SessionCheckpoint};
use closeclaw_tasks::NotificationPriority;
use serial_test::serial;
use std::sync::Arc;

/// After spawning a child session, the parent's checkpoint should
/// contain a SubSessionSpawn entry in `pending_operations` with the
/// correct `agent_id` and `task_summary`.
#[tokio::test]
#[serial]
async fn test_spawn_registers_child_state_in_checkpoint() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());

    // Register a parent checkpoint so it can be loaded.
    let parent_id = "parent-child-state";
    let mut parent_cp = SessionCheckpoint::new(parent_id.to_string());
    parent_cp.depth = 0;
    storage.save_checkpoint(&parent_cp).await.unwrap();

    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        Some(tmp.path().to_path_buf()),
        ReasoningLevel::default(),
    );

    let config = test_resolved_config("worker-child-state", None);
    setup_parent_with_conv(&mgr, parent_id).await;

    let child_id = mgr
        .create_child_session(
            &config,
            parent_id,
            1,
            "child state task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None,
            None,
            None, // prompt_template_prefix
        )
        .await
        .expect("create_child_session should succeed");

    // Read parent's ConversationSession and verify child_states.
    let parent_cs = mgr
        .get_conversation_session(parent_id)
        .await
        .expect("parent session should exist");
    {
        let guard = parent_cs.read().await;
        let states = guard
            .child_states
            .read()
            .expect("child_states lock poisoned");
        let entry = states
            .get(&child_id)
            .expect("child should be registered in child_states");
        assert!(
            matches!(entry.0, closeclaw_common::ChildSessionState::Running),
            "child state should be Running"
        );
        let detail = entry.1.as_ref().expect("detail should be present");
        match detail {
            closeclaw_session::pending_operation_detail::PendingOperationDetail::SubSessionSpawn {
                agent_id,
                task_summary,
                ..
            } => {
                assert_eq!(agent_id, "worker-child-state");
                assert_eq!(task_summary, "child state task");
            }
            other => panic!("expected SubSessionSpawn detail, got {:?}", other),
        }
    }

    // Verify the pending_operations include the SubSessionSpawn.
    {
        let guard = parent_cs.read().await;
        let ops = guard.collect_pending_operations();
        let spawn_ops: Vec<_> = ops
            .iter()
            .filter(|op| {
                matches!(
                    op.op_type,
                    closeclaw_session::persistence::PendingOperationType::SubSessionSpawn
                )
            })
            .collect();
        assert_eq!(
            spawn_ops.len(),
            1,
            "expected exactly 1 SubSessionSpawn pending operation"
        );
        assert_eq!(spawn_ops[0].op_id, child_id);
    }
}

/// After `try_push_announce` completes for a run-mode child, the
/// parent's `child_states` should no longer contain the child entry.
#[tokio::test]
#[serial]
async fn test_child_completion_deregisters_child_state() {
    clear_global_prompt_state();

    let _tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(_tmp.path()));
    let parent_id = setup_parent_with_conv(&mgr, "parent-dereg").await;

    let child_id = mgr
        .create_child_session(
            &test_resolved_config("worker-dereg", None),
            &parent_id,
            1,
            "deregister task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None,
            None,
            None, // prompt_template_prefix
        )
        .await
        .expect("create_child_session should succeed");

    // Verify child is registered.
    {
        let parent_cs = mgr
            .get_conversation_session(&parent_id)
            .await
            .expect("parent session should exist");
        let guard = parent_cs.read().await;
        let states = guard
            .child_states
            .read()
            .expect("child_states lock poisoned");
        assert!(
            states.contains_key(&child_id),
            "child should be in child_states before completion"
        );
    }

    // Simulate child completing with an assistant message.
    append_assistant_to_child(
        &mgr,
        &child_id,
        vec![closeclaw_llm::types::ContentBlock::Text("done".to_string())],
    )
    .await;

    // Trigger announce push (which deregisters child_state).
    mgr.try_push_announce(&child_id, NotificationPriority::Next)
        .await;

    // Verify child is deregistered.
    {
        let parent_cs = mgr
            .get_conversation_session(&parent_id)
            .await
            .expect("parent session should exist");
        let guard = parent_cs.read().await;
        let states = guard
            .child_states
            .read()
            .expect("child_states lock poisoned");
        assert!(
            !states.contains_key(&child_id),
            "child should be removed from child_states after completion"
        );
    }

    // Verify pending_operations no longer contains SubSessionSpawn.
    {
        let parent_cs = mgr
            .get_conversation_session(&parent_id)
            .await
            .expect("parent session should exist");
        let guard = parent_cs.read().await;
        let ops = guard.collect_pending_operations();
        let spawn_ops: Vec<_> = ops
            .iter()
            .filter(|op| {
                matches!(
                    op.op_type,
                    closeclaw_session::persistence::PendingOperationType::SubSessionSpawn
                )
            })
            .collect();
        assert!(
            spawn_ops.is_empty(),
            "no SubSessionSpawn should remain after child completion"
        );
    }
}

/// Verify that `register_child_state` on `ToolSession` trait adds the
/// child to `child_states` and triggers checkpoint persistence.
#[tokio::test]
#[serial]
async fn test_register_child_state_trait_method() {
    clear_global_prompt_state();

    let _tmp = tempfile::TempDir::new().unwrap();
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());
    let parent_id = "parent-trait-test";

    let mut parent_cp = SessionCheckpoint::new(parent_id.to_string());
    parent_cp.depth = 0;
    storage.save_checkpoint(&parent_cp).await.unwrap();

    // Create a ConversationSession directly with checkpoint_storage set.
    let cs_arc = Arc::new(tokio::sync::RwLock::new(ConversationSession::new(
        parent_id.to_string(),
        "test-model".to_string(),
        std::path::PathBuf::from("/tmp"),
    )));
    {
        let mut guard = cs_arc.write().await;
        guard.set_checkpoint_storage(storage.clone() as Arc<dyn PersistenceService>);
    }

    // Register a child.
    {
        let guard = cs_arc.read().await;
        guard
            .register_child_state(
                "child-1".to_string(),
                "agent-1".to_string(),
                "test task".to_string(),
            )
            .await;
    }

    // Verify child_states.
    {
        let guard = cs_arc.read().await;
        let states = guard
            .child_states
            .read()
            .expect("child_states lock poisoned");
        let entry = states
            .get("child-1")
            .expect("child should be in child_states");
        assert!(matches!(
            entry.0,
            closeclaw_common::ChildSessionState::Running
        ));
    }

    // Verify pending_operations includes the child.
    {
        let guard = cs_arc.read().await;
        let ops = guard.collect_pending_operations();
        let spawn_ops: Vec<_> = ops
            .iter()
            .filter(|op| {
                matches!(
                    op.op_type,
                    closeclaw_session::persistence::PendingOperationType::SubSessionSpawn
                )
            })
            .collect();
        assert_eq!(spawn_ops.len(), 1);
        assert_eq!(spawn_ops[0].op_id, "child-1");
    }

    // Deregister the child.
    {
        let guard = cs_arc.read().await;
        guard.deregister_child_state("child-1".to_string()).await;
    }

    // Verify child is removed.
    {
        let guard = cs_arc.read().await;
        let states = guard
            .child_states
            .read()
            .expect("child_states lock poisoned");
        assert!(
            !states.contains_key("child-1"),
            "child should be removed after deregister"
        );
    }

    // Verify pending_operations no longer includes the child.
    {
        let guard = cs_arc.read().await;
        let ops = guard.collect_pending_operations();
        let spawn_ops: Vec<_> = ops
            .iter()
            .filter(|op| {
                matches!(
                    op.op_type,
                    closeclaw_session::persistence::PendingOperationType::SubSessionSpawn
                )
            })
            .collect();
        assert!(spawn_ops.is_empty());
    }
}

/// Verify that `deregister_child_state` is idempotent — calling it
/// for a child that was never registered logs a warning but does not
/// panic.
#[tokio::test]
#[serial]
async fn test_deregister_child_state_idempotent() {
    clear_global_prompt_state();

    let cs = ConversationSession::new(
        "idempotent-test".to_string(),
        "test-model".to_string(),
        std::path::PathBuf::from("/tmp"),
    );

    // Should not panic even though child was never registered.
    cs.deregister_child_state("nonexistent-child".to_string())
        .await;
}
