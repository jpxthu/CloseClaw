//! Integration tests for Step 1.6: crash recovery scenarios.
//!
//! Verifies that non-Graceful shutdown (crash) recovery correctly:
//! - Detects dirty sessions with pending_operations
//! - Builds recovery notifications with correct detail fields
//! - Persists notification and tool failure results in checkpoint

use crate::persistence::{
    PendingOperation, PendingOperationDetail, PendingOperationStatus, PendingOperationType,
    PersistenceService, SessionCheckpoint,
};
use crate::recovery::{format_duration_seconds, SessionRecoveryService};
use crate::storage::memory::MemoryStorage;
use chrono::Utc;
use std::sync::Arc;

// ── Step 1.6: crash recovery detection ──────────────────────────────────

/// Simulate: tool call registered → checkpoint saved → crash (no deregister)
/// → restart recovery → verify dirty session detected.
#[tokio::test]
async fn test_crash_recovery_tool_call_detected_as_dirty() {
    let storage = Arc::new(MemoryStorage::new());
    let now = Utc::now();

    let cp = SessionCheckpoint::new("crash_tool".into()).with_pending_operations(vec![
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "call_crash_1".into(),
            op_type: PendingOperationType::ToolCall,
            detail: PendingOperationDetail::ToolCall {
                tool_name: "bash".into(),
                args_summary: r#"{"command":"ls -la"}"#.into(),
            },
            created_at: now,
        },
    ]);
    storage.save_checkpoint(&cp).await.unwrap();

    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await.unwrap();

    assert_eq!(report.recovered.len(), 1);
    assert!(
        report.dirty_sessions.contains(&"crash_tool".to_string()),
        "session with pending tool call should be detected as dirty"
    );
}

/// Simulate: child registered → checkpoint saved → crash (no deregister)
/// → restart recovery → verify notification contains SubSessionSpawn item.
#[tokio::test]
async fn test_crash_recovery_child_session_notification() {
    let storage = Arc::new(MemoryStorage::new());
    let now = Utc::now();

    let cp = SessionCheckpoint::new("crash_child".into()).with_pending_operations(vec![
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "child_crash_1".into(),
            op_type: PendingOperationType::SubSessionSpawn,
            detail: PendingOperationDetail::SubSessionSpawn {
                child_session_id: "child-agent-42".into(),
                agent_id: "worker".into(),
                task_summary: "process data batch".into(),
            },
            created_at: now,
        },
    ]);
    storage.save_checkpoint(&cp).await.unwrap();

    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await.unwrap();

    assert_eq!(report.recovered.len(), 1);
    assert!(
        report.dirty_sessions.contains(&"crash_child".to_string()),
        "session with pending child spawn should be dirty"
    );

    let loaded = storage
        .load_checkpoint("crash_child")
        .await
        .unwrap()
        .unwrap();
    let notif = loaded.recovery_notification.unwrap();
    assert!(notif.contains("子 Session"), "got: {}", notif);
    assert!(notif.contains("child-agent-42"), "got: {}", notif);
    assert!(notif.contains("已运行"), "got: {}", notif);
}

/// Verify recovery notification detail fields are correctly filled
/// for all three pending operation types.
#[tokio::test]
async fn test_crash_recovery_notification_detail_fields() {
    let storage = Arc::new(MemoryStorage::new());
    let now = Utc::now();
    let cp = SessionCheckpoint::new("crash_detail".into()).with_pending_operations(vec![
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "tool_detail".into(),
            op_type: PendingOperationType::ToolCall,
            detail: PendingOperationDetail::ToolCall {
                tool_name: "web_search".into(),
                args_summary: r#"{"query":"rust async"}"#.into(),
            },
            created_at: now,
        },
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "child_detail".into(),
            op_type: PendingOperationType::SubSessionSpawn,
            detail: PendingOperationDetail::SubSessionSpawn {
                child_session_id: "sub-proc-99".into(),
                agent_id: "analyst".into(),
                task_summary: "analyze logs".into(),
            },
            created_at: now,
        },
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "msg_detail".into(),
            op_type: PendingOperationType::OutboundMessage,
            detail: PendingOperationDetail::OutboundMessage {
                target_channel: "feishu".into(),
                message_id: "msg_xyz".into(),
                delivery_status: "pending".into(),
            },
            created_at: now,
        },
    ]);
    storage.save_checkpoint(&cp).await.unwrap();

    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await.unwrap();
    assert_eq!(report.recovered.len(), 1);
    assert!(report.dirty_sessions.contains(&"crash_detail".to_string()));

    let loaded = storage
        .load_checkpoint("crash_detail")
        .await
        .unwrap()
        .unwrap();
    let notif = loaded.recovery_notification.unwrap();

    // Tool call: tool_name and args_summary
    assert!(notif.contains("web_search"), "tool_name: {}", notif);
    assert!(
        notif.contains(r#"{"query":"rust async"}"#),
        "args_summary: {}",
        notif
    );

    // SubSessionSpawn: child_session_id and duration
    assert!(notif.contains("sub-proc-99"), "child_id: {}", notif);
    assert!(notif.contains("已运行"), "duration: {}", notif);

    // OutboundMessage: message_id
    assert!(notif.contains("msg_xyz"), "msg_id: {}", notif);

    // Tool failures: only ToolCall ops produce failure results
    assert_eq!(loaded.pending_tool_failures.len(), 1);
    assert!(loaded.pending_tool_failures[0].contains("web_search"));
    assert!(loaded.pending_tool_failures[0].contains("进程中断：网关重启"));
}

