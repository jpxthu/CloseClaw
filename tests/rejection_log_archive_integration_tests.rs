//! Integration Tests for Rejection Logging and Plan Archive
//!
//! Cross-module integration scenarios verifying end-to-end behavior of:
//! - Rejection log correct writing (all required fields, file logger, session mode)
//! - Plan archive complete flow (archival, integrity, edge cases)
//!
//! All tests use `tempfile::TempDir` — no hardcoded paths, no external dependencies.

use std::collections::HashMap;
use std::sync::Arc;

use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::session_mode_query::SessionModeQuery;
use closeclaw_permission::engine::{
    Effect, PermissionEngine, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use closeclaw_permission::engine::{FileRejectionLogger, RejectionLog, RejectionLogger};
use closeclaw_permission::rules::RuleSetBuilder;
use closeclaw_session::plan_archive::archive_completed_plans_with_threshold;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Mock session mode query — maps agent IDs to modes.
struct MockModeQuery {
    modes: HashMap<String, SessionMode>,
}

impl MockModeQuery {
    fn new() -> Self {
        Self {
            modes: HashMap::new(),
        }
    }

    fn with_mode(mut self, agent_id: &str, mode: SessionMode) -> Self {
        self.modes.insert(agent_id.to_string(), mode);
        self
    }
}

impl SessionModeQuery for MockModeQuery {
    fn get_session_mode(&self, agent_id: &str) -> Option<SessionMode> {
        self.modes.get(agent_id).copied()
    }
}

/// In-memory rejection logger for tests.
struct InMemoryLogger {
    entries: std::sync::Mutex<Vec<RejectionLog>>,
}

impl InMemoryLogger {
    fn new() -> Self {
        Self {
            entries: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn entries(&self) -> Vec<RejectionLog> {
        self.entries.lock().unwrap().clone()
    }

    fn count(&self) -> usize {
        self.entries.lock().unwrap().len()
    }
}

impl RejectionLogger for InMemoryLogger {
    fn log(&self, entry: &RejectionLog) {
        self.entries.lock().unwrap().push(entry.clone());
    }
}

/// Build an allow-all engine with mode query injected.
fn allow_all_engine_with_mode(mode: SessionMode) -> PermissionEngine {
    let ruleset = RuleSetBuilder::new()
        .default_file(Effect::Allow)
        .default_command(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_config(Effect::Allow)
        .build()
        .unwrap();
    let query: Arc<dyn SessionModeQuery> =
        Arc::new(MockModeQuery::new().with_mode("test-agent", mode));
    PermissionEngine::new_with_default_data_root(ruleset).with_session_mode_query(query)
}

/// Build a deny-all engine (no mode query, no rejection logger).
fn deny_all_engine() -> PermissionEngine {
    let ruleset = RuleSetBuilder::new()
        .default_file(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .build()
        .unwrap();
    PermissionEngine::new_with_default_data_root(ruleset)
}

/// Build a deny-all engine with rejection logger.
fn deny_all_engine_with_logger(logger: Arc<dyn RejectionLogger>) -> PermissionEngine {
    let ruleset = RuleSetBuilder::new()
        .default_file(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .build()
        .unwrap();
    PermissionEngine::new_with_default_data_root(ruleset).with_rejection_logger(logger)
}

// ============================================================================
// 1. Rejection log correct writing
// ============================================================================

/// File write rejection → log contains correct entry with all required fields.
#[test]
fn test_rejection_log_e2e_file_write_entry() {
    let logger = Arc::new(InMemoryLogger::new());
    let engine = deny_all_engine_with_logger(logger.clone());

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "agent-1".to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );

    assert!(matches!(resp, PermissionResponse::Denied { .. }));
    assert_eq!(logger.count(), 1);

    let entry = &logger.entries()[0];
    assert_eq!(entry.agent_id, "agent-1");
    assert_eq!(entry.tool_name, "file");
    assert_eq!(entry.operation, "write /repo/src/main.rs");
    assert!(!entry.reason.is_empty());
    assert!(!entry.timestamp.is_empty());
}

/// Command rejection → log contains correct tool_name and operation.
#[test]
fn test_rejection_log_e2e_command_entry() {
    let logger = Arc::new(InMemoryLogger::new());
    let engine = deny_all_engine_with_logger(logger.clone());

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "agent-2".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string(), "/tmp/foo".to_string()],
        }),
        None,
    );

    assert!(matches!(resp, PermissionResponse::Denied { .. }));
    assert_eq!(logger.count(), 1);

    let entry = &logger.entries()[0];
    assert_eq!(entry.agent_id, "agent-2");
    assert_eq!(entry.tool_name, "command");
    assert_eq!(entry.operation, "rm -rf /tmp/foo");
}

/// ConfigWrite rejection → log contains config_write tool_name.
#[test]
fn test_rejection_log_e2e_config_write_entry() {
    let logger = Arc::new(InMemoryLogger::new());
    let engine = deny_all_engine_with_logger(logger.clone());

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::ConfigWrite {
            agent: "agent-3".to_string(),
            config_file: "config.json".to_string(),
        }),
        None,
    );

    assert!(matches!(resp, PermissionResponse::Denied { .. }));

    let entry = &logger.entries()[0];
    assert_eq!(entry.tool_name, "config_write");
    assert_eq!(entry.operation, "config.json");
}

