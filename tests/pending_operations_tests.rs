//! Unit tests for pending_operations recovery mechanism (Step 1.4).
//!
//! Covers:
//! - collect_pending_operations collects three op_types
//! - Checkpoint serialization/deserialization of pending_operations
//! - Recovery injection with non-empty pending_operations
//! - Recovery flow unaffected when pending_operations is empty

use closeclaw_llm::session_state::{ChildSessionState, ToolExecState};
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::{
    PendingOperation, PendingOperationStatus, PendingOperationType, PersistenceService,
    SessionCheckpoint,
};

// ── Step 1.4: collect_pending_operations ────────────────────────────────

#[test]
fn test_collect_pending_operations_empty() {
    let cs = ConversationSession::new(
        "sess_empty".into(),
        "test-model".into(),
        std::path::PathBuf::from("/tmp"),
    );
    let ops = cs.collect_pending_operations();
    assert!(ops.is_empty(), "fresh session should have no pending ops");
}

#[test]
fn test_collect_pending_operations_tool_calls() {
    let cs = ConversationSession::new(
        "sess_tools".into(),
        "test-model".into(),
        std::path::PathBuf::from("/tmp"),
    );

    // Simulate running tool calls via pub(crate) field
    {
        let mut tool_states = cs.tool_states.write().unwrap();
        tool_states.insert("call_1".to_string(), ToolExecState::RunningForeground);
        tool_states.insert("call_2".to_string(), ToolExecState::RunningBackground);
        tool_states.insert("call_3".to_string(), ToolExecState::Pending);
    }

    let ops = cs.collect_pending_operations();
    let tool_ops: Vec<_> = ops
        .iter()
        .filter(|op| op.op_type == PendingOperationType::ToolCall)
        .collect();
    assert_eq!(tool_ops.len(), 3, "should collect 3 tool call ops");

    let ids: Vec<&str> = tool_ops.iter().map(|op| op.op_id.as_str()).collect();
    assert!(ids.contains(&"call_1"));
    assert!(ids.contains(&"call_2"));
    assert!(ids.contains(&"call_3"));
}

#[test]
fn test_collect_pending_operations_skips_completed_tools() {
    let cs = ConversationSession::new(
        "sess_completed".into(),
        "test-model".into(),
        std::path::PathBuf::from("/tmp"),
    );

    {
        let mut tool_states = cs.tool_states.write().unwrap();
        tool_states.insert("done".to_string(), ToolExecState::Completed);
        tool_states.insert("running".to_string(), ToolExecState::RunningForeground);
    }

    let ops = cs.collect_pending_operations();
    // Only the RunningForeground tool should be collected
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_id, "running");
    assert_eq!(ops[0].op_type, PendingOperationType::ToolCall);
}

#[test]
fn test_collect_pending_operations_child_sessions() {
    let cs = ConversationSession::new(
        "sess_children".into(),
        "test-model".into(),
        std::path::PathBuf::from("/tmp"),
    );

    {
        let mut child_states = cs.child_states.write().unwrap();
        child_states.insert("child_1".to_string(), ChildSessionState::Running);
        child_states.insert("child_2".to_string(), ChildSessionState::Running);
    }

    let ops = cs.collect_pending_operations();
    let child_ops: Vec<_> = ops
        .iter()
        .filter(|op| op.op_type == PendingOperationType::SubSessionSpawn)
        .collect();
    assert_eq!(child_ops.len(), 2, "should collect 2 child session ops");
}

#[test]
fn test_collect_pending_operations_outbound_messages() {
    let mut cs = ConversationSession::new(
        "sess_outbound".into(),
        "test-model".into(),
        std::path::PathBuf::from("/tmp"),
    );

    // Use restore_pending_messages to add unsent messages
    use closeclaw_session::persistence::PendingMessage;
    let messages = vec![
        {
            let mut pm = PendingMessage::new("msg_1".into(), "content_1".into());
            pm.sent = false;
            pm
        },
        {
            let mut pm = PendingMessage::new("msg_2".into(), "content_2".into());
            pm.sent = true; // This one is sent — should be skipped
            pm
        },
        {
            let mut pm = PendingMessage::new("msg_3".into(), "content_3".into());
            pm.sent = false;
            pm
        },
    ];
    cs.restore_pending_messages(messages);

    let ops = cs.collect_pending_operations();
    let outbound_ops: Vec<_> = ops
        .iter()
        .filter(|op| op.op_type == PendingOperationType::OutboundMessage)
        .collect();
    // Only unsent messages should be collected
    assert_eq!(
        outbound_ops.len(),
        2,
        "should collect 2 unsent outbound message ops"
    );
    let ids: Vec<&str> = outbound_ops.iter().map(|op| op.op_id.as_str()).collect();
    assert!(ids.contains(&"msg_1"));
    assert!(ids.contains(&"msg_3"));
}

