//! Outbound-specific tests for pending_operations (Step 1.5).
//!
//! Covers:
//! - Checkpoint serialization/deserialization of OutboundMessage ops
//! - record_outbound_pending_op / clear_outbound_pending_op boundary values
//! - Crash simulation: write-ahead op survives, outbound_pending empty
//! - Mixed op types in checkpoint serialization

use crate::persistence::{
    PendingOperation, PendingOperationDetail, PendingOperationStatus, PendingOperationType,
    PersistenceService, SessionCheckpoint,
};

// ── checkpoint serialization/deserialization of OutboundMessage ───────────

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
            detail: PendingOperationDetail::ToolCall {
                tool_name: "bash".into(),
                args_summary: r#"{"command":"ls"}"#.into(),
            },
            created_at: now,
        },
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "child_1".into(),
            op_type: PendingOperationType::SubSessionSpawn,
            detail: PendingOperationDetail::SubSessionSpawn {
                child_session_id: "sub-agent-1".into(),
                agent_id: String::new(),
                task_summary: String::new(),
            },
            created_at: now,
        },
        PendingOperation {
            status: PendingOperationStatus::Running,
            op_id: "msg_1".into(),
            op_type: PendingOperationType::OutboundMessage,
            detail: PendingOperationDetail::OutboundMessage {
                target_channel: "outbound-chat".into(),
                message_id: "msg_1".into(),
                delivery_status: "hello world".into(),
            },
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
    assert_eq!(
        parsed.pending_operations[0].detail.tool_name(),
        Some("bash")
    );
    assert_eq!(
        parsed.pending_operations[0].detail.args_summary(),
        Some(r#"{"command":"ls"}"#)
    );
    assert_eq!(
        parsed.pending_operations[1].op_type,
        PendingOperationType::SubSessionSpawn
    );
    assert_eq!(
        parsed.pending_operations[2].op_type,
        PendingOperationType::OutboundMessage
    );
    assert_eq!(
        parsed.pending_operations[2].detail.delivery_status(),
        Some("hello world")
    );
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
        detail: PendingOperationDetail::ToolCall {
            tool_name: "test_tool".into(),
            args_summary: String::new(),
        },
        created_at: chrono::Utc::now(),
    }];

    let cp = SessionCheckpoint::new("sess_builder".into()).with_pending_operations(ops);
    assert_eq!(cp.pending_operations.len(), 1);
    assert_eq!(cp.pending_operations[0].op_id, "op_1");
}

// ── record_outbound_pending_op / clear_outbound_pending_op ──────────────

fn make_outbound_op(message_id: &str) -> PendingOperation {
    PendingOperation {
        op_id: message_id.into(),
        op_type: PendingOperationType::OutboundMessage,
        status: PendingOperationStatus::Running,
        detail: PendingOperationDetail::OutboundMessage {
            target_channel: "telegram".into(),
            message_id: message_id.into(),
            delivery_status: "pending".into(),
        },
        created_at: chrono::Utc::now(),
    }
}

#[test]
fn test_record_outbound_pending_op_pushes() {
    let mut cp = SessionCheckpoint::new("sess_push".into());
    assert!(cp.pending_operations.is_empty());

    cp.record_outbound_pending_op(make_outbound_op("msg_1"));
    cp.record_outbound_pending_op(make_outbound_op("msg_2"));

    assert_eq!(cp.pending_operations.len(), 2);
    assert_eq!(cp.pending_operations[0].op_id, "msg_1");
    assert_eq!(cp.pending_operations[1].op_id, "msg_2");
    assert!(cp
        .pending_operations
        .iter()
        .all(|op| op.op_type == PendingOperationType::OutboundMessage));
}

#[test]
fn test_clear_outbound_pending_op_removes_matching() {
    let mut cp = SessionCheckpoint::new("sess_clear".into());
    cp.record_outbound_pending_op(make_outbound_op("msg_1"));
    cp.record_outbound_pending_op(make_outbound_op("msg_2"));
    // Also add a non-outbound op to ensure it is not removed.
    cp.pending_operations.push(PendingOperation {
        op_id: "tool_1".into(),
        op_type: PendingOperationType::ToolCall,
        status: PendingOperationStatus::Running,
        detail: PendingOperationDetail::ToolCall {
            tool_name: "bash".into(),
            args_summary: String::new(),
        },
        created_at: chrono::Utc::now(),
    });

    cp.clear_outbound_pending_op("msg_1");

    assert_eq!(cp.pending_operations.len(), 2);
    assert_eq!(cp.pending_operations[0].op_id, "msg_2");
    assert_eq!(cp.pending_operations[1].op_id, "tool_1");
}

#[test]
fn test_clear_outbound_pending_op_idempotent() {
    let mut cp = SessionCheckpoint::new("sess_idempotent".into());
    cp.record_outbound_pending_op(make_outbound_op("msg_1"));

    // Clear a non-existent message_id — should not panic.
    cp.clear_outbound_pending_op("nonexistent");

    assert_eq!(cp.pending_operations.len(), 1);
    assert_eq!(cp.pending_operations[0].op_id, "msg_1");
}