/// Rejection log records session mode when mode query is set.
#[test]
fn test_rejection_log_e2e_records_session_mode() {
    let logger = Arc::new(InMemoryLogger::new());
    let mode_query: Arc<dyn SessionModeQuery> =
        Arc::new(MockModeQuery::new().with_mode("agent-4", SessionMode::Plan));
    let engine = deny_all_engine_with_logger(logger.clone()).with_session_mode_query(mode_query);

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "agent-4".to_string(),
            path: "/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );

    assert!(matches!(resp, PermissionResponse::Denied { .. }));
    let entry = &logger.entries()[0];
    assert_eq!(entry.session_mode, Some(SessionMode::Plan));
}

/// No rejection logger: engine still works, denials just not logged.
#[test]
fn test_rejection_log_e2e_no_logger_still_works() {
    let engine = deny_all_engine();

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "agent-5".to_string(),
            path: "/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );

    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "expected Denied even without logger"
    );
}

/// Multiple denials in sequence → each logged with correct entry.
#[test]
fn test_rejection_log_e2e_multiple_denials() {
    let logger = Arc::new(InMemoryLogger::new());
    let engine = deny_all_engine_with_logger(logger.clone());

    let requests = vec![
        PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/x".to_string(),
            op: "write".to_string(),
        },
        PermissionRequestBody::CommandExec {
            agent: "b".to_string(),
            cmd: "ls".to_string(),
            args: vec![],
        },
        PermissionRequestBody::ConfigWrite {
            agent: "c".to_string(),
            config_file: "config.json".to_string(),
        },
    ];

    for body in requests {
        engine.evaluate(PermissionRequest::Bare(body), None);
    }

    let entries = logger.entries();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].agent_id, "a");
    assert_eq!(entries[0].tool_name, "file");
    assert_eq!(entries[1].agent_id, "b");
    assert_eq!(entries[1].tool_name, "command");
    assert_eq!(entries[2].agent_id, "c");
    assert_eq!(entries[2].tool_name, "config_write");
}

/// File rejection logger: writes JSON lines to disk, entries parseable.
#[test]
fn test_rejection_log_e2e_file_logger_disk() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_path = dir.path().join("rejections.log");
    let file_logger = Arc::new(FileRejectionLogger::new(log_path.clone()).unwrap());
    let engine = deny_all_engine_with_logger(file_logger);

    for i in 0..3 {
        engine.evaluate(
            PermissionRequest::Bare(PermissionRequestBody::FileOp {
                agent: format!("agent-{}", i),
                path: format!("/file-{}", i),
                op: "write".to_string(),
            }),
            None,
        );
    }

    let content = std::fs::read_to_string(&log_path).unwrap();
    let lines: Vec<&str> = content.trim().lines().collect();
    assert_eq!(lines.len(), 3, "expected 3 log lines on disk");

    for (i, line) in lines.iter().enumerate() {
        let parsed: RejectionLog = serde_json::from_str(line).unwrap();
        assert_eq!(parsed.agent_id, format!("agent-{}", i));
        assert_eq!(parsed.tool_name, "file");
        assert!(!parsed.timestamp.is_empty());
    }
}