#[test]
fn test_collect_pending_operations_mixed_types() {
    let mut cs = ConversationSession::new(
        "sess_mixed".into(),
        "test-model".into(),
        std::path::PathBuf::from("/tmp"),
    );

    // Add one of each type
    {
        let mut tool_states = cs.tool_states.write().unwrap();
        tool_states.insert("tool_1".to_string(), ToolExecState::RunningForeground);
    }
    {
        let mut child_states = cs.child_states.write().unwrap();
        child_states.insert("child_1".to_string(), ChildSessionState::Running);
    }
    {
        use closeclaw_session::persistence::PendingMessage;
        let mut pm = PendingMessage::new("msg_1".into(), "content".into());
        pm.sent = false;
        cs.restore_pending_messages(vec![pm]);
    }

    let ops = cs.collect_pending_operations();
    assert_eq!(ops.len(), 3, "should collect one of each op_type");

    let op_types: Vec<&PendingOperationType> = ops.iter().map(|op| &op.op_type).collect();
    assert!(op_types.contains(&&PendingOperationType::ToolCall));
    assert!(op_types.contains(&&PendingOperationType::SubSessionSpawn));
    assert!(op_types.contains(&&PendingOperationType::OutboundMessage));
}

// ── Step 1.4: checkpoint serialization/deserialization ──────────────────

#[test]
fn test_checkpoint_pending_operations_roundtrip_empty() {
    let cp = SessionCheckpoint::new("sess_rt_empty".into());
    assert!(cp.pending_operations.is_empty());

    let json = serde_json::to_string(&cp).unwrap();
    let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
    assert!(parsed.pending_operations.is_empty());
}

#[test]
fn test_checkpoint_pending_operations_roundtrip_with_ops() {
    let now = chrono::Utc::now();
    let ops = vec![
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "tool_call_1".into(),
            op_type: PendingOperationType::ToolCall,
            name: "bash".into(),
            args: r#"{"command":"ls"}"#.into(),
            created_at: now,
        },
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "child_1".into(),
            op_type: PendingOperationType::SubSessionSpawn,
            name: "sub-agent-1".into(),
            args: String::new(),
            created_at: now,
        },
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "msg_1".into(),
            op_type: PendingOperationType::OutboundMessage,
            name: "outbound-chat".into(),
            args: "hello world".into(),
            created_at: now,
        },
    ];

    let cp = SessionCheckpoint::new("sess_rt_ops".into()).with_pending_operations(ops);
    let json = serde_json::to_string(&cp).unwrap();
    let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.pending_operations.len(), 3);
    assert_eq!(
        parsed.pending_operations[0].op_type,
        PendingOperationType::ToolCall
    );
    assert_eq!(parsed.pending_operations[0].name, "bash");
    assert_eq!(parsed.pending_operations[0].args, r#"{"command":"ls"}"#);
    assert_eq!(
        parsed.pending_operations[1].op_type,
        PendingOperationType::SubSessionSpawn
    );
    assert_eq!(
        parsed.pending_operations[2].op_type,
        PendingOperationType::OutboundMessage
    );
    assert_eq!(parsed.pending_operations[2].args, "hello world");
}

#[test]
fn test_checkpoint_pending_operations_missing_json_defaults_empty() {
    // Old checkpoint JSON without pending_operations field should default
    // to empty Vec
    let cp = SessionCheckpoint::new("sess_old".into());
    let mut json_value: serde_json::Value = serde_json::to_value(&cp).unwrap();
    json_value
        .as_object_mut()
        .unwrap()
        .remove("pending_operations");
    let json_str = serde_json::to_string(&json_value).unwrap();
    let parsed: SessionCheckpoint = serde_json::from_str(&json_str).unwrap();
    assert!(
        parsed.pending_operations.is_empty(),
        "old data without pending_operations should default to empty Vec"
    );
}

#[test]
fn test_pending_operation_type_serde_roundtrip() {
    for op_type in [
        PendingOperationType::ToolCall,
        PendingOperationType::SubSessionSpawn,
        PendingOperationType::OutboundMessage,
    ] {
        let json = serde_json::to_string(&op_type).unwrap();
        let parsed: PendingOperationType = serde_json::from_str(&json).unwrap();
        assert_eq!(op_type, parsed);
    }
}

