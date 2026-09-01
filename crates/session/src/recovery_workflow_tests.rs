#[cfg(test)]
mod tests {
    use crate::persistence::{
        DreamingStatus, ReasoningLevel, ReasoningMode, ReasoningModeState, SessionCheckpoint,
        SessionMode, SessionStatus,
    };
    use crate::workflow_recovery::{inject_workflow_recovery, WORKFLOW_RECOVERY_PREFIX};
    use closeclaw_workflow::run::{Phase, StepHistoryEntry, WorkflowRun};

    fn make_workflow_run(current_step: usize, phase: Phase) -> WorkflowRun {
        WorkflowRun {
            workflow_id: "test-wf".to_string(),
            definition_name: "test-wf".to_string(),
            definition_version: "0.1".to_string(),
            current_step,
            phase,
            step_history: vec![StepHistoryEntry {
                step_id: 0,
                step_name: "Step Zero".to_string(),
                completed_at: "2026-01-01T00:00:00Z".to_string(),
            }],
            step_data: Default::default(),
            pending_verify: 0,
        }
    }

    fn make_test_checkpoint(session_id: &str) -> SessionCheckpoint {
        SessionCheckpoint {
            session_id: session_id.to_string(),
            last_message_id: None,
            mode_state: ReasoningModeState::default(),
            outbound_pending: Vec::new(),
            reasoning_mode: ReasoningMode::Direct,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            ttl_seconds: 604800,
            status: SessionStatus::Active,
            last_message_at: None,
            last_user_activity_at: None,
            message_count: 0,
            platform: None,
            peer_id: None,
            account_id: None,
            agent_id: None,
            role: None,
            reasoning_level: ReasoningLevel::default(),
            system_appends: Vec::new(),
            thread_id: None,
            reply_ref: None,
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
            snapshot_metas: Vec::new(),
            workflow_run: None,
        }
    }

    #[tokio::test]
    async fn test_inject_notification_for_executing_phase() {
        let mut cp = make_test_checkpoint("wf-1");
        cp.workflow_run = Some(make_workflow_run(1, Phase::Executing));

        inject_workflow_recovery("wf-1", &mut cp).await;

        let notif = cp
            .system_appends
            .iter()
            .find(|s| s.starts_with(WORKFLOW_RECOVERY_PREFIX));
        assert!(notif.is_some(), "recovery notification not found");
        let notif = notif.unwrap();
        assert!(notif.contains("test-wf"), "got: {}", notif);
        assert!(notif.contains("Step 1"), "got: {}", notif);
        assert!(notif.contains("Step Zero"), "got: {}", notif);
    }

    #[tokio::test]
    async fn test_skip_complete_phase() {
        let mut cp = make_test_checkpoint("wf-2");
        cp.workflow_run = Some(make_workflow_run(1, Phase::Complete));

        inject_workflow_recovery("wf-2", &mut cp).await;

        let has_recovery = cp
            .system_appends
            .iter()
            .any(|s| s.starts_with(WORKFLOW_RECOVERY_PREFIX));
        assert!(!has_recovery, "should skip completed workflow");
    }

    #[tokio::test]
    async fn test_verifying_phase() {
        let mut cp = make_test_checkpoint("wf-3");
        let mut run = make_workflow_run(0, Phase::Verifying);
        run.pending_verify = 2;
        cp.workflow_run = Some(run);

        inject_workflow_recovery("wf-3", &mut cp).await;

        let notif = cp
            .system_appends
            .iter()
            .find(|s| s.starts_with(WORKFLOW_RECOVERY_PREFIX))
            .unwrap();
        assert!(notif.contains("Step 0"), "got: {}", notif);
    }

    #[tokio::test]
    async fn test_blocked_phase() {
        let mut cp = make_test_checkpoint("wf-4");
        cp.workflow_run = Some(make_workflow_run(0, Phase::Blocked));

        inject_workflow_recovery("wf-4", &mut cp).await;

        let notif = cp
            .system_appends
            .iter()
            .find(|s| s.starts_with(WORKFLOW_RECOVERY_PREFIX))
            .unwrap();
        assert!(notif.contains("test-wf"), "got: {}", notif);
    }

    #[tokio::test]
    async fn test_preserves_other_appends() {
        let mut cp = make_test_checkpoint("wf-5");
        cp.workflow_run = Some(make_workflow_run(0, Phase::Executing));
        cp.system_appends.push("existing-append".to_string());

        inject_workflow_recovery("wf-5", &mut cp).await;

        assert!(
            cp.system_appends.iter().any(|s| s == "existing-append"),
            "existing append should be preserved"
        );
    }

    #[tokio::test]
    async fn test_no_workflow_run() {
        let mut cp = make_test_checkpoint("wf-6");
        // No workflow_run set

        inject_workflow_recovery("wf-6", &mut cp).await;

        let has_recovery = cp
            .system_appends
            .iter()
            .any(|s| s.starts_with(WORKFLOW_RECOVERY_PREFIX));
        assert!(!has_recovery, "should not inject without workflow_run");
    }

    #[tokio::test]
    async fn test_replaces_existing_notification() {
        let mut cp = make_test_checkpoint("wf-7");
        cp.workflow_run = Some(make_workflow_run(0, Phase::Executing));
        cp.system_appends
            .push(format!("{}old notification", WORKFLOW_RECOVERY_PREFIX));

        inject_workflow_recovery("wf-7", &mut cp).await;

        let notif_count = cp
            .system_appends
            .iter()
            .filter(|s| s.starts_with(WORKFLOW_RECOVERY_PREFIX))
            .count();
        assert_eq!(notif_count, 1, "should have exactly one notification");

        let notif = cp
            .system_appends
            .iter()
            .find(|s| s.starts_with(WORKFLOW_RECOVERY_PREFIX))
            .unwrap();
        assert!(notif.contains("test-wf"), "got: {}", notif);
    }