/// File rejection logger: parent directories are created automatically.
#[test]
fn test_rejection_log_e2e_file_logger_creates_dirs() {
    let dir = tempfile::TempDir::new().unwrap();
    let log_path = dir.path().join("sub").join("dir").join("rejections.log");
    let file_logger = Arc::new(FileRejectionLogger::new(log_path.clone()).unwrap());
    let engine = deny_all_engine_with_logger(file_logger);

    engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/x".to_string(),
            op: "write".to_string(),
        }),
        None,
    );

    assert!(log_path.exists());
    let content = std::fs::read_to_string(&log_path).unwrap();
    let parsed: RejectionLog = serde_json::from_str(content.trim()).unwrap();
    assert_eq!(parsed.agent_id, "a");
}

/// Plan mode denial → rejection log records session_mode = Plan.
#[test]
fn test_rejection_log_e2e_plan_mode_denial_logged() {
    let logger = Arc::new(InMemoryLogger::new());
    let mode_query: Arc<dyn SessionModeQuery> =
        Arc::new(MockModeQuery::new().with_mode("plan-agent", SessionMode::Plan));
    let engine = allow_all_engine_with_mode(SessionMode::Plan)
        .with_rejection_logger(logger.clone())
        .with_session_mode_query(mode_query);

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "plan-agent".to_string(),
            path: "/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );

    assert!(matches!(resp, PermissionResponse::Denied { .. }));
    assert_eq!(logger.count(), 1);
    let entry = &logger.entries()[0];
    assert_eq!(entry.session_mode, Some(SessionMode::Plan));
    assert!(entry.reason.contains("Plan mode"));
}

// ============================================================================
// 2. Plan archive complete flow
// ============================================================================

/// Plan archive: full flow — create plans, archive old ones, verify integrity.
#[test]
fn test_plan_archive_e2e_complete_flow() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans_dir = dir.path().join("plans");
    std::fs::create_dir_all(&plans_dir).unwrap();

    let completed_old = plans_dir.join("completed-old.md");
    std::fs::write(
        &completed_old,
        "# Plan Old\n\n| 状态 | completed |\n\n## Tasks\n- [x] Done\n",
    )
    .unwrap();
    filetime::set_file_mtime(
        &completed_old,
        filetime::FileTime::from_system_time(
            std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400),
        ),
    )
    .unwrap();

    let completed_new = plans_dir.join("completed-new.md");
    std::fs::write(
        &completed_new,
        "# Plan New\n\n| 状态 | completed |\n\n## Tasks\n- [x] Done\n",
    )
    .unwrap();

    let draft_old = plans_dir.join("draft-old.md");
    std::fs::write(
        &draft_old,
        "# Draft Old\n\n| 状态 | draft |\n\n## Tasks\n- [ ] Todo\n",
    )
    .unwrap();
    filetime::set_file_mtime(
        &draft_old,
        filetime::FileTime::from_system_time(
            std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400),
        ),
    )
    .unwrap();

    let executing_old = plans_dir.join("executing-old.md");
    std::fs::write(
        &executing_old,
        "# Executing\n\n| 状态 | executing |\n\n## Tasks\n- [x] A\n- [ ] B\n",
    )
    .unwrap();
    filetime::set_file_mtime(
        &executing_old,
        filetime::FileTime::from_system_time(
            std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400),
        ),
    )
    .unwrap();

    let count = archive_completed_plans_with_threshold(dir.path(), 7).unwrap();
    assert_eq!(count, 1, "only completed-old should be archived");

    let archive_dir = plans_dir.join("archive");
    assert!(archive_dir.exists());
    let archived = archive_dir.join("completed-old.md");
    assert!(archived.exists(), "completed-old should be in archive/");

    let content = std::fs::read_to_string(&archived).unwrap();
    assert!(content.contains("completed"));
    assert!(content.contains("# Plan Old"));

    assert!(completed_new.exists(), "completed-new should stay");
    assert!(draft_old.exists(), "draft-old should stay");
    assert!(executing_old.exists(), "executing-old should stay");
}

/// Plan archive: no plans/ directory → returns 0, no error.
#[test]
fn test_plan_archive_e2e_no_plans_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    let count = archive_completed_plans_with_threshold(dir.path(), 7).unwrap();
    assert_eq!(count, 0);
}