#[test]
fn test_pending_operation_type_serde_values() {
    assert_eq!(
        serde_json::to_string(&PendingOperationType::ToolCall).unwrap(),
        "\"tool_call\""
    );
    assert_eq!(
        serde_json::to_string(&PendingOperationType::SubSessionSpawn).unwrap(),
        "\"sub_session_spawn\""
    );
    assert_eq!(
        serde_json::to_string(&PendingOperationType::OutboundMessage).unwrap(),
        "\"outbound_message\""
    );
}

#[test]
fn test_checkpoint_with_pending_operations_builder() {
    let ops = vec![PendingOperation {
        op_id: "op_1".into(),
        op_type: PendingOperationType::ToolCall,
        status: PendingOperationStatus::Running,
        name: "test_tool".into(),
        args: String::new(),
        created_at: chrono::Utc::now(),
    }];

    let cp = SessionCheckpoint::new("sess_builder".into()).with_pending_operations(ops);
    assert_eq!(cp.pending_operations.len(), 1);
    assert_eq!(cp.pending_operations[0].op_id, "op_1");
}

// ── Step 1.4: recovery injection ────────────────────────────────────────

use closeclaw_session::recovery::SessionRecoveryService;
use closeclaw_session::storage::memory::MemoryStorage;
use std::sync::Arc;

fn make_checkpoint_with_pending_ops(session_id: &str) -> SessionCheckpoint {
    let now = chrono::Utc::now();
    SessionCheckpoint::new(session_id.into()).with_pending_operations(vec![
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "tool_1".into(),
            op_type: PendingOperationType::ToolCall,
            name: "bash".into(),
            args: r#"{"cmd":"echo"}"#.into(),
            created_at: now,
        },
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "child_1".into(),
            op_type: PendingOperationType::SubSessionSpawn,
            name: "sub-agent".into(),
            args: String::new(),
            created_at: now,
        },
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "msg_1".into(),
            op_type: PendingOperationType::OutboundMessage,
            name: "chat-output".into(),
            args: "pending reply".into(),
            created_at: now,
        },
    ])
}

#[tokio::test]
async fn test_recovery_with_pending_operations_marks_dirty() {
    let storage = Arc::new(MemoryStorage::new());
    storage
        .save_checkpoint(&make_checkpoint_with_pending_ops("dirty_session"))
        .await
        .unwrap();

    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await.unwrap();

    assert_eq!(report.recovered.len(), 1);
    assert!(
        report.dirty_sessions.contains(&"dirty_session".to_string()),
        "session with pending_operations should be in dirty_sessions"
    );
}

#[tokio::test]
async fn test_recovery_without_pending_operations_not_dirty() {
    let storage = Arc::new(MemoryStorage::new());
    let cp = SessionCheckpoint::new("clean_session".into());
    assert!(cp.pending_operations.is_empty());
    storage.save_checkpoint(&cp).await.unwrap();

    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await.unwrap();

    assert_eq!(report.recovered.len(), 1);
    assert!(
        report.dirty_sessions.is_empty(),
        "session without pending_operations should NOT be dirty"
    );
}

#[tokio::test]
async fn test_recovery_mixed_dirty_and_clean_sessions() {
    let storage = Arc::new(MemoryStorage::new());
    storage
        .save_checkpoint(&make_checkpoint_with_pending_ops("dirty_1"))
        .await
        .unwrap();
    storage
        .save_checkpoint(&SessionCheckpoint::new("clean_1".into()))
        .await
        .unwrap();
    storage
        .save_checkpoint(&make_checkpoint_with_pending_ops("dirty_2"))
        .await
        .unwrap();

    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await.unwrap();

    assert_eq!(report.recovered.len(), 3);
    assert_eq!(report.dirty_sessions.len(), 2);
    assert!(report.dirty_sessions.contains(&"dirty_1".to_string()));
    assert!(report.dirty_sessions.contains(&"dirty_2".into()));
    assert!(!report.dirty_sessions.contains(&"clean_1".to_string()));
}

#[tokio::test]
async fn test_recovery_inject_calls_restore_callback_for_dirty_session() {
    let storage = Arc::new(MemoryStorage::new());
    storage
        .save_checkpoint(&make_checkpoint_with_pending_ops("dirty_cb"))
        .await
        .unwrap();

    let service = SessionRecoveryService::new(Arc::clone(&storage));

    let restored = Arc::new(std::sync::Mutex::new(Vec::new()));
    let restored_clone = Arc::clone(&restored);

    service
        .set_restore_callback(
            move |session_id, checkpoint, _notification, _tool_failures| {
                // The callback is called even for dirty sessions — it can
                // inspect pending_operations to inject recovery notifications
                restored_clone
                    .lock()
                    .unwrap()
                    .push((session_id.to_string(), checkpoint.pending_operations.len()));
                Ok(())
            },
        )
        .await;

    let report = service.recover().await.unwrap();

    assert_eq!(report.recovered.len(), 1);
    let restored_sessions = restored.lock().unwrap();
    assert_eq!(restored_sessions.len(), 1);
    assert_eq!(restored_sessions[0].0, "dirty_cb");
    assert_eq!(
        restored_sessions[0].1, 3,
        "callback should see 3 pending operations"
    );
}