#[test]
fn test_clear_outbound_pending_op_empty_vec_no_panic() {
    let mut cp = SessionCheckpoint::new("sess_empty_clear".into());
    // Clearing on an empty vec should not panic.
    cp.clear_outbound_pending_op("msg_1");
    assert!(cp.pending_operations.is_empty());
}

#[tokio::test]
async fn test_record_outbound_pending_op_persists_to_checkpoint() {
    let storage = std::sync::Arc::new(crate::storage::memory::MemoryStorage::new());

    // record_outbound_pending_op is a pure checkpoint method.
    // Verify it integrates with persistence by loading the checkpoint.
    let mut cp = SessionCheckpoint::new("outbound_push_persist".into());
    cp.record_outbound_pending_op(make_outbound_op("msg_persist"));
    storage.save_checkpoint(&cp).await.unwrap();

    let loaded = storage
        .load_checkpoint("outbound_push_persist")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.pending_operations.len(), 1);
    assert_eq!(loaded.pending_operations[0].op_id, "msg_persist");
    assert_eq!(
        loaded.pending_operations[0].op_type,
        PendingOperationType::OutboundMessage
    );
}

#[tokio::test]
async fn test_clear_outbound_pending_op_persists_to_checkpoint() {
    let storage = std::sync::Arc::new(crate::storage::memory::MemoryStorage::new());

    // Build a checkpoint with two outbound ops and one tool op.
    let mut cp = SessionCheckpoint::new("outbound_clear_persist".into());
    cp.record_outbound_pending_op(make_outbound_op("msg_a"));
    cp.record_outbound_pending_op(make_outbound_op("msg_b"));
    cp.pending_operations.push(PendingOperation {
        op_id: "tool_x".into(),
        op_type: PendingOperationType::ToolCall,
        status: PendingOperationStatus::Running,
        detail: PendingOperationDetail::ToolCall {
            tool_name: "grep".into(),
            args_summary: String::new(),
        },
        created_at: chrono::Utc::now(),
    });
    storage.save_checkpoint(&cp).await.unwrap();

    // Clear one outbound op, persist again.
    let mut loaded = storage
        .load_checkpoint("outbound_clear_persist")
        .await
        .unwrap()
        .unwrap();
    loaded.clear_outbound_pending_op("msg_a");
    storage.save_checkpoint(&loaded).await.unwrap();

    let final_cp = storage
        .load_checkpoint("outbound_clear_persist")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(final_cp.pending_operations.len(), 2);
    let ids: Vec<&str> = final_cp
        .pending_operations
        .iter()
        .map(|op| op.op_id.as_str())
        .collect();
    assert!(ids.contains(&"msg_b"));
    assert!(ids.contains(&"tool_x"));
    assert!(!ids.contains(&"msg_a"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Boundary values — multiple outbound pending ops
// ═══════════════════════════════════════════════════════════════════════════

/// Single session checkpoint with multiple OutboundMessage pending ops.
/// All ops are correctly recorded and can be individually cleared.
#[test]
fn test_multiple_outbound_pending_ops_record_and_clear() {
    let mut cp = SessionCheckpoint::new("multi_outbound".into());
    for i in 0..5 {
        cp.record_outbound_pending_op(make_outbound_op(&format!("msg_{}", i)));
    }
    assert_eq!(cp.pending_operations.len(), 5);
    cp.clear_outbound_pending_op("msg_2");
    assert_eq!(cp.pending_operations.len(), 4);
    let ids: Vec<&str> = cp
        .pending_operations
        .iter()
        .map(|op| op.op_id.as_str())
        .collect();
    assert!(!ids.contains(&"msg_2"));
    assert!(ids.contains(&"msg_0"));
    assert!(ids.contains(&"msg_4"));
    // Clear all remaining.
    for i in [0, 1, 3, 4] {
        cp.clear_outbound_pending_op(&format!("msg_{}", i));
    }
    assert_eq!(cp.pending_operations.len(), 0);
}

/// Multiple OutboundMessage ops coexist with ToolCall and SubSessionSpawn.
/// clear_outbound_pending_op only removes OutboundMessage ops.
#[test]
fn test_clear_outbound_only_affects_outbound_type() {
    let mut cp = SessionCheckpoint::new("mixed_clear".into());
    cp.record_outbound_pending_op(make_outbound_op("out_1"));
    cp.pending_operations.push(PendingOperation {
        op_id: "tool_1".into(),
        op_type: PendingOperationType::ToolCall,
        status: PendingOperationStatus::Running,
        detail: PendingOperationDetail::ToolCall {
            tool_name: "bash".into(),
            args_summary: String::new(),
        },
        created_at: chrono::Utc::now(),
    });
    cp.record_outbound_pending_op(make_outbound_op("out_2"));
    cp.pending_operations.push(PendingOperation {
        op_id: "child_1".into(),
        op_type: PendingOperationType::SubSessionSpawn,
        status: PendingOperationStatus::Running,
        detail: PendingOperationDetail::SubSessionSpawn {
            child_session_id: "child_1".into(),
            agent_id: String::new(),
            task_summary: String::new(),
        },
        created_at: chrono::Utc::now(),
    });
    assert_eq!(cp.pending_operations.len(), 4);
    cp.clear_outbound_pending_op("out_1");
    assert_eq!(cp.pending_operations.len(), 3);
    let op_ids: Vec<&str> = cp
        .pending_operations
        .iter()
        .map(|op| op.op_id.as_str())
        .collect();
    assert!(!op_ids.contains(&"out_1"));
    assert!(op_ids.contains(&"tool_1"));
    assert!(op_ids.contains(&"out_2"));
    assert!(op_ids.contains(&"child_1"));
}

/// OutboundMessage pending ops survive checkpoint serialization roundtrip
/// with mixed operation types.
#[test]
fn test_outbound_ops_roundtrip_with_mixed_types() {
    let mut cp = SessionCheckpoint::new("mixed_rt".into());
    cp.record_outbound_pending_op(make_outbound_op("out_a"));
    cp.pending_operations.push(PendingOperation {
        op_id: "tool_x".into(),
        op_type: PendingOperationType::ToolCall,
        status: PendingOperationStatus::Running,
        detail: PendingOperationDetail::ToolCall {
            tool_name: "grep".into(),
            args_summary: "pattern".into(),
        },
        created_at: chrono::Utc::now(),
    });
    cp.record_outbound_pending_op(make_outbound_op("out_b"));
    let json = serde_json::to_string(&cp).unwrap();
    let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.pending_operations.len(), 3);
    let outbound_count = parsed
        .pending_operations
        .iter()
        .filter(|op| op.op_type == PendingOperationType::OutboundMessage)
        .count();
    assert_eq!(outbound_count, 2);
}

/// Boundary: clear_outbound_pending_op on a checkpoint with only
/// non-OutboundMessage ops does nothing.
#[test]
fn test_clear_outbound_on_non_outbound_ops_no_effect() {
    let mut cp = SessionCheckpoint::new("no_outbound".into());
    cp.pending_operations.push(PendingOperation {
        op_id: "tool_1".into(),
        op_type: PendingOperationType::ToolCall,
        status: PendingOperationStatus::Running,
        detail: PendingOperationDetail::ToolCall {
            tool_name: "exec".into(),
            args_summary: String::new(),
        },
        created_at: chrono::Utc::now(),
    });
    cp.pending_operations.push(PendingOperation {
        op_id: "child_1".into(),
        op_type: PendingOperationType::SubSessionSpawn,
        status: PendingOperationStatus::Running,
        detail: PendingOperationDetail::SubSessionSpawn {
            child_session_id: "child_1".into(),
            agent_id: String::new(),
            task_summary: String::new(),
        },
        created_at: chrono::Utc::now(),
    });

    // Clear by op_id — but since they are not OutboundMessage, nothing is removed.
    cp.clear_outbound_pending_op("tool_1");
    cp.clear_outbound_pending_op("child_1");
    cp.clear_outbound_pending_op("nonexistent");
    assert_eq!(cp.pending_operations.len(), 2);
}

/// Boundary: record_outbound_pending_op with duplicate message_id
/// creates multiple ops (no dedup, per design doc). Clearing by
/// message_id removes ALL matching entries (retain-based).
#[test]
fn test_record_outbound_duplicate_message_id_no_dedup() {
    let mut cp = SessionCheckpoint::new("dup_msg".into());
    cp.record_outbound_pending_op(make_outbound_op("dup"));
    cp.record_outbound_pending_op(make_outbound_op("dup"));
    assert_eq!(cp.pending_operations.len(), 2, "no dedup");
    cp.clear_outbound_pending_op("dup");
    assert_eq!(cp.pending_operations.len(), 0, "clear removes all");
}

// ═══════════════════════════════════════════════════════════════════════════
// Step 1.5 — Crash simulation: write-ahead op in checkpoint, no cache
// ═══════════════════════════════════════════════════════════════════════════

/// Simulate crash after write-ahead but before outbound_pending cache
/// entry is added. The checkpoint contains the OutboundMessage op but
/// outbound_pending is empty — op survives crash for later recovery.
#[test]
fn test_outbound_op_survives_crash_no_cache() {
    let mut cp = SessionCheckpoint::new("crash_no_cache".into());
    cp.record_outbound_pending_op(make_outbound_op("msg_crash"));
    // outbound_pending is intentionally empty.
    assert_eq!(cp.pending_operations.len(), 1);
    assert_eq!(cp.outbound_pending.len(), 0);

    // Verify op survives serialization roundtrip (crash → restart).
    let json = serde_json::to_string(&cp).unwrap();
    let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.pending_operations.len(), 1);
    assert_eq!(
        parsed.pending_operations[0].op_type,
        PendingOperationType::OutboundMessage
    );
    assert_eq!(parsed.outbound_pending.len(), 0);
}