/// Plan archive: empty plans/ directory → returns 0.
#[test]
fn test_plan_archive_e2e_empty_plans_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("plans")).unwrap();
    let count = archive_completed_plans_with_threshold(dir.path(), 7).unwrap();
    assert_eq!(count, 0);
}

/// Plan archive: multiple old completed plans → all archived.
#[test]
fn test_plan_archive_e2e_multiple_old_completed() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans_dir = dir.path().join("plans");
    std::fs::create_dir_all(&plans_dir).unwrap();

    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);

    for i in 0..5 {
        let path = plans_dir.join(format!("plan-{}.md", i));
        std::fs::write(&path, format!("# Plan {}\n\n| 状态 | completed |\n", i)).unwrap();
        filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(old_time)).unwrap();
    }

    let count = archive_completed_plans_with_threshold(dir.path(), 7).unwrap();
    assert_eq!(count, 5);

    let archive_dir = plans_dir.join("archive");
    for i in 0..5 {
        assert!(
            archive_dir.join(format!("plan-{}.md", i)).exists(),
            "plan-{}.md should be archived",
            i
        );
    }
}

/// Plan archive: threshold not met → no archive.
#[test]
fn test_plan_archive_e2e_threshold_not_met() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans_dir = dir.path().join("plans");
    std::fs::create_dir_all(&plans_dir).unwrap();

    let path = plans_dir.join("recent.md");
    std::fs::write(&path, "# Plan\n\n| 状态 | completed |\n").unwrap();

    let count = archive_completed_plans_with_threshold(dir.path(), 7).unwrap();
    assert_eq!(count, 0);
    assert!(path.exists());
}

/// Plan archive: archived file content is byte-identical.
#[test]
fn test_plan_archive_e2e_content_integrity() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans_dir = dir.path().join("plans");
    std::fs::create_dir_all(&plans_dir).unwrap();

    let original_content =
        "# My Plan\n\n| 状态 | completed |\n\n## Tasks\n\n- [x] Step 1\n- [x] Step 2\n";
    let path = plans_dir.join("integrity-test.md");
    std::fs::write(&path, original_content).unwrap();

    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);
    filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(old_time)).unwrap();

    archive_completed_plans_with_threshold(dir.path(), 7).unwrap();

    let archived = plans_dir.join("archive").join("integrity-test.md");
    let archived_content = std::fs::read_to_string(&archived).unwrap();
    assert_eq!(archived_content, original_content);
}

/// Plan archive: non-.md files in plans/ are skipped.
#[test]
fn test_plan_archive_e2e_skips_non_md() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans_dir = dir.path().join("plans");
    std::fs::create_dir_all(&plans_dir).unwrap();

    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);

    let txt_path = plans_dir.join("notes.txt");
    std::fs::write(&txt_path, "not a plan").unwrap();
    filetime::set_file_mtime(&txt_path, filetime::FileTime::from_system_time(old_time)).unwrap();

    let json_path = plans_dir.join("data.json");
    std::fs::write(&json_path, "{}").unwrap();
    filetime::set_file_mtime(&json_path, filetime::FileTime::from_system_time(old_time)).unwrap();

    let count = archive_completed_plans_with_threshold(dir.path(), 7).unwrap();
    assert_eq!(count, 0);
    assert!(txt_path.exists());
    assert!(json_path.exists());
}

/// Plan archive: already-archived files in archive/ are not re-archived.
#[test]
fn test_plan_archive_e2e_skips_archive_subdir() {
    let dir = tempfile::TempDir::new().unwrap();
    let plans_dir = dir.path().join("plans");
    let archive_dir = plans_dir.join("archive");
    std::fs::create_dir_all(&archive_dir).unwrap();

    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86400);

    let archived_path = archive_dir.join("already-here.md");
    std::fs::write(&archived_path, "# Old\n\n| 状态 | completed |\n").unwrap();
    filetime::set_file_mtime(
        &archived_path,
        filetime::FileTime::from_system_time(old_time),
    )
    .unwrap();

    let count = archive_completed_plans_with_threshold(dir.path(), 7).unwrap();
    assert_eq!(count, 0);
    assert!(archived_path.exists());
}
