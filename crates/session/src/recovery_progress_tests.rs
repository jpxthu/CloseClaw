// ProgressTool history fallback tests (split from recovery_tests.rs to stay under 1000 lines)

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;

    use crate::llm_session::SessionMessage;
    use crate::llm_session::PROGRESS_APPEND_PREFIX;
    use crate::persistence::{PersistenceService, ProgressToolCallRecord};
    use crate::recovery::{
        parse_progress_call_record, rebuild_plan_state_from_calls,
        rebuild_progress_summary_from_calls, scan_progress_tool_calls, SessionRecoveryService,
        APPROVAL_HISTORY_PREFIX, PLAN_REFERENCES_PREFIX,
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
            outbound_pending: Vec::new(),
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
            approval_tool_calls: Vec::new(),
            plan_references: Vec::new(),
            session_mode: SessionMode::default(),
            pending_messages: Vec::new(),
            label: None,
            communication_config: None,
            spawn_mode: None,
            snapshot_metas: Vec::new(),
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
    async fn test_inject_plan_references_normal() {
        let storage = Arc::new(MemoryStorage::new());
        // Normal case: layer 4 injects plan references
        let mut cp1 = create_test_checkpoint("plan-refs-fallback");
        cp1.plan_state = None;
        cp1.plan_references = vec![
            "Plan: implement user auth".to_string(),
            "File: src/auth.rs".to_string(),
        ];
        storage.save_checkpoint(&cp1).await.unwrap();
        // Skipped when plan_state exists
        let mut cp2 = create_test_checkpoint("has-plan-state");
        cp2.plan_state = Some(closeclaw_common::PlanState::default());
        cp2.plan_references = vec!["ignored".to_string()];
        storage.save_checkpoint(&cp2).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let _report = service.recover().await.unwrap();

        // plan-refs-fallback: should have plan references injected
        let loaded1 = storage
            .load_checkpoint("plan-refs-fallback")
            .await
            .unwrap()
            .unwrap();
        let pa = loaded1
            .system_appends
            .iter()
            .find(|s| s.starts_with(PLAN_REFERENCES_PREFIX));
        assert!(pa.is_some(), "plan references should be injected");
        assert!(pa.unwrap().contains("implement user auth"));
        // has-plan-state: should NOT have plan references (layer 1 available)
        let loaded2 = storage
            .load_checkpoint("has-plan-state")
            .await
            .unwrap()
            .unwrap();
        let pa2 = loaded2
            .system_appends
            .iter()
            .find(|s| s.starts_with(PLAN_REFERENCES_PREFIX));
        assert!(
            pa2.is_none(),
            "layer 4 should not inject when plan_state exists"
        );
    }

    #[tokio::test]
    async fn test_layer2_check_skips_layer4_when_progress_summary_exists() {
        // Normal path: system_appends has PROGRESS_APPEND_PREFIX →
        // layer 4 should NOT trigger
        let storage = Arc::new(MemoryStorage::new());
        let mut cp = create_test_checkpoint("has-progress-summary");
        cp.plan_state = None;
        cp.system_appends
            .push(format!("{}Step 1 done", PROGRESS_APPEND_PREFIX));
        cp.plan_references = vec!["some ref".to_string()];
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let _report = service.recover().await.unwrap();

        let loaded = storage
            .load_checkpoint("has-progress-summary")
            .await
            .unwrap()
            .unwrap();
        let history = loaded
            .system_appends
            .iter()
            .find(|s| s.starts_with(PLAN_REFERENCES_PREFIX));
        assert!(
            history.is_none(),
            "layer 4 should NOT inject when layer 2 progress summary exists"
        );
        // The original layer 2 entry must remain untouched
        let progress = loaded
            .system_appends
            .iter()
            .find(|s| s.starts_with(PROGRESS_APPEND_PREFIX));
        assert!(
            progress.is_some(),
            "layer 2 progress summary should still be present"
        );
    }

    #[tokio::test]
    async fn test_layer4_triggers_when_no_progress_summary() {
        // Reverse verification: no PROGRESS_APPEND_PREFIX and other layers
        // unavailable → layer 4 should trigger
        let storage = Arc::new(MemoryStorage::new());
        let mut cp = create_test_checkpoint("no-progress-summary");
        cp.plan_state = None;
        // system_appends is empty — no layer 2 or layer 3 entries
        cp.plan_references = vec![
            "Plan: implement feature".to_string(),
            "File: src/lib.rs".to_string(),
        ];
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let _report = service.recover().await.unwrap();

        let loaded = storage
            .load_checkpoint("no-progress-summary")
            .await
            .unwrap()
            .unwrap();
        let history = loaded
            .system_appends
            .iter()
            .find(|s| s.starts_with(PLAN_REFERENCES_PREFIX));
        assert!(
            history.is_some(),
            "layer 4 should trigger when no layer 2 progress summary exists"
        );
        assert!(
            history.unwrap().contains("implement feature"),
            "layer 4 should inject plan references"
        );
    }

    #[tokio::test]
    async fn test_layer3_hits_before_layer2_boundary() {
        // Boundary: both PROGRESS_APPEND_PREFIX and APPROVAL_HISTORY_PREFIX
        // present → layer 3 check hits first (returns early before layer 2)
        let storage = Arc::new(MemoryStorage::new());
        let mut cp = create_test_checkpoint("both-layers-2-and-3");
        cp.plan_state = None;
        cp.system_appends
            .push(format!("{}Step 1 done", PROGRESS_APPEND_PREFIX));
        cp.system_appends
            .push(format!("{}approval data", APPROVAL_HISTORY_PREFIX));
        cp.plan_references = vec!["ref 1".to_string()];
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let _report = service.recover().await.unwrap();

        let loaded = storage
            .load_checkpoint("both-layers-2-and-3")
            .await
            .unwrap()
            .unwrap();
        // Layer 4 should not inject — either layer 3 or layer 2 short-circuits
        let history = loaded
            .system_appends
            .iter()
            .find(|s| s.starts_with(PLAN_REFERENCES_PREFIX));
        assert!(
            history.is_none(),
            "layer 4 should NOT inject when both layer 2 and layer 3 exist"
        );
        // Both original entries must remain untouched
        assert!(
            loaded
                .system_appends
                .iter()
                .any(|s| s.starts_with(PROGRESS_APPEND_PREFIX)),
            "layer 2 entry should remain"
        );
        assert!(
            loaded
                .system_appends
                .iter()
                .any(|s| s.starts_with(APPROVAL_HISTORY_PREFIX)),
            "layer 3 entry should remain"
        );
    }

    #[tokio::test]
    async fn test_state_transition_layer2_absent_to_present() {
        // State transition: from having layer 2 data (no layer 4) to not
        // having it (layer 4 triggers)
        let storage = Arc::new(MemoryStorage::new());

        // Checkpoint A: has layer 2 progress summary → layer 4 skipped
        let mut cp_a = create_test_checkpoint("transition-with-layer2");
        cp_a.plan_state = None;
        cp_a.system_appends
            .push(format!("{}Step 0 done", PROGRESS_APPEND_PREFIX));
        cp_a.plan_references = vec!["ref a".to_string()];
        storage.save_checkpoint(&cp_a).await.unwrap();

        // Checkpoint B: no layer 2 progress summary → layer 4 triggers
        let mut cp_b = create_test_checkpoint("transition-without-layer2");
        cp_b.plan_state = None;
        cp_b.plan_references = vec![
            "Plan: feature X".to_string(),
            "File: src/main.rs".to_string(),
        ];
        storage.save_checkpoint(&cp_b).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let _report = service.recover().await.unwrap();

        // Checkpoint A: layer 4 should NOT have injected
        let loaded_a = storage
            .load_checkpoint("transition-with-layer2")
            .await
            .unwrap()
            .unwrap();
        assert!(
            loaded_a
                .system_appends
                .iter()
                .find(|s| s.starts_with(PLAN_REFERENCES_PREFIX))
                .is_none(),
            "layer 4 should NOT inject when layer 2 exists"
        );

        // Checkpoint B: layer 4 SHOULD have injected
        let loaded_b = storage
            .load_checkpoint("transition-without-layer2")
            .await
            .unwrap()
            .unwrap();
        assert!(
            loaded_b
                .system_appends
                .iter()
                .find(|s| s.starts_with(PLAN_REFERENCES_PREFIX))
                .is_some(),
            "layer 4 SHOULD inject when layer 2 is absent"
        );
    }

    // ── Plan Tasks section injection tests (Step 1.3 / Gap 3) ──────

    use crate::recovery::PLAN_TASKS_PREFIX;
    use closeclaw_common::{PlanPhase, PlanState};

    /// Helper: create a temp plan file with a Tasks section.
    fn create_plan_file_with_tasks(dir: &std::path::Path, tasks_content: &str) -> String {
        let plan_file = dir.join("test-plan.md");
        let content = format!(
            "# Plan\n\n| 字段 | 值 |\n| 状态 | executing |\n\n## 开发步骤\n\n{}\n\n## 进度\n",
            tasks_content
        );
        std::fs::write(&plan_file, content).unwrap();
        plan_file.to_string_lossy().to_string()
    }

    /// Helper: create a checkpoint with plan_state.
    fn checkpoint_with_plan(
        session_id: &str,
        plan_file_path: &str,
    ) -> crate::persistence::SessionCheckpoint {
        let mut cp = crate::persistence::SessionCheckpoint::new(session_id.into());
        cp.plan_state = Some(PlanState {
            phase: PlanPhase::FinalPlan,
            plan_file_path: plan_file_path.to_string(),
            ..PlanState::new()
        });
        cp
    }

    #[tokio::test]
    async fn test_inject_plan_tasks_executing_session() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = create_plan_file_with_tasks(dir.path(), "### Step 1.1\nDo stuff");

        let storage = Arc::new(MemoryStorage::new());
        let cp = checkpoint_with_plan("exec-session", &plan_path);
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();
        assert!(report.recovered.contains(&"exec-session".to_string()));

        let loaded = storage
            .load_checkpoint("exec-session")
            .await
            .unwrap()
            .unwrap();
        let tasks_append = loaded
            .system_appends
            .iter()
            .find(|s| s.starts_with(PLAN_TASKS_PREFIX));
        assert!(
            tasks_append.is_some(),
            "plan tasks should be injected for Executing session"
        );
        assert!(tasks_append.unwrap().contains("Step 1.1"));
    }

    #[tokio::test]
    async fn test_inject_plan_tasks_paused_session() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = create_plan_file_with_tasks(dir.path(), "### Step 2.1\nPaused work");

        let storage = Arc::new(MemoryStorage::new());
        let cp = checkpoint_with_plan("paused-session", &plan_path);
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();
        assert!(report.recovered.contains(&"paused-session".to_string()));

        let loaded = storage
            .load_checkpoint("paused-session")
            .await
            .unwrap()
            .unwrap();
        let tasks_append = loaded
            .system_appends
            .iter()
            .find(|s| s.starts_with(PLAN_TASKS_PREFIX));
        assert!(
            tasks_append.is_some(),
            "plan tasks should be injected for Paused session"
        );
        assert!(tasks_append.unwrap().contains("Step 2.1"));
    }

    #[tokio::test]
    async fn test_inject_plan_tasks_no_plan_state() {
        let storage = Arc::new(MemoryStorage::new());
        let cp = crate::persistence::SessionCheckpoint::new("no-plan".into());
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();
        assert!(report.recovered.contains(&"no-plan".to_string()));

        let loaded = storage.load_checkpoint("no-plan").await.unwrap().unwrap();
        assert!(
            loaded.system_appends.is_empty(),
            "no injection when plan_state is absent"
        );
    }

    #[tokio::test]
    async fn test_inject_plan_tasks_file_not_found() {
        let storage = Arc::new(MemoryStorage::new());
        let cp = checkpoint_with_plan("missing-file", "/nonexistent/path/plan.md");
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();
        assert!(report.recovered.contains(&"missing-file".to_string()));

        let loaded = storage
            .load_checkpoint("missing-file")
            .await
            .unwrap()
            .unwrap();
        assert!(
            loaded.system_appends.is_empty(),
            "graceful skip when plan file not found"
        );
    }

    #[tokio::test]
    async fn test_inject_plan_tasks_empty_section() {
        let dir = tempfile::tempdir().unwrap();
        let plan_file = dir.path().join("empty-tasks.md");
        let content = "# Plan\n\n## 开发步骤\n\n## 进度\n";
        std::fs::write(&plan_file, content).unwrap();
        let plan_path = plan_file.to_string_lossy().to_string();

        let storage = Arc::new(MemoryStorage::new());
        let cp = checkpoint_with_plan("empty-tasks", &plan_path);
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();
        assert!(report.recovered.contains(&"empty-tasks".to_string()));

        let loaded = storage
            .load_checkpoint("empty-tasks")
            .await
            .unwrap()
            .unwrap();
        assert!(
            loaded.system_appends.is_empty(),
            "no injection when Tasks section is empty"
        );
    }

    #[tokio::test]
    async fn test_inject_plan_tasks_replaces_existing() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = create_plan_file_with_tasks(dir.path(), "### New Step\nUpdated");

        let storage = Arc::new(MemoryStorage::new());
        let mut cp = checkpoint_with_plan("replace-tasks", &plan_path);
        cp.system_appends
            .push(format!("{}old tasks data", PLAN_TASKS_PREFIX));
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();
        assert!(report.recovered.contains(&"replace-tasks".to_string()));

        let loaded = storage
            .load_checkpoint("replace-tasks")
            .await
            .unwrap()
            .unwrap();
        let tasks: Vec<_> = loaded
            .system_appends
            .iter()
            .filter(|s| s.starts_with(PLAN_TASKS_PREFIX))
            .collect();
        assert_eq!(tasks.len(), 1, "should have exactly one tasks entry");
        assert!(tasks[0].contains("New Step"));
        assert!(!tasks[0].contains("old tasks data"));
    }

    #[tokio::test]
    async fn test_inject_plan_tasks_preserves_other_appends() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = create_plan_file_with_tasks(dir.path(), "### Step\nContent");

        let storage = Arc::new(MemoryStorage::new());
        let mut cp = checkpoint_with_plan("preserve-tasks", &plan_path);
        cp.system_appends.push("other content".to_string());
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();
        assert!(report.recovered.contains(&"preserve-tasks".to_string()));

        let loaded = storage
            .load_checkpoint("preserve-tasks")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.system_appends.len(), 2);
        assert!(loaded.system_appends.contains(&"other content".to_string()));
        let tasks = loaded
            .system_appends
            .iter()
            .find(|s| s.starts_with(PLAN_TASKS_PREFIX));
        assert!(tasks.is_some());
        assert!(tasks.unwrap().contains("Step"));
    }
}
