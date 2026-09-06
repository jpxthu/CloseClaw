//! Tests for OutboundMessage exclusion from recovery notification text.
//!
//! Step 1.2 of the Run Health alignment plan removed OutboundMessage
//! from `build_notification_text` — these tests verify that behavior.

#[cfg(test)]
mod tests {
    use crate::persistence::{
        PendingOperation, PendingOperationDetail, PendingOperationStatus, PendingOperationType,
        PersistenceService, SessionCheckpoint,
    };
    use crate::recovery::SessionRecoveryService;
    use crate::storage::memory::MemoryStorage;
    use chrono::Utc;
    use std::sync::Arc;

    /// Verify that OutboundMessage pending operations are excluded from
    /// the recovery notification text (auto-redelivered by daemon startup).
    #[tokio::test]
    async fn test_notification_excludes_outbound_message_only() {
        let storage = Arc::new(MemoryStorage::new());
        let dirty = SessionCheckpoint::new("outbound-only".into()).with_pending_operations(vec![
            PendingOperation {
                status: PendingOperationStatus::Running,
                op_id: "out_1".into(),
                op_type: PendingOperationType::OutboundMessage,
                detail: PendingOperationDetail::OutboundMessage {
                    target_channel: "feishu".into(),
                    message_id: "om_msg123".into(),
                    delivery_status: "pending".into(),
                },
                created_at: Utc::now(),
            },
        ]);
        storage.save_checkpoint(&dirty).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        // Session is recovered but dirty (has pending ops)
        assert!(report.recovered.contains(&"outbound-only".to_string()));
        assert!(report.dirty_sessions.contains(&"outbound-only".to_string()));

        // Notification should NOT contain OutboundMessage content
        let loaded = storage
            .load_checkpoint("outbound-only")
            .await
            .unwrap()
            .unwrap();
        let notif = loaded.recovery_notification.unwrap();
        assert!(notif.contains("网关已重启"), "notification header present");
        assert!(!notif.contains("feishu"), "should not contain channel name");
        assert!(
            !notif.contains("om_msg123"),
            "should not contain message id"
        );
    }

    /// Verify that mixed ToolCall + OutboundMessage only shows ToolCall
    /// in the notification; OutboundMessage is silently excluded.
    #[tokio::test]
    async fn test_notification_mixed_ops_excludes_outbound() {
        let storage = Arc::new(MemoryStorage::new());
        let dirty = SessionCheckpoint::new("mixed-ops".into()).with_pending_operations(vec![
            PendingOperation {
                status: PendingOperationStatus::Running,
                op_id: "call_1".into(),
                op_type: PendingOperationType::ToolCall,
                detail: PendingOperationDetail::ToolCall {
                    tool_name: "exec".into(),
                    args_summary: r#"{"cmd":"echo hi"}"#.into(),
                },
                created_at: Utc::now(),
            },
            PendingOperation {
                status: PendingOperationStatus::Running,
                op_id: "out_1".into(),
                op_type: PendingOperationType::OutboundMessage,
                detail: PendingOperationDetail::OutboundMessage {
                    target_channel: "telegram".into(),
                    message_id: "tg_msg_789".into(),
                    delivery_status: "pending".into(),
                },
                created_at: Utc::now(),
            },
        ]);
        storage.save_checkpoint(&dirty).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        service.recover().await.unwrap();

        let loaded = storage.load_checkpoint("mixed-ops").await.unwrap().unwrap();
        let notif = loaded.recovery_notification.unwrap();
        // ToolCall IS present
        assert!(
            notif.contains("工具调用: exec"),
            "should contain ToolCall: {}",
            notif
        );
        // OutboundMessage is NOT present
        assert!(
            !notif.contains("telegram"),
            "should not contain OutboundMessage channel: {}",
            notif
        );
        assert!(
            !notif.contains("tg_msg_789"),
            "should not contain OutboundMessage id: {}",
            notif
        );
    }

    /// Verify that ToolCall + SubSessionSpawn (no OutboundMessage) still
    /// works correctly — no regression from the OutboundMessage skip.
    #[tokio::test]
    async fn test_notification_no_outbound_unaffected() {
        let storage = Arc::new(MemoryStorage::new());
        let dirty = SessionCheckpoint::new("no-outbound".into()).with_pending_operations(vec![
            PendingOperation {
                status: PendingOperationStatus::Running,
                op_id: "call_1".into(),
                op_type: PendingOperationType::ToolCall,
                detail: PendingOperationDetail::ToolCall {
                    tool_name: "bash".into(),
                    args_summary: "ls".into(),
                },
                created_at: Utc::now(),
            },
            PendingOperation {
                status: PendingOperationStatus::Running,
                op_id: "child_1".into(),
                op_type: PendingOperationType::SubSessionSpawn,
                detail: PendingOperationDetail::SubSessionSpawn {
                    child_session_id: "sub-agent".into(),
                    agent_id: "eda".into(),
                    task_summary: String::new(),
                },
                created_at: Utc::now(),
            },
        ]);
        storage.save_checkpoint(&dirty).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        service.recover().await.unwrap();

        let loaded = storage
            .load_checkpoint("no-outbound")
            .await
            .unwrap()
            .unwrap();
        let notif = loaded.recovery_notification.unwrap();
        assert!(
            notif.contains("工具调用: bash"),
            "should contain ToolCall: {}",
            notif
        );
        assert!(
            notif.contains("子 Session: sub-agent"),
            "should contain SubSessionSpawn: {}",
            notif
        );
    }

    /// Verify that tool_failures only contains ToolCall entries,
    /// not OutboundMessage entries.
    #[tokio::test]
    async fn test_tool_failures_excludes_outbound() {
        let storage = Arc::new(MemoryStorage::new());
        let dirty = SessionCheckpoint::new("fail-outbound".into()).with_pending_operations(vec![
            PendingOperation {
                status: PendingOperationStatus::Running,
                op_id: "call_1".into(),
                op_type: PendingOperationType::ToolCall,
                detail: PendingOperationDetail::ToolCall {
                    tool_name: "exec".into(),
                    args_summary: String::new(),
                },
                created_at: Utc::now(),
            },
            PendingOperation {
                status: PendingOperationStatus::Running,
                op_id: "out_1".into(),
                op_type: PendingOperationType::OutboundMessage,
                detail: PendingOperationDetail::OutboundMessage {
                    target_channel: "feishu".into(),
                    message_id: "om_abc".into(),
                    delivery_status: "pending".into(),
                },
                created_at: Utc::now(),
            },
        ]);
        storage.save_checkpoint(&dirty).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        service.recover().await.unwrap();

        let loaded = storage
            .load_checkpoint("fail-outbound")
            .await
            .unwrap()
            .unwrap();
        // Only ToolCall generates a tool_failure; OutboundMessage does not
        assert_eq!(loaded.pending_tool_failures.len(), 1);
        assert!(loaded.pending_tool_failures[0].contains("exec"));
        assert!(!loaded.pending_tool_failures[0].contains("feishu"));
    }
}
