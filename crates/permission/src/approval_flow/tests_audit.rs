//! Audit logging integration tests for ApprovalFlow.
//!
//! Extracted from `tests.rs` to keep that file under the
//! 1000-line limit.

use super::super::*;
use super::{test_approval_flow, test_runtime, test_session_lookup};
use crate::engine::audit_log::{AuditDisposition, AuditLogEntry, AuditLogger, FileAuditLogger};
use crate::engine::engine_types::RuleSet;
use crate::mock_session_lookup::MockSessionLookup;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

/// Thread-safe in-memory audit logger for tests.
struct TestAuditLogger {
    entries: Mutex<Vec<AuditLogEntry>>,
}

impl TestAuditLogger {
    fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }

    fn entries(&self) -> Vec<AuditLogEntry> {
        self.entries.lock().unwrap().clone()
    }
}

impl AuditLogger for TestAuditLogger {
    fn log(&self, entry: &AuditLogEntry) {
        self.entries.lock().unwrap().push(entry.clone());
    }
}

fn flow_with_audit_logger(logger: Arc<dyn AuditLogger>) -> ApprovalFlow {
    ApprovalFlow::new(
        Arc::new(MockSessionLookup::new()),
        Arc::new(|_: ApprovalNotification| {}),
        Arc::new(|_| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        RuleSet::default(),
    )
    .with_audit_logger(logger)
}

fn test_caller() -> Caller {
    Caller {
        user_id: "user_1".to_string(),
        agent: "agent_1".to_string(),
        creator_id: "creator_1".to_string(),
    }
}

fn test_request() -> PermissionRequestBody {
    PermissionRequestBody::ToolCall {
        agent: "agent_1".to_string(),
        skill: "test_skill".to_string(),
        method: "test_method".to_string(),
    }
}

#[tokio::test]
async fn test_approval_flow_logs_approved_to_audit_log() {
    let logger = Arc::new(TestAuditLogger::new());
    let mut flow = flow_with_audit_logger(logger.clone() as Arc<dyn AuditLogger>);
    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();
    let result = flow.approve_request(&request_id, ApprovalMode::Once).await;
    assert!(result.is_ok());
    assert!(result.unwrap());

    let entries = logger.entries();
    assert_eq!(entries.len(), 1, "should have one audit log entry");
    assert_eq!(entries[0].disposition, AuditDisposition::Approved);
    assert_eq!(entries[0].agent_id, "agent_1");
    assert_eq!(entries[0].tool_name, "tool_call");
    assert_eq!(entries[0].operation, "test_skill.test_method");
    assert_eq!(entries[0].reason, "user approved");
    assert_eq!(entries[0].risk_level, RiskLevel::Low);
}

#[tokio::test]
async fn test_approval_flow_logs_denied_to_audit_log() {
    let logger = Arc::new(TestAuditLogger::new());
    let mut flow = flow_with_audit_logger(logger.clone() as Arc<dyn AuditLogger>);
    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();
    let result = flow.deny_request(&request_id);
    assert!(result);

    let entries = logger.entries();
    assert_eq!(entries.len(), 1, "should have one audit log entry");
    assert_eq!(entries[0].disposition, AuditDisposition::Rejected);
    assert_eq!(entries[0].agent_id, "agent_1");
    assert_eq!(entries[0].tool_name, "tool_call");
    assert_eq!(entries[0].operation, "test_skill.test_method");
    assert_eq!(entries[0].reason, "user denied");
    assert_eq!(entries[0].risk_level, RiskLevel::Low);
}

#[test]
fn test_approval_flow_no_audit_log_when_not_configured() {
    let rt = test_runtime();
    let sm = test_session_lookup();
    let notify_count = Arc::new(AtomicUsize::new(0));
    let mut flow = test_approval_flow(sm, Arc::clone(&notify_count), &rt);
    // No audit logger configured — approve/deny should not
    // panic.
    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();
    let result = flow.deny_request(&request_id);
    assert!(result);
}

#[tokio::test]
async fn test_file_audit_logger_integration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("approval_audit.log");
    let logger = Arc::new(FileAuditLogger::new(path.clone()).unwrap());
    let mut flow = flow_with_audit_logger(logger.clone() as Arc<dyn AuditLogger>);
    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::Low, "session_1", false)
        .unwrap();
    flow.approve_request(&request_id, ApprovalMode::Once)
        .await
        .unwrap();

    // Verify the audit log file contains the entry.
    let content = std::fs::read_to_string(&path).unwrap();
    let entry: AuditLogEntry = serde_json::from_str(content.trim()).unwrap();
    assert_eq!(entry.disposition, AuditDisposition::Approved);
    assert_eq!(entry.agent_id, "agent_1");
}

#[tokio::test]
async fn test_approved_audit_log_records_high_risk_level() {
    let logger = Arc::new(TestAuditLogger::new());
    let mut flow = flow_with_audit_logger(logger.clone() as Arc<dyn AuditLogger>);
    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::High, "session_1", false)
        .unwrap();
    let result = flow.approve_request(&request_id, ApprovalMode::Once).await;
    assert!(result.is_ok());
    assert!(result.unwrap());

    let entries = logger.entries();
    assert_eq!(entries.len(), 1, "should have one audit log entry");
    assert_eq!(entries[0].disposition, AuditDisposition::Approved);
    assert_eq!(entries[0].risk_level, RiskLevel::High);
}

#[tokio::test]
async fn test_denied_audit_log_records_high_risk_level() {
    let logger = Arc::new(TestAuditLogger::new());
    let mut flow = flow_with_audit_logger(logger.clone() as Arc<dyn AuditLogger>);
    let caller = test_caller();
    let request = test_request();
    let request_id = flow
        .submit_denial(&caller, &request, RiskLevel::High, "session_1", false)
        .unwrap();
    let result = flow.deny_request(&request_id);
    assert!(result);

    let entries = logger.entries();
    assert_eq!(entries.len(), 1, "should have one audit log entry");
    assert_eq!(entries[0].disposition, AuditDisposition::Rejected);
    assert_eq!(entries[0].risk_level, RiskLevel::High);
}
