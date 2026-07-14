#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use crate::persistence::{
        DreamingStatus, PendingOperation, PendingOperationStatus, PendingOperationType,
        PersistenceError, PersistenceService, ReasoningLevel, ReasoningMode, ReasoningModeState,
        SessionCheckpoint, SessionMode, SessionStatus,
    };
    use crate::recovery::{
        extract_tasks_from_content, RecoveryReport, SessionRecoveryService, SpawnTree,
        APPROVAL_HISTORY_PREFIX, PLAN_REFERENCES_PREFIX,
    };
    use crate::storage::memory::MemoryStorage;
    use chrono::Utc;
    fn create_test_checkpoint(session_id: &str) -> SessionCheckpoint {
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
            transcript: Vec::new(),
            label: None,
            communication_config: None,
            spawn_mode: None,
            snapshot_metas: Vec::new(),
        }
    }

    #[tokio::test]
    async fn test_recovery_report() {
        let full = RecoveryReport {
            recovered: vec!["s1".to_string(), "s2".to_string()],
            failed: Vec::new(),
            spawn_tree: SpawnTree::default(),
            dirty_sessions: Vec::new(),
        };
        assert!(full.is_full_success());
        assert_eq!(full.total(), 2);
        let partial = RecoveryReport {
            recovered: vec!["s1".to_string()],
            failed: vec!["s2".to_string()],
            spawn_tree: SpawnTree::default(),
            dirty_sessions: Vec::new(),
        };
        assert!(!partial.is_full_success());
        assert_eq!(partial.total(), 2);
    }

    #[tokio::test]
    async fn test_recovery_service_recover_empty() {
        let storage = Arc::new(MemoryStorage::new());
        let service = SessionRecoveryService::new(storage);

        let report = service.recover().await.unwrap();
        assert!(report.recovered.is_empty());
        assert!(report.failed.is_empty());
    }

    #[tokio::test]
    async fn test_recovery_service_recover_with_callback() {
        use chrono::Utc;
        let storage = Arc::new(MemoryStorage::new());
        let now = Utc::now();

        // Clean session
        storage
            .save_checkpoint(&create_test_checkpoint("session1"))
            .await
            .unwrap();
        // Dirty session with tool call
        let dirty = SessionCheckpoint::new("session2".into()).with_pending_operations(vec![
            PendingOperation {
                status: PendingOperationStatus::Running,
                op_id: "op_1".into(),
                op_type: PendingOperationType::ToolCall,
                name: "bash".into(),
                args: String::new(),
                created_at: now,
            },
        ]);
        storage.save_checkpoint(&dirty).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));

        // Capture callback parameters
        let restored = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_notification = Arc::new(std::sync::Mutex::new(Vec::<Option<String>>::new()));
        let captured_failures = Arc::new(std::sync::Mutex::new(Vec::<Vec<String>>::new()));
        let r = Arc::clone(&restored);
        let cn = Arc::clone(&captured_notification);
        let cf = Arc::clone(&captured_failures);

        service
            .set_restore_callback(
                move |session_id, _checkpoint, notification, tool_failures| {
                    r.lock().unwrap().push(session_id.to_string());
                    cn.lock().unwrap().push(notification.map(String::from));
                    cf.lock().unwrap().push(tool_failures.to_vec());
                    Ok(())
                },
            )
            .await;

        let report = service.recover().await.unwrap();
        assert_eq!(report.recovered.len(), 2);
        assert!(report.failed.is_empty());

        let mut restored_sessions = restored.lock().unwrap();
        restored_sessions.sort();
        assert_eq!(restored_sessions[0], "session1");
        assert_eq!(restored_sessions[1], "session2");

        // Dirty session callback should receive notification
        let notifs = captured_notification.lock().unwrap();
        let notif = notifs.iter().find(|n| n.is_some()).unwrap();
        assert!(notif.as_ref().unwrap().contains("网关已重启"));

        // Dirty session callback should receive tool failures
        let failures = captured_failures.lock().unwrap();
        let dirty_failures = failures.iter().find(|f| !f.is_empty()).unwrap();
        assert_eq!(dirty_failures.len(), 1);
        assert!(dirty_failures[0].contains("进程中断：网关重启"));
    }

    // -----------------------------------------------------------------
    // Spawn tree tests
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_recovery_spawn_tree_root_sessions() {
        let storage = Arc::new(MemoryStorage::new());
        storage
            .save_checkpoint(&create_test_checkpoint("root1"))
            .await
            .unwrap();
        storage
            .save_checkpoint(&create_test_checkpoint("root2"))
            .await
            .unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        assert_eq!(report.recovered.len(), 2);
        let tree = &report.spawn_tree;
        assert_eq!(tree.roots.len(), 2);
        assert!(tree.roots.contains(&"root1".to_string()));
        assert!(tree.roots.contains(&"root2".to_string()));
        assert!(tree.children.is_empty());
    }

    #[tokio::test]
    async fn test_recovery_spawn_tree_parent_child() {
        let storage = Arc::new(MemoryStorage::new());

        // Parent session
        let mut parent_cp = create_test_checkpoint("parent");
        parent_cp.parent_session_id = None;
        parent_cp.depth = 0;
        storage.save_checkpoint(&parent_cp).await.unwrap();

        // Child session
        let mut child_cp = create_test_checkpoint("child");
        child_cp.parent_session_id = Some("parent".to_string());
        child_cp.depth = 1;
        storage.save_checkpoint(&child_cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        assert_eq!(report.recovered.len(), 2);
        let tree = &report.spawn_tree;

        // Parent is root, child is registered under parent
        assert!(tree.is_root("parent"));
        assert!(!tree.is_root("child"));
        let children = tree.get_children("parent").unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0], "child");
    }

    #[tokio::test]
    async fn test_recovery_spawn_tree_orphan_demoted_to_root() {
        let storage = Arc::new(MemoryStorage::new());

        // Child session whose parent is NOT in storage (swept)
        let mut child_cp = create_test_checkpoint("orphan_child");
        child_cp.parent_session_id = Some("missing_parent".to_string());
        child_cp.depth = 2;
        storage.save_checkpoint(&child_cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        assert_eq!(report.recovered.len(), 1);
        let tree = &report.spawn_tree;

        // Orphan child is demoted to root
        assert!(tree.is_root("orphan_child"));
        assert!(tree.children.is_empty());
    }

    #[tokio::test]
    async fn test_recovery_spawn_tree_multi_level() {
        let storage = Arc::new(MemoryStorage::new());

        // root -> child1 -> grandchild
        let mut root_cp = create_test_checkpoint("root");
        root_cp.parent_session_id = None;
        root_cp.depth = 0;
        storage.save_checkpoint(&root_cp).await.unwrap();

        let mut child_cp = create_test_checkpoint("child1");
        child_cp.parent_session_id = Some("root".to_string());
        child_cp.depth = 1;
        storage.save_checkpoint(&child_cp).await.unwrap();

        let mut grandchild_cp = create_test_checkpoint("grandchild");
        grandchild_cp.parent_session_id = Some("child1".to_string());
        grandchild_cp.depth = 2;
        storage.save_checkpoint(&grandchild_cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        assert_eq!(report.recovered.len(), 3);
        let tree = &report.spawn_tree;

        assert!(tree.is_root("root"));
        assert!(!tree.is_root("child1"));
        assert!(!tree.is_root("grandchild"));

        let root_children = tree.get_children("root").unwrap();
        assert_eq!(root_children, &vec!["child1".to_string()]);

        let child1_children = tree.get_children("child1").unwrap();
        assert_eq!(child1_children, &vec!["grandchild".to_string()]);
    }

    #[test]
    fn test_spawn_tree_unit() {
        // is_root
        let tree = SpawnTree {
            roots: vec!["r1".to_string(), "r2".to_string()],
            children: HashMap::new(),
        };
        assert!(tree.is_root("r1"));
        assert!(tree.is_root("r2"));
        assert!(!tree.is_root("r3"));

        // get_children
        let mut children = HashMap::new();
        children.insert("p1".to_string(), vec!["c1".to_string(), "c2".to_string()]);
        let tree = SpawnTree {
            roots: vec![],
            children,
        };
        assert_eq!(tree.get_children("p1").unwrap().len(), 2);
        assert!(tree.get_children("p2").is_none());

        // root_ids
        let tree = SpawnTree {
            roots: vec!["a".to_string(), "b".to_string()],
            children: HashMap::new(),
        };
        assert_eq!(tree.root_ids(), &["a", "b"]);
    }

    #[test]
    fn test_build_spawn_tree_demoted_depth_reset() {
        // orphan child with depth=2 should be demoted to root with depth=0
        let mut checkpoints = HashMap::new();
        let mut orphan_cp = create_test_checkpoint("orphan");
        orphan_cp.parent_session_id = Some("missing_parent".to_string());
        orphan_cp.depth = 2;
        checkpoints.insert("orphan".to_string(), orphan_cp);

        let recovered = vec!["orphan".to_string()];
        let (tree, demoted) =
            SessionRecoveryService::<MemoryStorage>::build_spawn_tree(&mut checkpoints, &recovered);

        assert!(tree.is_root("orphan"));
        assert_eq!(checkpoints["orphan"].depth, 0);
        assert!(demoted.contains(&"orphan".to_string()));
    }

    #[test]
    fn test_build_spawn_tree_edge_cases() {
        // empty
        let (tree, demoted) =
            SessionRecoveryService::<MemoryStorage>::build_spawn_tree(&mut HashMap::new(), &[]);
        assert!(tree.roots.is_empty());
        assert!(tree.children.is_empty());
        assert!(demoted.is_empty());
        // partial recovery: parent recovered, child not
        let mut checkpoints = HashMap::new();
        let mut parent_cp = create_test_checkpoint("parent");
        parent_cp.parent_session_id = None;
        checkpoints.insert("parent".to_string(), parent_cp);
        let (tree2, demoted2) = SessionRecoveryService::<MemoryStorage>::build_spawn_tree(
            &mut checkpoints,
            &["parent".to_string()],
        );
        assert_eq!(tree2.roots, vec!["parent".to_string()]);
        assert!(tree2.children.is_empty());
        assert!(demoted2.is_empty());
    }

    #[tokio::test]
    async fn test_recovery_notifications_and_tool_failures() {
        let storage = Arc::new(MemoryStorage::new());
        let now = Utc::now();

        // Dirty session: tool call + sub-spawn
        let dirty = SessionCheckpoint::new("dirty_tools".into()).with_pending_operations(vec![
            PendingOperation {
                status: PendingOperationStatus::Running,
                op_id: "call_1".into(),
                op_type: PendingOperationType::ToolCall,
                name: "exec".into(),
                args: r#"{"command":"kubectl get pods"}"#.into(),
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
        storage.save_checkpoint(&dirty).await.unwrap();

        // Clean session: no pending ops
        let clean = SessionCheckpoint::new("clean_notif".into());
        storage.save_checkpoint(&clean).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        assert_eq!(report.recovered.len(), 2);
        assert!(report.dirty_sessions.contains(&"dirty_tools".to_string()));
        assert!(
            report.dirty_sessions.is_empty()
                || !report.dirty_sessions.contains(&"clean_notif".to_string())
        );

        // Dirty: notification stored, tool failures built
        let loaded = storage
            .load_checkpoint("dirty_tools")
            .await
            .unwrap()
            .unwrap();
        assert!(loaded.recovery_notification.is_some());
        let notif = loaded.recovery_notification.unwrap();
        assert!(notif.contains("网关已重启"));
        assert!(notif.contains("工具调用: exec"));
        assert_eq!(loaded.pending_tool_failures.len(), 1);
        assert!(loaded.pending_tool_failures[0].contains("exec"));

        // Clean: no notification
        let loaded = storage
            .load_checkpoint("clean_notif")
            .await
            .unwrap()
            .unwrap();
        assert!(loaded.recovery_notification.is_none());
    }

    // ── Step 1.3: archived session recovery tests ───────────────────

    /// Minimal mock storage for testing the "checkpoint not found" path.
    struct MockNotFoundStorage;

    #[async_trait::async_trait]
    impl PersistenceService for MockNotFoundStorage {
        async fn save_checkpoint(&self, _: &SessionCheckpoint) -> Result<(), PersistenceError> {
            Ok(())
        }
        async fn load_checkpoint(
            &self,
            _: &str,
        ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
            Ok(None)
        }
        async fn delete_checkpoint(&self, _: &str) -> Result<(), PersistenceError> {
            Ok(())
        }
        async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Ok(Vec::new())
        }
        async fn list_archived_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Ok(vec!["archived-not-found".to_string()])
        }
    }

    #[tokio::test]
    async fn test_recovery_scans_archived_sessions() {
        let storage = Arc::new(MemoryStorage::new());
        let now = Utc::now();

        // Create an archived checkpoint with pending operations
        let mut cp = create_test_checkpoint("archived-dirty");
        cp.status = SessionStatus::Active;
        cp.pending_operations = vec![PendingOperation {
            op_id: "op_archived".into(),
            op_type: PendingOperationType::ToolCall,
            status: PendingOperationStatus::Running,
            name: "exec".into(),
            args: r#"{"cmd":"echo hello"}"#.into(),
            created_at: now,
        }];
        // Save to active first (so load_checkpoint can find it), then archive,
        // then remove from active so it's only in the archived map
        storage.save_checkpoint(&cp).await.unwrap();
        storage.archive_checkpoint(&cp).await.unwrap();
        storage.remove_active("archived-dirty").await;

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        // Archived session with pending_operations should be recovered
        assert!(
            report.recovered.contains(&"archived-dirty".to_string()),
            "archived session with pending ops should be recovered"
        );

        // Should be marked as dirty
        assert!(
            report
                .dirty_sessions
                .contains(&"archived-dirty".to_string()),
            "restored archived session should be in dirty_sessions"
        );

        // Notification should have been stored in the checkpoint
        let loaded = storage
            .load_checkpoint("archived-dirty")
            .await
            .unwrap()
            .expect("checkpoint should exist after restore");
        assert!(
            loaded.recovery_notification.is_some(),
            "recovery_notification should be stored"
        );
        let notif = loaded.recovery_notification.unwrap();
        assert!(notif.contains("网关已重启"));
        assert!(notif.contains("工具调用: exec"));
    }

    #[tokio::test]
    async fn test_recovery_skips_clean_archived() {
        let storage = Arc::new(MemoryStorage::new());

        // Create an archived checkpoint with NO pending operations
        let cp = create_test_checkpoint("archived-clean");
        // Save to active first, archive, then remove from active
        // so it only exists in the archived map
        storage.save_checkpoint(&cp).await.unwrap();
        storage.archive_checkpoint(&cp).await.unwrap();
        storage.remove_active("archived-clean").await;

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        // Clean archived session should NOT be recovered
        assert!(
            !report.recovered.contains(&"archived-clean".to_string()),
            "clean archived session should be skipped"
        );
        assert!(
            !report
                .dirty_sessions
                .contains(&"archived-clean".to_string()),
            "clean archived session should not be dirty"
        );
    }

    #[tokio::test]
    async fn test_recovery_archived_not_found() {
        let storage = Arc::new(MockNotFoundStorage);
        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        // list_archived_sessions returns ["archived-not-found"] but
        // load_checkpoint returns None → checkpoint not found → failed
        assert!(
            report.failed.contains(&"archived-not-found".to_string()),
            "archived session with missing checkpoint should be in failed"
        );
        assert!(report.recovered.is_empty());
    }
    // ── Step 1.7: plan file content injection tests ─────────────────

    #[test]
    fn test_extract_tasks_from_content_chinese_heading() {
        let content = "# Plan\n\n## 来源\n\n| 字段 | 值 |\n|------|-----|\n| source_type | design-doc |\n\n## 开发步骤\n\n### Step 1.1：测试步骤\n\n**目标**：测试目标\n\n### Step 1.2：第二步\n\n**目标**：第二个目标\n\n## 进度\n\n| | Step | 状态 |\n|------|------|------|\n| ✅ | 1.1 | |\n";
        let result = extract_tasks_from_content(content).unwrap();
        assert!(result.contains("### Step 1.1"));
        assert!(result.contains("### Step 1.2"));
        assert!(!result.contains("## 进度"));
        assert!(!result.contains("## 来源"));
    }

    #[test]
    fn test_extract_tasks_from_content_english_heading() {
        let content = "# Plan\n\n## Tasks\n\n### Step 1\n\nDo something\n\n### Step 2\n\nDo another thing\n\n## Progress\n\nDone\n";
        let result = extract_tasks_from_content(content).unwrap();
        assert!(result.contains("### Step 1"));
        assert!(result.contains("### Step 2"));
        assert!(!result.contains("## Progress"));
    }

    #[test]
    fn test_extract_tasks_from_content_no_tasks_section() {
        let content = "# Plan\n\n## Source\n\nSomething\n";
        let result = extract_tasks_from_content(content);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_tasks_from_content_empty_tasks() {
        let content = "# Plan\n\n## 开发步骤\n\n## 进度\n";
        let result = extract_tasks_from_content(content);
        assert!(result.is_none(), "empty Tasks section should return None");
    }

    #[test]
    fn test_extract_tasks_from_content_tasks_at_end() {
        let content = "# Plan\n\n## 来源\n\nSomething\n\n## 开发步骤\n\n### Step 1\n\nDo stuff\n";
        let result = extract_tasks_from_content(content).unwrap();
        assert!(result.contains("### Step 1"));
        assert!(result.contains("Do stuff"));
    }

    #[tokio::test]
    async fn test_inject_approval_history_normal() {
        use crate::persistence::ApprovalToolCallRecord;
        let storage = Arc::new(MemoryStorage::new());

        let mut cp = create_test_checkpoint("approval-session");
        cp.approval_tool_calls = vec![ApprovalToolCallRecord {
            tool_name: "plan_approval".to_string(),
            plan_summary: "Implement feature X".to_string(),
            request_id: Some("req-001".to_string()),
            timestamp: None,
        }];
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        assert!(report.recovered.contains(&"approval-session".to_string()));

        let loaded = storage
            .load_checkpoint("approval-session")
            .await
            .unwrap()
            .unwrap();
        let approval_append = loaded
            .system_appends
            .iter()
            .find(|s| s.starts_with(APPROVAL_HISTORY_PREFIX));
        assert!(
            approval_append.is_some(),
            "approval history should be in system_appends"
        );
        let content = approval_append.unwrap();
        assert!(content.contains("plan_approval"));
        assert!(content.contains("Implement feature X"));
    }

    #[tokio::test]
    async fn test_inject_approval_history_empty() {
        let storage = Arc::new(MemoryStorage::new());

        let cp = create_test_checkpoint("no-approval");
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();

        assert!(report.recovered.contains(&"no-approval".to_string()));

        let loaded = storage
            .load_checkpoint("no-approval")
            .await
            .unwrap()
            .unwrap();
        assert!(loaded.system_appends.is_empty(), "no injection for empty");
    }

    #[tokio::test]
    async fn test_inject_approval_history_replaces_existing() {
        use crate::persistence::ApprovalToolCallRecord;
        let storage = Arc::new(MemoryStorage::new());

        let mut cp = create_test_checkpoint("replace-approval");
        cp.approval_tool_calls = vec![ApprovalToolCallRecord {
            tool_name: "plan_approval".to_string(),
            plan_summary: "New plan".to_string(),
            request_id: None,
            timestamp: None,
        }];
        cp.system_appends
            .push(format!("{}old data", APPROVAL_HISTORY_PREFIX));
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();
        assert!(report.recovered.contains(&"replace-approval".to_string()));

        let loaded = storage
            .load_checkpoint("replace-approval")
            .await
            .unwrap()
            .unwrap();
        let approvals: Vec<_> = loaded
            .system_appends
            .iter()
            .filter(|s| s.starts_with(APPROVAL_HISTORY_PREFIX))
            .collect();
        assert_eq!(approvals.len(), 1, "should have exactly one approval entry");
        assert!(approvals[0].contains("New plan"));
        assert!(!approvals[0].contains("old data"));
    }

    #[tokio::test]
    async fn test_inject_approval_history_preserves_other_appends() {
        use crate::persistence::ApprovalToolCallRecord;
        let storage = Arc::new(MemoryStorage::new());

        let mut cp = create_test_checkpoint("preserve-approval");
        cp.approval_tool_calls = vec![ApprovalToolCallRecord {
            tool_name: "plan_approval".to_string(),
            plan_summary: "My plan".to_string(),
            request_id: None,
            timestamp: None,
        }];
        cp.system_appends.push("other content".to_string());
        storage.save_checkpoint(&cp).await.unwrap();

        let service = SessionRecoveryService::new(Arc::clone(&storage));
        let report = service.recover().await.unwrap();
        assert!(report.recovered.contains(&"preserve-approval".to_string()));

        let loaded = storage
            .load_checkpoint("preserve-approval")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.system_appends.len(), 2);
        assert!(loaded.system_appends.contains(&"other content".to_string()));
        let approval = loaded
            .system_appends
            .iter()
            .find(|s| s.starts_with(APPROVAL_HISTORY_PREFIX));
        assert!(approval.is_some());
        assert!(approval.unwrap().contains("My plan"));
    }

    // ── Step 1.8: ProgressTool call history fallback (layer 4) tests ──

    use crate::llm_session::SessionMessage;
    use crate::persistence::ProgressToolCallRecord;
    use crate::recovery::{
        parse_progress_call_record, rebuild_plan_state_from_calls,
        rebuild_progress_summary_from_calls, scan_progress_tool_calls,
    };
    use closeclaw_common::{ContentBlock, ExecutionStepStatus};

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
            "Plan: implement user auth flow".to_string(),
            "File: crates/auth/src/lib.rs".to_string(),
        ];
        storage.save_checkpoint(&cp1).await.unwrap();
        // Skipped when plan_state exists
        let mut cp2 = create_test_checkpoint("has-plan-state");
        cp2.plan_state = Some(closeclaw_common::PlanState::default());
        cp2.plan_references = vec!["ignored ref".to_string()];
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
        assert!(pa.unwrap().contains("implement user auth flow"));
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

    /// Verify that communication_config is preserved through
    /// checkpoint save/load cycle (serde roundtrip).
    #[tokio::test]
    async fn test_checkpoint_communication_config_persisted_through_save_load() {
        let storage = Arc::new(MemoryStorage::new());
        let mut cp = create_test_checkpoint("comm-persist");
        cp.communication_config = Some(
            closeclaw_common::communication::CommunicationConfig::default_with_parent(Some(
                "parent-agent-1",
            )),
        );

        storage.save_checkpoint(&cp).await.unwrap();
        let loaded = storage
            .load_checkpoint("comm-persist")
            .await
            .unwrap()
            .unwrap();
        let comm = loaded
            .communication_config
            .as_ref()
            .expect("communication_config should survive save/load");
        assert_eq!(comm.outbound, vec!["parent-agent-1".to_string()]);
        assert_eq!(comm.inbound, vec!["parent-agent-1".to_string()]);
    }

    /// Verify that old checkpoints without communication_config
    /// deserialize correctly (backward compatibility).
    #[test]
    fn test_old_checkpoint_without_communication_config_deserializes() {
        let cp = create_test_checkpoint("old-cp");
        let mut json_val = serde_json::to_value(&cp).unwrap();
        json_val
            .as_object_mut()
            .unwrap()
            .remove("communication_config");
        let json_str = serde_json::to_string(&json_val).unwrap();
        let parsed: SessionCheckpoint = serde_json::from_str(&json_str).unwrap();
        assert!(
            parsed.communication_config.is_none(),
            "old checkpoint without communication_config should deserialize to None"
        );
    }
}