    #[tokio::test]
    async fn test_empty_step_history_fallback() {
        let mut cp = make_test_checkpoint("wf-8");
        let mut run = make_workflow_run(0, Phase::Executing);
        run.step_history.clear();
        cp.workflow_run = Some(run);

        inject_workflow_recovery("wf-8", &mut cp).await;

        let notif = cp
            .system_appends
            .iter()
            .find(|s| s.starts_with(WORKFLOW_RECOVERY_PREFIX))
            .unwrap();
        assert!(notif.contains("unknown"), "got: {}", notif);
    }

    // ── cleanup_workflow_exit tests ─────────────────────────────────────

    use crate::workflow_recovery::cleanup_workflow_exit;
    use closeclaw_workflow::context_append::build_workflow_context_append;
    use closeclaw_workflow::definition::{Step, Workflow};

    fn make_test_workflow_def() -> Workflow {
        Workflow {
            id: "test-wf".to_string(),
            name: "Test Workflow".to_string(),
            description: "A test workflow".to_string(),
            version: Some("0.1".to_string()),
            allow_blocked: false,
            verify_retry_limit: 3,
            step_data_schema: serde_yaml::Value::Null,
            steps: vec![Step {
                id: 0,
                name: "Step Zero".to_string(),
                allow_blocked: None,
                goal: "Do the first thing".to_string(),
                verify: vec![],
                jump: vec![],
                transitions: vec![],
            }],
        }
    }

    #[test]
    fn test_cleanup_removes_workflow_context() {
        let mut cp = make_test_checkpoint("wf-c1");
        cp.workflow_run = Some(make_workflow_run(0, Phase::Complete));
        cp.system_appends
            .push(build_workflow_context_append(&make_test_workflow_def()));

        let report = cleanup_workflow_exit(&mut cp);

        assert_eq!(report.removed_contexts, 1);
        assert!(cp
            .system_appends
            .iter()
            .all(|s| !s.starts_with("--- WORKFLOW ---")));
    }

    #[test]
    fn test_cleanup_removes_recovery_notification() {
        let mut cp = make_test_checkpoint("wf-c2");
        cp.workflow_run = Some(make_workflow_run(1, Phase::Executing));
        cp.system_appends
            .push(format!("{}recovery msg", WORKFLOW_RECOVERY_PREFIX));

        let report = cleanup_workflow_exit(&mut cp);

        assert_eq!(report.removed_recovery_notifications, 1);
        assert!(cp
            .system_appends
            .iter()
            .all(|s| !s.starts_with(WORKFLOW_RECOVERY_PREFIX)));
    }

    #[test]
    fn test_cleanup_clears_workflow_run() {
        let mut cp = make_test_checkpoint("wf-c3");
        cp.workflow_run = Some(make_workflow_run(0, Phase::Complete));

        let report = cleanup_workflow_exit(&mut cp);

        assert!(report.had_workflow_run);
        assert!(cp.workflow_run.is_none());
    }

    #[test]
    fn test_cleanup_no_workflow_run() {
        let mut cp = make_test_checkpoint("wf-c4");
        // No workflow_run set.

        let report = cleanup_workflow_exit(&mut cp);

        assert!(!report.had_workflow_run);
        assert!(cp.workflow_run.is_none());
    }

    #[test]
    fn test_cleanup_preserves_non_workflow_appends() {
        let mut cp = make_test_checkpoint("wf-c5");
        cp.workflow_run = Some(make_workflow_run(0, Phase::Complete));
        cp.system_appends
            .push(build_workflow_context_append(&make_test_workflow_def()));
        cp.system_appends
            .push(format!("{}recovery", WORKFLOW_RECOVERY_PREFIX));
        cp.system_appends.push("user-managed-append".to_string());
        cp.system_appends.push("another-user-append".to_string());

        let report = cleanup_workflow_exit(&mut cp);

        assert_eq!(report.removed_contexts, 1);
        assert_eq!(report.removed_recovery_notifications, 1);
        assert_eq!(cp.system_appends.len(), 2);
        assert!(cp
            .system_appends
            .contains(&"user-managed-append".to_string()));
        assert!(cp
            .system_appends
            .contains(&"another-user-append".to_string()));
    }

    #[test]
    fn test_cleanup_full_lifecycle() {
        // Simulate a complete workflow lifecycle: inject → cleanup.
        let mut cp = make_test_checkpoint("wf-c6");
        cp.workflow_run = Some(make_workflow_run(1, Phase::Executing));
        cp.system_appends
            .push(build_workflow_context_append(&make_test_workflow_def()));
        cp.system_appends.push("user-append".to_string());

        // First inject recovery state (as would happen on resume).
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(inject_workflow_recovery("wf-c6", &mut cp));

        // Verify injection happened.
        assert!(cp
            .system_appends
            .iter()
            .any(|s| s.starts_with(WORKFLOW_RECOVERY_PREFIX)));
        assert!(cp.workflow_run.is_some());

        // Now cleanup.
        let report = cleanup_workflow_exit(&mut cp);

        assert!(report.removed_contexts >= 1);
        assert!(report.removed_recovery_notifications >= 1);
        assert!(report.had_workflow_run);
        assert!(cp.workflow_run.is_none());
        assert!(cp
            .system_appends
            .iter()
            .all(|s| !s.starts_with("--- WORKFLOW ---")));
        assert!(cp
            .system_appends
            .iter()
            .all(|s| !s.starts_with(WORKFLOW_RECOVERY_PREFIX)));
        assert!(cp.system_appends.contains(&"user-append".to_string()));
    }
}