#[tokio::test]
async fn test_recovery_empty_pending_operations_no_dirty() {
    let storage = Arc::new(MemoryStorage::new());
    let mut cp = SessionCheckpoint::new("empty_pending".into());
    cp.pending_operations = Vec::new();
    storage.save_checkpoint(&cp).await.unwrap();

    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await.unwrap();

    assert_eq!(report.recovered.len(), 1);
    assert!(report.dirty_sessions.is_empty());
}

#[tokio::test]
async fn test_recovery_pending_operations_preserved_in_checkpoint() {
    // Verify that the checkpoint's pending_operations survive the recovery
    // flow (save → load → check)
    let storage = Arc::new(MemoryStorage::new());
    let original = make_checkpoint_with_pending_ops("preserved");
    storage.save_checkpoint(&original).await.unwrap();

    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await.unwrap();
    assert_eq!(report.recovered.len(), 1);

    // Load checkpoint and verify pending_operations are intact
    let loaded = storage.load_checkpoint("preserved").await.unwrap().unwrap();
    assert_eq!(loaded.pending_operations.len(), 3);
    assert_eq!(
        loaded.pending_operations[0].op_type,
        PendingOperationType::ToolCall
    );
    assert_eq!(
        loaded.pending_operations[1].op_type,
        PendingOperationType::SubSessionSpawn
    );
    assert_eq!(
        loaded.pending_operations[2].op_type,
        PendingOperationType::OutboundMessage
    );
}

#[tokio::test]
async fn test_recovery_report_dirty_sessions_count() {
    let storage = Arc::new(MemoryStorage::new());
    // 5 sessions, 3 with pending ops
    for i in 0..5u32 {
        if i < 3 {
            storage
                .save_checkpoint(&make_checkpoint_with_pending_ops(&format!("s_{}", i)))
                .await
                .unwrap();
        } else {
            storage
                .save_checkpoint(&SessionCheckpoint::new(format!("s_{}", i)))
                .await
                .unwrap();
        }
    }

    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await.unwrap();

    assert_eq!(report.recovered.len(), 5);
    assert_eq!(report.dirty_sessions.len(), 3);
    assert!(report.is_full_success());
}

#[tokio::test]
async fn test_recovery_callback_receives_notification_and_tool_failures() {
    use closeclaw_session::persistence::{
        PendingOperation, PendingOperationStatus, PendingOperationType, PersistenceService,
        SessionCheckpoint,
    };
    use closeclaw_session::storage::memory::MemoryStorage;
    use std::sync::Arc;

    let storage = Arc::new(MemoryStorage::new());
    let now = chrono::Utc::now();
    let cp = SessionCheckpoint::new("notif_cb".into()).with_pending_operations(vec![
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "tool_1".into(),
            op_type: PendingOperationType::ToolCall,
            name: "bash".into(),
            args: r#"{"cmd":"ls"}"#.into(),
            created_at: now,
        },
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "child_1".into(),
            op_type: PendingOperationType::SubSessionSpawn,
            name: "sub-agent".into(),
            args: String::new(),
            created_at: now,
        },
    ]);
    storage.save_checkpoint(&cp).await.unwrap();

    let service = SessionRecoveryService::new(Arc::clone(&storage));

    let captured = Arc::new(std::sync::Mutex::new((
        None::<String>,
        Vec::<String>::new(),
    )));
    let captured_clone = Arc::clone(&captured);

    service
        .set_restore_callback(
            move |_session_id, _checkpoint, notification, tool_failures| {
                *captured_clone.lock().unwrap() =
                    (notification.map(String::from), tool_failures.to_vec());
                Ok(())
            },
        )
        .await;

    let report = service.recover().await.unwrap();
    assert_eq!(report.recovered.len(), 1);

    let (notif, failures) = captured.lock().unwrap().clone();
    // Notification should contain the pending tool call
    assert!(notif.is_some());
    let notif = notif.unwrap();
    assert!(notif.contains("网关已重启"));
    assert!(notif.contains("工具调用: bash"));
    assert!(notif.contains("子 Session: sub-agent"));

    // Only ToolCall ops produce failure results
    assert_eq!(failures.len(), 1);
    assert!(failures[0].contains("进程中断：网关重启"));
    assert!(failures[0].contains("bash"));
    assert!(failures[0].contains("tool_1"));
}