/// Simulate crash with multiple pending operations and verify all
/// are correctly detected and notified.
#[tokio::test]
async fn test_crash_recovery_mixed_operations_all_detected() {
    let storage = Arc::new(MemoryStorage::new());
    let now = Utc::now();

    let cp = SessionCheckpoint::new("crash_mixed".into()).with_pending_operations(vec![
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "t1".into(),
            op_type: PendingOperationType::ToolCall,
            detail: PendingOperationDetail::ToolCall {
                tool_name: "exec".into(),
                args_summary: String::new(),
            },
            created_at: now,
        },
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "t2".into(),
            op_type: PendingOperationType::ToolCall,
            detail: PendingOperationDetail::ToolCall {
                tool_name: "bash".into(),
                args_summary: String::new(),
            },
            created_at: now,
        },
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "c1".into(),
            op_type: PendingOperationType::SubSessionSpawn,
            detail: PendingOperationDetail::SubSessionSpawn {
                child_session_id: "child_a".into(),
                agent_id: String::new(),
                task_summary: String::new(),
            },
            created_at: now,
        },
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "m1".into(),
            op_type: PendingOperationType::OutboundMessage,
            detail: PendingOperationDetail::OutboundMessage {
                target_channel: "feishu".into(),
                message_id: "m1".into(),
                delivery_status: "pending".into(),
            },
            created_at: now,
        },
    ]);
    storage.save_checkpoint(&cp).await.unwrap();

    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await.unwrap();
    assert_eq!(report.recovered.len(), 1);
    assert!(report.dirty_sessions.contains(&"crash_mixed".to_string()));

    let loaded = storage
        .load_checkpoint("crash_mixed")
        .await
        .unwrap()
        .unwrap();
    let notif = loaded.recovery_notification.unwrap();
    assert!(notif.contains("exec"));
    assert!(notif.contains("bash"));
    assert!(notif.contains("child_a"));
    assert!(notif.contains("已运行"));
    assert!(notif.contains("m1"));

    // 2 tool calls → 2 failure results
    assert_eq!(loaded.pending_tool_failures.len(), 2);
}

/// Verify clean session is NOT marked dirty after recovery.
#[tokio::test]
async fn test_crash_recovery_clean_session_not_dirty() {
    let storage = Arc::new(MemoryStorage::new());
    let cp = SessionCheckpoint::new("crash_clean".into());
    assert!(cp.pending_operations.is_empty());
    storage.save_checkpoint(&cp).await.unwrap();

    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await.unwrap();
    assert_eq!(report.recovered.len(), 1);
    assert!(
        report.dirty_sessions.is_empty(),
        "clean session should not be dirty"
    );

    let loaded = storage
        .load_checkpoint("crash_clean")
        .await
        .unwrap()
        .unwrap();
    assert!(loaded.recovery_notification.is_none());
    assert!(loaded.pending_tool_failures.is_empty());
}

/// Verify notification and tool failures are persisted in checkpoint
/// after crash recovery.
#[tokio::test]
async fn test_crash_recovery_persists_notification_in_checkpoint() {
    let storage = Arc::new(MemoryStorage::new());
    let now = Utc::now();
    let cp = SessionCheckpoint::new("crash_persist".into()).with_pending_operations(vec![
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "op_persist".into(),
            op_type: PendingOperationType::ToolCall,
            detail: PendingOperationDetail::ToolCall {
                tool_name: "grep".into(),
                args_summary: "pattern".into(),
            },
            created_at: now,
        },
    ]);
    storage.save_checkpoint(&cp).await.unwrap();

    // Before recovery: no notification
    let before = storage
        .load_checkpoint("crash_persist")
        .await
        .unwrap()
        .unwrap();
    assert!(before.recovery_notification.is_none());
    assert!(before.pending_tool_failures.is_empty());

    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await.unwrap();
    assert!(report.dirty_sessions.contains(&"crash_persist".to_string()));

    // After recovery: notification and tool failures persisted
    let after = storage
        .load_checkpoint("crash_persist")
        .await
        .unwrap()
        .unwrap();
    assert!(after.recovery_notification.is_some());
    assert_eq!(after.pending_tool_failures.len(), 1);
    assert!(after.pending_tool_failures[0].contains("grep"));
}

