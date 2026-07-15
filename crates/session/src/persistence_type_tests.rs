//! Type-level serde and display tests for persistence types.
//!
//! Moved from `persistence.rs` inline `#[cfg(test)]` module to comply
//! with the 1000-line file limit.

#[cfg(test)]
mod tests {
    use crate::persistence::{DreamingStatus, ReasoningLevel, SessionCheckpoint};

    #[test]
    fn test_reasoning_level_basics() {
        assert_eq!(ReasoningLevel::default(), ReasoningLevel::High);
        assert_eq!(ReasoningLevel::Low.to_string(), "low");
        assert_eq!(ReasoningLevel::Medium.to_string(), "medium");
        assert_eq!(ReasoningLevel::High.to_string(), "high");
        assert_eq!(ReasoningLevel::Max.to_string(), "max");
        // serde roundtrip
        for level in [
            ReasoningLevel::Low,
            ReasoningLevel::Medium,
            ReasoningLevel::High,
            ReasoningLevel::Max,
        ] {
            let json = serde_json::to_string(&level).unwrap();
            let parsed: ReasoningLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(level, parsed);
        }
        // deserialize from string
        assert_eq!(
            serde_json::from_str::<ReasoningLevel>("\"low\"").unwrap(),
            ReasoningLevel::Low
        );
        assert_eq!(
            serde_json::from_str::<ReasoningLevel>("\"medium\"").unwrap(),
            ReasoningLevel::Medium
        );
        assert_eq!(
            serde_json::from_str::<ReasoningLevel>("\"high\"").unwrap(),
            ReasoningLevel::High
        );
        assert_eq!(
            serde_json::from_str::<ReasoningLevel>("\"max\"").unwrap(),
            ReasoningLevel::Max
        );
    }

    #[test]
    fn test_session_checkpoint_reasoning_level() {
        let checkpoint = SessionCheckpoint::new("sess_1".into());
        assert_eq!(checkpoint.reasoning_level, ReasoningLevel::High);
        let checkpoint =
            SessionCheckpoint::new("sess_2".into()).with_reasoning_level(ReasoningLevel::Low);
        assert_eq!(checkpoint.reasoning_level, ReasoningLevel::Low);
        assert!(serde_json::from_str::<ReasoningLevel>("\"extreme\"").is_err());
    }

