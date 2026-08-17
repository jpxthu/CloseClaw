//! Integration tests for AuditLogger with PermissionEngine.
//!
//! Covers: audit log records both rejections AND allowed decisions,
//! audit logs only in Auto Mode, no logs in other modes or without
//! session_mode_query.

use crate::engine::audit_log::{AuditDisposition, AuditLogEntry, AuditLogger};
use crate::engine::engine_eval::PermissionEngine;
use crate::engine::engine_risk::RiskLevel;
use crate::engine::engine_types::{
    Effect, PermissionRequest, PermissionRequestBody, PermissionResponse,
};
use crate::engine::rejection_log::{RejectionLog, RejectionLogger};
use crate::rules::RuleSetBuilder;
use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::session_mode_query::SessionModeQuery;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

struct MockModeQuery {
    mode: Option<SessionMode>,
}

impl SessionModeQuery for MockModeQuery {
    fn get_session_mode(&self, _agent_id: &str) -> Option<SessionMode> {
        self.mode
    }
}

struct TestRejectionLogger {
    entries: Mutex<Vec<RejectionLog>>,
}

impl TestRejectionLogger {
    fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }

    fn entries(&self) -> Vec<RejectionLog> {
        self.entries.lock().unwrap().clone()
    }
}

impl RejectionLogger for TestRejectionLogger {
    fn log(&self, entry: &RejectionLog) {
        self.entries.lock().unwrap().push(entry.clone());
    }
}

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

/// Build a deny-all engine with both loggers injected.
fn deny_all_engine_with_loggers(
    mode: Option<SessionMode>,
    rejection_logger: Arc<dyn RejectionLogger>,
    audit_logger: Arc<dyn AuditLogger>,
) -> PermissionEngine {
    let query = Arc::new(MockModeQuery { mode });
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .build()
        .unwrap();
    PermissionEngine::new_with_default_data_root(ruleset)
        .with_session_mode_query(query)
        .with_rejection_logger(rejection_logger)
        .with_audit_logger(audit_logger)
}

// ---------------------------------------------------------------------------
// test_engine_logs_rejection_to_audit_log
// ---------------------------------------------------------------------------

#[test]
fn test_engine_logs_rejection_to_audit_log() {
    let rejection_logger = Arc::new(TestRejectionLogger::new());
    let audit_logger = Arc::new(TestAuditLogger::new());
    let engine = deny_all_engine_with_loggers(
        Some(SessionMode::Auto),
        rejection_logger.clone(),
        audit_logger.clone(),
    );

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "agent-auto".to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );

    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "expected Denied"
    );

    // Rejection log should have one entry.
    let rejection_entries = rejection_logger.entries();
    assert_eq!(
        rejection_entries.len(),
        1,
        "expected 1 rejection log entry"
    );
    assert_eq!(rejection_entries[0].agent_id, "agent-auto");

    // Audit log should also have one entry.
    let audit_entries = audit_logger.entries();
    assert_eq!(
        audit_entries.len(),
        1,
        "expected 1 audit log entry on rejection"
    );

    let entry = &audit_entries[0];
    assert_eq!(entry.agent_id, "agent-auto");
    assert_eq!(entry.tool_name, "file");
    assert_eq!(entry.operation, "write /repo/src/main.rs");
    assert_eq!(entry.disposition, AuditDisposition::Rejected);
    assert_eq!(entry.risk_level, RiskLevel::Low);
    assert_eq!(entry.session_mode, Some(SessionMode::Auto));
    assert!(!entry.timestamp.is_empty());
}

// ---------------------------------------------------------------------------
// test_engine_audit_log_auto_mode_only
// ---------------------------------------------------------------------------

#[test]
fn test_engine_audit_log_auto_mode_only() {
    let rejection_logger = Arc::new(TestRejectionLogger::new());
    let audit_logger = Arc::new(TestAuditLogger::new());

    // --- Plan Mode: audit log should NOT be written ---
    let engine_plan = deny_all_engine_with_loggers(
        Some(SessionMode::Plan),
        rejection_logger.clone(),
        audit_logger.clone(),
    );
    // Write in Plan mode is denied by plan_mode_filter, not auto_mode_filter.
    let resp_plan = engine_plan.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "agent-plan".to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(matches!(resp_plan, PermissionResponse::Denied { .. }));
    assert!(
        audit_logger.entries().is_empty(),
        "no audit log expected in Plan Mode"
    );

    // --- Normal Mode: audit log should NOT be written ---
    let engine_normal = deny_all_engine_with_loggers(
        Some(SessionMode::Normal),
        rejection_logger.clone(),
        audit_logger.clone(),
    );
    let resp_normal = engine_normal.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "agent-normal".to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(matches!(resp_normal, PermissionResponse::Denied { .. }));
    assert!(
        audit_logger.entries().is_empty(),
        "no audit log expected in Normal Mode"
    );

    // --- No session_mode_query: audit log should NOT be written ---
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .default_command(Effect::Deny)
        .default_network(Effect::Deny)
        .default_inter_agent(Effect::Deny)
        .default_config(Effect::Deny)
        .build()
        .unwrap();
    let engine_no_query =
        PermissionEngine::new_with_default_data_root(ruleset)
            .with_rejection_logger(rejection_logger.clone())
            .with_audit_logger(audit_logger.clone());
    let resp_no_query = engine_no_query.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "agent-none".to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(matches!(resp_no_query, PermissionResponse::Denied { .. }));
    assert!(
        audit_logger.entries().is_empty(),
        "no audit log expected without session_mode_query"
    );

    // --- Auto Mode: audit log SHOULD be written ---
    let engine_auto = deny_all_engine_with_loggers(
        Some(SessionMode::Auto),
        rejection_logger.clone(),
        audit_logger.clone(),
    );
    let resp_auto = engine_auto.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "agent-auto".to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );
    assert!(matches!(resp_auto, PermissionResponse::Denied { .. }));
    let entries = audit_logger.entries();
    assert_eq!(
        entries.len(),
        1,
        "expected 1 audit log entry in Auto Mode"
    );
    assert_eq!(entries[0].disposition, AuditDisposition::Rejected);
    assert_eq!(entries[0].session_mode, Some(SessionMode::Auto));
}