// ── Step 1.6: full lifecycle integration ────────────────────────────────

/// Full lifecycle: register → collect_pending_operations → save → crash
/// → recover → verify notification with correct detail.
#[tokio::test]
async fn test_crash_recovery_full_lifecycle_tool_call() {
    use crate::llm_session::ConversationSession;

    let storage = Arc::new(MemoryStorage::new());
    let session = ConversationSession::new(
        "lifecycle_tool".into(),
        "test-model".into(),
        std::path::PathBuf::from("/tmp"),
    );

    // Register a tool call (simulates tool fork before crash)
    session.register_tool_call("call_lifecycle_1", "bash", "ls -la");

    // Collect pending operations (simulates shutdown checkpoint save)
    let ops = session.collect_pending_operations();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, PendingOperationType::ToolCall);
    assert_eq!(ops[0].detail.tool_name(), Some("bash"));
    assert_eq!(ops[0].detail.args_summary(), Some("ls -la"));

    // Save checkpoint (then crash — no deregister)
    let cp = SessionCheckpoint::new("lifecycle_tool".into()).with_pending_operations(ops);
    storage.save_checkpoint(&cp).await.unwrap();

    // Restart and recover
    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await.unwrap();
    assert!(report
        .dirty_sessions
        .contains(&"lifecycle_tool".to_string()));

    let loaded = storage
        .load_checkpoint("lifecycle_tool")
        .await
        .unwrap()
        .unwrap();
    let notif = loaded.recovery_notification.unwrap();
    assert!(notif.contains("bash"));
    assert!(notif.contains("ls -la"));
    assert!(notif.contains("发起于"));
}

/// Full lifecycle with child session registration.
#[tokio::test]
async fn test_crash_recovery_full_lifecycle_child_session() {
    use crate::llm_session::ConversationSession;

    let storage = Arc::new(MemoryStorage::new());
    let session = ConversationSession::new(
        "lifecycle_child".into(),
        "test-model".into(),
        std::path::PathBuf::from("/tmp"),
    );

    // Register a child session (simulates spawn before crash)
    session.register_child("child_lc_1", "worker", "run tests");

    // Collect pending operations
    let ops = session.collect_pending_operations();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, PendingOperationType::SubSessionSpawn);
    assert_eq!(ops[0].detail.child_session_id(), Some("child_lc_1"));

    // Save checkpoint (then crash)
    let cp = SessionCheckpoint::new("lifecycle_child".into()).with_pending_operations(ops);
    storage.save_checkpoint(&cp).await.unwrap();

    // Recover
    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await.unwrap();
    assert!(report
        .dirty_sessions
        .contains(&"lifecycle_child".to_string()));

    let loaded = storage
        .load_checkpoint("lifecycle_child")
        .await
        .unwrap()
        .unwrap();
    let notif = loaded.recovery_notification.unwrap();
    assert!(notif.contains("子 Session"));
    assert!(notif.contains("child_lc_1"));
    assert!(notif.contains("已运行"));
}

/// Verify that after crash recovery, deregister works and the session
/// can return to a clean state.
#[tokio::test]
async fn test_crash_recovery_then_deregister_cleans_session() {
    use crate::llm_session::ConversationSession;

    let storage = Arc::new(MemoryStorage::new());
    let session = ConversationSession::new(
        "lifecycle_clean".into(),
        "test-model".into(),
        std::path::PathBuf::from("/tmp"),
    );

    // Register then collect (simulates crash recovery followed by
    // successful completion)
    session.register_tool_call("call_clean", "exec", "echo ok");
    let ops_before = session.collect_pending_operations();
    assert_eq!(ops_before.len(), 1);

    // Save checkpoint with pending op
    let cp = SessionCheckpoint::new("lifecycle_clean".into()).with_pending_operations(ops_before);
    storage.save_checkpoint(&cp).await.unwrap();

    // Recover (checkpoint has pending op)
    let service = SessionRecoveryService::new(Arc::clone(&storage));
    let report = service.recover().await.unwrap();
    assert!(report
        .dirty_sessions
        .contains(&"lifecycle_clean".to_string()));

    // Deregister (simulates successful completion after restart)
    session.deregister_tool_call("call_clean");
    let ops_after = session.collect_pending_operations();
    assert!(ops_after.is_empty());
}

// ── Step 1.5: recovery notification duration format tests ─────────────

#[test]
fn test_format_duration_seconds_various() {
    assert_eq!(format_duration_seconds(0), "0s");
    assert_eq!(format_duration_seconds(30), "30s");
    assert_eq!(format_duration_seconds(90), "1m30s");
    assert_eq!(format_duration_seconds(3661), "1h1m1s");
    assert_eq!(format_duration_seconds(86400), "1d");
    assert_eq!(format_duration_seconds(-10), "0s");
}
