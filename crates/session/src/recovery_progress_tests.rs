// ProgressTool history fallback tests (split from recovery_tests.rs to stay under 1000 lines)

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;

    use crate::llm_session::SessionMessage;
    use crate::persistence::{PersistenceService, ProgressToolCallRecord};
    use crate::recovery::{
        parse_progress_call_record, rebuild_plan_state_from_calls,
        rebuild_progress_summary_from_calls, scan_progress_tool_calls, SessionRecoveryService,
        PROGRESS_HISTORY_APPEND_PREFIX,
    };
    use crate::storage::memory::MemoryStorage;
    use closeclaw_common::{ContentBlock, ExecutionStepStatus};

    fn create_test_checkpoint(session_id: &str) -> crate::persistence::SessionCheckpoint {
        use crate::persistence::*;
        use chrono::Utc;
        SessionCheckpoint {
            session_id: session_id.to_string(),
            last_message_id: Some("msg123".to_string()),
            mode_state: ReasoningModeState {
                current_step: 1,
                total_steps: 3,
                step_messages: vec!["Step 1".to_string()],
                is_complete: false,
            },
            pending_messages: Vec::new(),
            mode: ReasoningMode::Plan,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            ttl_seconds: 604800,
            status: SessionStatus::Active,
            last_message_at: None,
            message_count: 0,
            platform: None,
            peer_id: None,
            account_id: None,
            agent_id: None,
            role: None,
            reasoning_level: ReasoningLevel::default(),
            system_appends: Vec::new(),
            thread_id: None,
            sender_id: None,
            parent_session_id: None,
            depth: 0,
            effective_max_spawn_depth: None,
            mined: false,
            mined_at: None,
            dreaming_status: DreamingStatus::default(),
            pending_operations: Vec::new(),
            recovery_notification: None,
            pending_tool_failures: Vec::new(),
            verbosity_level: closeclaw_common::VerbosityLevel::default(),
            plan_state: None,
            progress_tool_calls: Vec::new(),
        }
    }

    fn make_tool_use_message(tool_name: &str, input_json: &str) -> SessionMessage {
        SessionMessage {
            role: "assistant".to_string(),
            content_blocks: vec![ContentBlock::ToolUse {
                id: "call_test".to_string(),
                name: tool_name.to_string(),
                input: input_json.to_string(),
            }],
            timestamp: Utc::now(),
        }
    }

    fn make_text_message(text: &str) -> SessionMessage {
        SessionMessage {
            role: "assistant".to_string(),
            content_blocks: vec![ContentBlock::Text(text.to_string())],
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn test_parse_progress_call_record() {
        // valid cases
        let r1 =
            parse_progress_call_record(r#"{"step_index":0,"status":"completed","summary":"done"}"#)
                .unwrap();
        assert_eq!(r1.step_index, 0);
        assert_eq!(r1.status, ExecutionStepStatus::Completed);
        assert_eq!(r1.summary.as_deref(), Some("done"));
        let r2 = parse_progress_call_record(
            r#"{"step_index":1,"status":"failed","error_message":"boom"}"#,
        )
        .unwrap();
        assert_eq!(r2.error_message.as_deref(), Some("boom"));
        // invalid cases
        assert!(parse_progress_call_record("not json").is_none());
        assert!(parse_progress_call_record("{}").is_none());
        assert!(parse_progress_call_record(r#"{"step_index":0}"#).is_none());
        assert!(parse_progress_call_record(r#"{"step_index":0,"status":"unknown"}"#).is_none());
    }

    #[test]
    fn test_scan_progress_tool_calls_basic() {
        // empty
        let records = scan_progress_tool_calls(&[]);
        assert!(records.is_empty());
        // non-progress tool calls
        let msgs2 = vec![
            make_tool_use_message("bash", r#"{"command":"ls"}"#),
            make_text_message("hello"),
        ];
        assert!(scan_progress_tool_calls(&msgs2).is_empty());
        // finds Progress calls
        let messages = vec![
            make_tool_use_message("Progress", r#"{"step_index":0,"status":"in_progress"}"#),
            make_tool_use_message(
                "Progress",
                r#"{"step_index":0,"status":"completed","summary":"done"}"#,
            ),
        ];
        let records = scan_progress_tool_calls(&messages);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].step_index, 0);
        assert_eq!(records[0].status, ExecutionStepStatus::InProgress);
        assert_eq!(records[1].status, ExecutionStepStatus::Completed);
        assert_eq!(records[1].summary.as_deref(), Some("done"));
    }

    #[test]
    fn test_rebuild_plan_state_single_step() {
        let calls = vec![
            ProgressToolCallRecord {
                step_index: 0,
                status: ExecutionStepStatus::InProgress,
                summary: None,
                error_message: None,
            },
            ProgressToolCallRecord {
                step_index: 0,
                status: ExecutionStepStatus::Completed,
                summary: Some("done".to_string()),
                error_message: None,
            },
        ];
        let ps = rebuild_plan_state_from_calls(&calls);
        assert_eq!(ps.execution_steps.len(), 1);
        assert_eq!(ps.execution_steps[0].status, ExecutionStepStatus::Completed);
        assert_eq!(ps.execution_steps[0].summary, "done");
        assert_eq!(ps.current_step, Some(0));
    }

    #[test]
    fn test_rebuild_plan_state_multi_step_and_invalid() {
        // Valid: in_progress -> completed -> in_progress -> failed
        let calls = vec![
            ProgressToolCallRecord {
                step_index: 0,
                status: ExecutionStepStatus::InProgress,
                summary: None,
                error_message: None,
            },
            ProgressToolCallRecord {
                step_index: 0,
                status: ExecutionStepStatus::Completed,
                summary: None,
                error_message: None,
            },
            ProgressToolCallRecord {
                step_index: 1,
                status: ExecutionStepStatus::InProgress,
                summary: None,
                error_message: None,
            },
            ProgressToolCallRecord {
                step_index: 1,
                status: ExecutionStepStatus::Failed,
                summary: None,
                error_message: Some("oops".to_string()),
            },
        ];
        let ps = rebuild_plan_state_from_calls(&calls);
        assert_eq!(ps.execution_steps.len(), 2);
        assert_eq!(ps.execution_steps[0].status, ExecutionStepStatus::Completed);
        assert_eq!(ps.execution_steps[1].status, ExecutionStepStatus::Failed);
        assert_eq!(ps.execution_steps[1].error_message.as_deref(), Some("oops"));
        assert_eq!(ps.current_step, Some(1));
        // Invalid: completed without in_progress → stays pending
        let invalid = vec![ProgressToolCallRecord {
            step_index: 0,
            status: ExecutionStepStatus::Completed,
            summary: None,
            error_message: None,
        }];
        let ps2 = rebuild_plan_state_from_calls(&invalid);
        assert_eq!(ps2.execution_steps[0].status, ExecutionStepStatus::Pending);
    }

    #[test]
    fn test_rebuild_progress_summary_from_calls() {
        let summary = rebuild_progress_summary_from_calls(&[]);
        assert!(summary.is_empty());
        let calls = vec![
            ProgressToolCallRecord {
                step_index: 0,
                status: ExecutionStepStatus::InProgress,
                summary: None,
                error_message: None,
            },
            ProgressToolCallRecord {
                step_index: 0,
                status: ExecutionStepStatus::Completed,
                summary: None,
                error_message: None,
            },
            ProgressToolCallRecord {
                step_index: 1,
                status: ExecutionStepStatus::InProgress,
                summary: None,
                error_message: None,
            },
        ];
        let s = rebuild_progress_summary_from_calls(&calls);
        assert!(s.contains("Step 1/2: completed"));
        assert!(s.contains("in_progress"));
    }

    #[tokio::test]
    async fn test_inject_progress_from_tool_calls() {
        let storage = Arc::new(MemoryStorage::new());
        // Normal case: layer 4 injects progress history
        let mut cp1 = create_test_checkpoint("progress-fallback");
        cp1.plan_state = None;
        cp1.progress_tool_calls = vec![
            ProgressToolCallRecord {
                step_index: 0,
                status: ExecutionStepStatus::InProgress,
                summary: None,
                error_message: None,
            },
            ProgressToolCallRecord {
                step_index: 0,
                status: ExecutionStepStatus::Completed,
                summary: None,
                error_message: None,
            },
            ProgressToolCallRecord {
                step_index: 1,
                status: ExecutionStepStatus::InProgress,
                summary: None,
                error_message: None,
            },
        ];
        storage.save_checkpoint(&cp1).await.unwrap();
        // Skipped when plan_state exists
        let mut cp2 = create_test_checkpoint("has-plan-state");
        cp2.plan_state = Some(closeclaw_common::PlanState::default());
        cp2.progress_tool_calls = vec![ProgressToolCallRecord {
            step_index: 0,
            status: ExecutionStepStatus::Completed,
            summary: None,
            error_message: None,
        }];
        storage.save_checkpoint(&cp2).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let _report = service.recover().await.unwrap();
        // progress-fallback: should have progress history injected
        let loaded1 = storage
            .load_checkpoint("progress-fallback")
            .await
            .unwrap()
            .unwrap();
        let pa = loaded1
            .system_appends
            .iter()
            .find(|s| s.starts_with(PROGRESS_HISTORY_APPEND_PREFIX));
        assert!(pa.is_some(), "progress history should be injected");
        assert!(pa.unwrap().contains("Step 1/2: completed"));
        // has-plan-state: should NOT have progress history (layer 1 available)
        let loaded2 = storage
            .load_checkpoint("has-plan-state")
            .await
            .unwrap()
            .unwrap();
        let pa2 = loaded2
            .system_appends
            .iter()
            .find(|s| s.starts_with(PROGRESS_HISTORY_APPEND_PREFIX));
        assert!(
            pa2.is_none(),
            "layer 4 should not inject when plan_state exists"
        );
    }
}
