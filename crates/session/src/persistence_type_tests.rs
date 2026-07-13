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
        use crate::persistence::{PendingOperation, PendingOperationStatus, PendingOperationType};
        let op = PendingOperation {
            op_id: "op1".into(),
            op_type: PendingOperationType::ToolCall,
            status: PendingOperationStatus::Running,
            name: "test".into(),
            args: String::new(),
            created_at: chrono::Utc::now(),
        };
        assert_eq!(op.status, PendingOperationStatus::Running);
    }

    #[test]
    fn test_pending_operation_serde_roundtrip_with_status() {
        use crate::persistence::{PendingOperation, PendingOperationStatus, PendingOperationType};
        let op = PendingOperation {
            op_id: "op1".into(),
            op_type: PendingOperationType::ToolCall,
            status: PendingOperationStatus::Running,
            name: "test".into(),
            args: String::new(),
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
}