    #[test]
    fn test_dreaming_status_defaults_display_and_serde() {
        // serde default stays Completed for backward compat with old JSON data
        assert_eq!(DreamingStatus::default(), DreamingStatus::Completed);
        // new checkpoint defaults to Pending
        let checkpoint = SessionCheckpoint::new("sess_pending".into());
        assert_eq!(checkpoint.dreaming_status, DreamingStatus::Pending);
        assert_eq!(DreamingStatus::Pending.to_string(), "pending");
        assert_eq!(DreamingStatus::InLight.to_string(), "in_light");
        assert_eq!(DreamingStatus::InRem.to_string(), "in_rem");
        assert_eq!(DreamingStatus::InDeep.to_string(), "in_deep");
        assert_eq!(DreamingStatus::Completed.to_string(), "completed");
        // serde roundtrip
        for status in [
            DreamingStatus::Pending,
            DreamingStatus::InLight,
            DreamingStatus::InRem,
            DreamingStatus::InDeep,
            DreamingStatus::Completed,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let parsed: DreamingStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, parsed);
        }
        // deserialize from string
        assert_eq!(
            serde_json::from_str::<DreamingStatus>("\"pending\"").unwrap(),
            DreamingStatus::Pending
        );
        assert_eq!(
            serde_json::from_str::<DreamingStatus>("\"in_light\"").unwrap(),
            DreamingStatus::InLight
        );
        assert_eq!(
            serde_json::from_str::<DreamingStatus>("\"completed\"").unwrap(),
            DreamingStatus::Completed
        );
    }

    #[test]
    fn test_session_checkpoint_mined_dreaming_status() {
        // defaults
        let checkpoint = SessionCheckpoint::new("sess_mined".into());
        assert!(!checkpoint.mined, "mined should default to false");
        assert_eq!(checkpoint.dreaming_status, DreamingStatus::Pending);
        // setters
        let checkpoint = SessionCheckpoint::new("sess_mined".into()).with_mined(true);
        assert!(checkpoint.mined);
        let checkpoint =
            SessionCheckpoint::new("sess_dream".into()).with_dreaming_status(DreamingStatus::InRem);
        assert_eq!(checkpoint.dreaming_status, DreamingStatus::InRem);
        // serde roundtrip
        let cp = SessionCheckpoint::new("s-roundtrip-md".into())
            .with_mined(true)
            .with_dreaming_status(DreamingStatus::InLight);
        let json = serde_json::to_string(&cp).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
        assert!(parsed.mined);
        assert_eq!(parsed.dreaming_status, DreamingStatus::InLight);
        // missing fields default
        let mut json_value: serde_json::Value = serde_json::to_value(&cp).unwrap();
        json_value.as_object_mut().unwrap().remove("mined");
        json_value
            .as_object_mut()
            .unwrap()
            .remove("dreaming_status");
        let json_str = serde_json::to_string(&json_value).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json_str).unwrap();
        assert!(
            !parsed.mined,
            "old data without mined should default to false"
        );
        assert_eq!(parsed.dreaming_status, DreamingStatus::Completed);
    }

    #[test]
    fn test_communication_config_serde_roundtrip_and_defaults() {
        // Default is None
        let cp = SessionCheckpoint::new("s-comm".into());
        assert!(cp.communication_config.is_none());
        // Set via builder
        let config = closeclaw_common::communication::CommunicationConfig::default_with_parent(
            Some("parent-agent"),
        );
        let cp = SessionCheckpoint::new("s-comm2".into()).with_communication_config(config.clone());
        let stored = cp.communication_config.as_ref().unwrap();
        assert_eq!(stored.outbound, vec!["parent-agent".to_string()]);
        assert_eq!(stored.inbound, vec!["parent-agent".to_string()]);
        // Serde roundtrip
        let json = serde_json::to_string(&cp).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
        let stored = parsed.communication_config.as_ref().unwrap();
        assert_eq!(stored.outbound, vec!["parent-agent".to_string()]);
        assert_eq!(stored.inbound, vec!["parent-agent".to_string()]);
    }

    #[test]
    fn test_communication_config_missing_json_defaults_to_none() {
        let cp = SessionCheckpoint::new("s-old".into());
        let mut json_value: serde_json::Value = serde_json::to_value(&cp).unwrap();
        json_value
            .as_object_mut()
            .unwrap()
            .remove("communication_config");
        let json_str = serde_json::to_string(&json_value).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json_str).unwrap();
        assert!(
            parsed.communication_config.is_none(),
            "old checkpoint without communication_config should default to None"
        );
    }

    #[test]
    fn test_pending_operation_status_default_is_running() {
        use crate::persistence::PendingOperationStatus;
        let status = PendingOperationStatus::default();
        assert_eq!(status, PendingOperationStatus::Running);
    }

    #[test]
    fn test_pending_operation_status_display() {
        use crate::persistence::PendingOperationStatus;
        assert_eq!(PendingOperationStatus::Running.to_string(), "running");
    }

    #[test]
    fn test_pending_operation_status_serde_roundtrip() {
        use crate::persistence::PendingOperationStatus;
        let json = serde_json::to_string(&PendingOperationStatus::Running).unwrap();
        assert_eq!(json, "\"running\"");
        let parsed: PendingOperationStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, PendingOperationStatus::Running);
    }

    #[test]
    fn test_pending_operation_new_has_status_running() {
        use crate::persistence::{
            PendingOperation, PendingOperationDetail, PendingOperationStatus, PendingOperationType,
        };
        let op = PendingOperation {
            op_id: "op1".into(),
            op_type: PendingOperationType::ToolCall,
            status: PendingOperationStatus::Running,
            detail: PendingOperationDetail::ToolCall {
                tool_name: "test".into(),
                args_summary: String::new(),
            },
            created_at: chrono::Utc::now(),
        };
        assert_eq!(op.status, PendingOperationStatus::Running);
    }

    #[test]
    fn test_pending_operation_serde_roundtrip_with_status() {
        use crate::persistence::{
            PendingOperation, PendingOperationDetail, PendingOperationStatus, PendingOperationType,
        };
        let op = PendingOperation {
            op_id: "op1".into(),
            op_type: PendingOperationType::ToolCall,
            status: PendingOperationStatus::Running,
            detail: PendingOperationDetail::ToolCall {
                tool_name: "test".into(),
                args_summary: String::new(),
            },
            created_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&op).unwrap();
        let parsed: PendingOperation = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.status, PendingOperationStatus::Running);
        assert!(json.contains("\"status\":\"running\""));
    }

    #[test]
    fn test_pending_operation_old_json_without_status_defaults_to_running() {
        use crate::persistence::{PendingOperation, PendingOperationStatus};
        // Simulate old checkpoint JSON without status field
        let old_json = r#"{
            "op_id": "op_old",
            "op_type": "tool_call",
            "name": "old_tool",
            "args": "{}",
            "created_at": "2026-01-01T00:00:00Z"
        }"#;
        let parsed: PendingOperation = serde_json::from_str(old_json).unwrap();
        assert_eq!(
            parsed.status,
            PendingOperationStatus::Running,
            "old PendingOperation JSON without status field should default to Running"
        );
    }

    #[test]
    fn test_pending_operation_in_checkpoint_old_json_without_status() {
        // Verify that a checkpoint containing old PendingOperation without
        // status field still deserializes correctly
        let cp = SessionCheckpoint::new("s-old-pending".into());
        let mut json_value: serde_json::Value = serde_json::to_value(&cp).unwrap();
        // Insert a pending operation without status field
        json_value["pending_operations"] = serde_json::json!([{
            "op_id": "old_op",
            "op_type": "tool_call",
            "name": "old_tool",
            "args": "{}",
            "created_at": "2026-01-01T00:00:00Z"
        }]);
        let json_str = serde_json::to_string(&json_value).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.pending_operations.len(), 1);
        assert_eq!(
            parsed.pending_operations[0].status,
            crate::persistence::PendingOperationStatus::Running,
            "old checkpoint PendingOperation without status should default to Running"
        );
    }

    // ===================================================================
    // PendingOperationDetail tests (Step 1.4)
    // ===================================================================

    #[test]
    fn test_pending_operation_detail_tool_call_construction() {
        use crate::persistence::{
            PendingOperation, PendingOperationDetail, PendingOperationStatus, PendingOperationType,
        };
        let detail = PendingOperationDetail::ToolCall {
            tool_name: "bash".into(),
            args_summary: "ls -la".into(),
        };
        let op = PendingOperation {
            op_id: "op-tool".into(),
            op_type: PendingOperationType::ToolCall,
            status: PendingOperationStatus::Running,
            detail,
            created_at: chrono::Utc::now(),
        };
        assert_eq!(op.detail.tool_name(), Some("bash"));
        assert_eq!(op.detail.args_summary(), Some("ls -la"));
        assert_eq!(op.detail.child_session_id(), None);
        assert_eq!(op.detail.target_channel(), None);
    }

    #[test]
    fn test_pending_operation_detail_sub_session_spawn_construction() {
        use crate::persistence::{
            PendingOperation, PendingOperationDetail, PendingOperationStatus, PendingOperationType,
        };
        let detail = PendingOperationDetail::SubSessionSpawn {
            child_session_id: "child-001".into(),
            agent_id: "agent-eda".into(),
            task_summary: "Run unit tests".into(),
        };
        let op = PendingOperation {
            op_id: "op-spawn".into(),
            op_type: PendingOperationType::SubSessionSpawn,
            status: PendingOperationStatus::Running,
            detail,
            created_at: chrono::Utc::now(),
        };
        assert_eq!(op.detail.child_session_id(), Some("child-001"));
        assert_eq!(op.detail.tool_name(), None);
        assert_eq!(op.detail.target_channel(), None);
    }

    #[test]
    fn test_pending_operation_detail_outbound_message_construction() {
        use crate::persistence::{
            PendingOperation, PendingOperationDetail, PendingOperationStatus, PendingOperationType,
        };
        let detail = PendingOperationDetail::OutboundMessage {
            target_channel: "feishu".into(),
            message_id: "om_msg123".into(),
            delivery_status: "pending".into(),
        };
        let op = PendingOperation {
            op_id: "op-out".into(),
            op_type: PendingOperationType::OutboundMessage,
            status: PendingOperationStatus::Running,
            detail,
            created_at: chrono::Utc::now(),
        };
        assert_eq!(op.detail.target_channel(), Some("feishu"));
        assert_eq!(op.detail.message_id(), Some("om_msg123"));
        assert_eq!(op.detail.delivery_status(), Some("pending"));
        assert_eq!(op.detail.tool_name(), None);
    }

    #[test]
    fn test_pending_operation_detail_tool_call_empty_fields() {
        use crate::persistence::PendingOperationDetail;
        let detail = PendingOperationDetail::ToolCall {
            tool_name: String::new(),
            args_summary: String::new(),
        };
        assert_eq!(detail.tool_name(), Some(""));
        assert_eq!(detail.args_summary(), Some(""));
    }

    #[test]
    fn test_pending_operation_detail_sub_session_spawn_empty_fields() {
        use crate::persistence::PendingOperationDetail;
        let detail = PendingOperationDetail::SubSessionSpawn {
            child_session_id: String::new(),
            agent_id: String::new(),
            task_summary: String::new(),
        };
        assert_eq!(detail.child_session_id(), Some(""));
        assert_eq!(detail.tool_name(), None);
    }

    #[test]
    fn test_pending_operation_detail_outbound_message_empty_fields() {
        use crate::persistence::PendingOperationDetail;
        let detail = PendingOperationDetail::OutboundMessage {
            target_channel: String::new(),
            message_id: String::new(),
            delivery_status: String::new(),
        };
        assert_eq!(detail.target_channel(), Some(""));
        assert_eq!(detail.message_id(), Some(""));
        assert_eq!(detail.delivery_status(), Some(""));
    }

    #[test]
    fn test_pending_operation_detail_serde_roundtrip_all_variants() {
        use crate::persistence::PendingOperationDetail;
        let variants = vec![
            PendingOperationDetail::ToolCall {
                tool_name: "web_search".into(),
                args_summary: r#"{"q": "rust"}"#.into(),
            },
            PendingOperationDetail::SubSessionSpawn {
                child_session_id: "child-abc".into(),
                agent_id: "agent-test".into(),
                task_summary: "Analyze code".into(),
            },
            PendingOperationDetail::OutboundMessage {
                target_channel: "telegram".into(),
                message_id: "tg_msg_456".into(),
                delivery_status: "sent".into(),
            },
        ];
        for variant in variants {
            let json = serde_json::to_string(&variant).unwrap();
            let parsed: PendingOperationDetail = serde_json::from_str(&json).unwrap();
            assert_eq!(variant, parsed);
        }
    }

    #[test]
    fn test_pending_operation_detail_serde_tag_format() {
        use crate::persistence::PendingOperationDetail;
        // Verify the tagged enum serialization includes "variant" key
        let detail = PendingOperationDetail::ToolCall {
            tool_name: "bash".into(),
            args_summary: "echo hi".into(),
        };
        let json_value = serde_json::to_value(&detail).unwrap();
        assert_eq!(json_value["variant"], "tool_call");
        assert_eq!(json_value["tool_name"], "bash");
        assert_eq!(json_value["args_summary"], "echo hi");
    }

    #[test]
    fn test_pending_operation_old_format_name_args_to_tool_call() {
        use crate::persistence::PendingOperation;
        // Old format: name + args (no detail field)
        let old_json = r#"{
            "op_id": "op_legacy",
            "op_type": "tool_call",
            "status": "running",
            "name": "web_search",
            "args": "{\"query\": \"hello\"}",
            "created_at": "2026-01-01T00:00:00Z"
        }"#;
        let parsed: PendingOperation = serde_json::from_str(old_json).unwrap();
        assert_eq!(parsed.op_id, "op_legacy");
        assert_eq!(
            parsed.op_type,
            crate::persistence::PendingOperationType::ToolCall
        );
        // Old name+args should be converted to ToolCall variant
        assert_eq!(parsed.detail.tool_name(), Some("web_search"));
        assert_eq!(parsed.detail.args_summary(), Some(r#"{"query": "hello"}"#));
    }

    #[test]
    fn test_pending_operation_old_format_empty_name_args_to_tool_call() {
        use crate::persistence::PendingOperation;
        // Old format with missing name and args → default to empty strings
        let old_json = r#"{
            "op_id": "op_legacy_empty",
            "op_type": "tool_call",
            "created_at": "2026-01-01T00:00:00Z"
        }"#;
        let parsed: PendingOperation = serde_json::from_str(old_json).unwrap();
        assert_eq!(parsed.detail.tool_name(), Some(""));
        assert_eq!(parsed.detail.args_summary(), Some(""));
    }

    #[test]
    fn test_pending_operation_new_detail_field_serde_roundtrip() {
        use crate::persistence::{
            PendingOperation, PendingOperationDetail, PendingOperationStatus, PendingOperationType,
        };
        // New format with detail field
        let op = PendingOperation {
            op_id: "op_new".into(),
            op_type: PendingOperationType::ToolCall,
            status: PendingOperationStatus::Running,
            detail: PendingOperationDetail::ToolCall {
                tool_name: "grep".into(),
                args_summary: "pattern".into(),
            },
            created_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&op).unwrap();
        let parsed: PendingOperation = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.op_id, "op_new");
        assert_eq!(parsed.detail.tool_name(), Some("grep"));
        assert_eq!(parsed.detail.args_summary(), Some("pattern"));
        // Verify detail field is serialized (not name/args)
        let json_value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(json_value.get("detail").is_some());
        assert!(json_value.get("name").is_none());
    }

    #[test]
    fn test_pending_operation_in_checkpoint_old_format_roundtrip() {
        use crate::persistence::PendingOperationStatus;
        // Old format checkpoint with name+args pending operation
        let cp = SessionCheckpoint::new("s-old-detail".into());
        let mut json_value: serde_json::Value = serde_json::to_value(&cp).unwrap();
        json_value["pending_operations"] = serde_json::json!([{
            "op_id": "op_old_ckpt",
            "op_type": "tool_call",
            "name": "old_tool",
            "args": "old_args",
            "created_at": "2026-01-01T00:00:00Z"
        }]);
        let json_str = serde_json::to_string(&json_value).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.pending_operations.len(), 1);
        assert_eq!(
            parsed.pending_operations[0].detail.tool_name(),
            Some("old_tool")
        );
        assert_eq!(
            parsed.pending_operations[0].detail.args_summary(),
            Some("old_args")
        );
        assert_eq!(
            parsed.pending_operations[0].status,
            PendingOperationStatus::Running
        );
    }

    #[test]
    fn test_pending_operation_detail_clone_and_debug() {
        use crate::persistence::PendingOperationDetail;
        let detail = PendingOperationDetail::ToolCall {
            tool_name: "test".into(),
            args_summary: "args".into(),
        };
        let cloned = detail.clone();
        assert_eq!(detail, cloned);
        // Debug should not panic
        let _ = format!("{:?}", detail);
    }

    // ===================================================================
    // pending_messages field tests (Step 1.3 + Step 1.4)
    // ===================================================================

    #[test]
    fn test_pending_messages_serializes_as_pending_messages_key() {
        use crate::llm_session::SessionMessage;
        use closeclaw_common::ContentBlock;

        let msg = SessionMessage {
            role: "system".into(),
            content_blocks: vec![ContentBlock::Text("hello".into())],
            timestamp: chrono::Utc::now(),
        };
        let mut cp = SessionCheckpoint::new("s-key".into());
        cp.pending_messages = vec![msg];
        let json_value = serde_json::to_value(&cp).unwrap();
        // Key should be "pending_messages", not "transcript"
        assert!(
            json_value.get("pending_messages").is_some(),
            "serialized JSON should have pending_messages key"
        );
        assert!(
            json_value.get("transcript").is_none(),
            "serialized JSON should NOT have transcript key"
        );
    }

    #[test]
    fn test_pending_messages_backward_compat_from_transcript_key() {
        use crate::llm_session::SessionMessage;
        use closeclaw_common::ContentBlock;

        let msg = SessionMessage {
            role: "system".into(),
            content_blocks: vec![ContentBlock::Text("old transcript".into())],
            timestamp: chrono::Utc::now(),
        };
        let mut cp = SessionCheckpoint::new("s-backcompat".into());
        cp.pending_messages = vec![msg.clone()];
        // Serialize, then replace key with old name "transcript"
        let mut json_value = serde_json::to_value(&cp).unwrap();
        let messages = json_value["pending_messages"].clone();
        json_value
            .as_object_mut()
            .unwrap()
            .remove("pending_messages");
        json_value
            .as_object_mut()
            .unwrap()
            .insert("transcript".into(), messages);
        let json_str = serde_json::to_string(&json_value).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.pending_messages.len(), 1);
        assert_eq!(
            parsed.pending_messages[0].content_blocks,
            msg.content_blocks
        );
    }

    #[test]
    fn test_pending_messages_empty_checkpoint_roundtrip() {
        let cp = SessionCheckpoint::new("s-empty-pm".into());
        assert!(cp.pending_messages.is_empty());
        let json = serde_json::to_string(&cp).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
        assert!(
            parsed.pending_messages.is_empty(),
            "empty pending_messages should survive roundtrip"
        );
    }

    #[test]
    fn test_pending_messages_nonempty_checkpoint_roundtrip() {
        use crate::llm_session::SessionMessage;
        use closeclaw_common::ContentBlock;

        let msgs = vec![
            SessionMessage {
                role: "user".into(),
                content_blocks: vec![ContentBlock::Text("msg1".into())],
                timestamp: chrono::Utc::now(),
            },
            SessionMessage {
                role: "assistant".into(),
                content_blocks: vec![ContentBlock::Text("msg2".into())],
                timestamp: chrono::Utc::now(),
            },
        ];
        let mut cp = SessionCheckpoint::new("s-nonempty-pm".into());
        cp.pending_messages = msgs;
        let json = serde_json::to_string(&cp).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.pending_messages.len(), 2);
        assert_eq!(parsed.pending_messages[0].role, "user");
        assert_eq!(parsed.pending_messages[1].role, "assistant");
    }

    #[test]
    fn test_pending_messages_missing_field_defaults_to_empty() {
        use crate::llm_session::SessionMessage;
        use closeclaw_common::ContentBlock;

        let mut cp = SessionCheckpoint::new("s-missing-pm".into());
        cp.pending_messages = vec![SessionMessage {
            role: "system".into(),
            content_blocks: vec![ContentBlock::Text("data".into())],
            timestamp: chrono::Utc::now(),
        }];
        let mut json_value = serde_json::to_value(&cp).unwrap();
        json_value
            .as_object_mut()
            .unwrap()
            .remove("pending_messages");
        let json_str = serde_json::to_string(&json_value).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json_str).unwrap();
        assert!(
            parsed.pending_messages.is_empty(),
            "missing pending_messages in old JSON should default to empty Vec"
        );
    }

    #[test]
    fn test_pending_messages_preserves_outbound_pending_separately() {
        use crate::llm_session::SessionMessage;
        use closeclaw_common::{ContentBlock, PendingMessage};

        let mut cp = SessionCheckpoint::new("s-separate".into());
        cp.pending_messages = vec![SessionMessage {
            role: "system".into(),
            content_blocks: vec![ContentBlock::Text("transcript".into())],
            timestamp: chrono::Utc::now(),
        }];
        cp.outbound_pending = vec![PendingMessage::new("out1".into(), "outbound msg".into())];
        let json = serde_json::to_string(&cp).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
        // pending_messages (transcript) and outbound_pending are independent
        assert_eq!(parsed.pending_messages.len(), 1);
        assert_eq!(parsed.outbound_pending.len(), 1);
        assert_eq!(parsed.outbound_pending[0].message_id, "out1");
    }

    // ===================================================================
    // DEFAULT_PURGE_AFTER_MINUTES tests (Step 1.2)
    // ===================================================================

    #[test]
    fn test_default_purge_after_minutes_is_zero() {
        // The constant should be 0 (never purge)
        assert_eq!(closeclaw_config::session::DEFAULT_PURGE_AFTER_MINUTES, 0);
    }

    #[test]
    fn test_hardcoded_config_returns_purge_after_minutes_zero() {
        use closeclaw_common::AgentRole;
        use closeclaw_config::session::JsonSessionConfigProvider;
        use closeclaw_config::SessionConfigProvider;
        // When no config file exists, hardcoded_config should return default
        // which has purge_after_minutes = 0
        let temp = tempfile::TempDir::new().unwrap();
        let nonexistent = temp.path().join("nonexistent.json");
        let provider = JsonSessionConfigProvider::new(&nonexistent).unwrap();
        for role in [AgentRole::MainAgent, AgentRole::SubAgent] {
            let cfg = provider.session_config_for("any-agent", role);
            assert_eq!(
                cfg.purge_after_minutes, 0,
                "hardcoded fallback for {:?} should have purge_after_minutes=0",
                role
            );
        }
    }

    #[test]
    fn test_per_agent_session_config_default_purge_zero() {
        use closeclaw_config::session::PerAgentSessionConfig;
        let cfg = PerAgentSessionConfig::default();
        assert_eq!(cfg.purge_after_minutes, 0);
        assert_eq!(cfg.idle_minutes, 30);
    }
}
