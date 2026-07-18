//! Integration tests for RejectionLogger with PermissionEngine.
//!
//! Covers: rejection logs only in Auto Mode, no logs in Plan/Normal/unknown
//! modes or when session_mode_query is absent, concurrent safety.

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

/// In-memory logger for integration tests.
struct TestLogger {
    entries: Mutex<Vec<RejectionLog>>,
}

impl TestLogger {
    fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }

    fn entries(&self) -> Vec<RejectionLog> {
        self.entries.lock().unwrap().clone()
    }
}

impl RejectionLogger for TestLogger {
    fn log(&self, entry: &RejectionLog) {
        self.entries.lock().unwrap().push(entry.clone());
    }
}

/// Mock session mode query for tests.
struct MockModeQuery {
    mode: Option<SessionMode>,
}

impl SessionModeQuery for MockModeQuery {
    fn get_session_mode(&self, _agent_id: &str) -> Option<SessionMode> {
        self.mode
    }
}

/// Build a deny-all engine.
fn deny_all_engine() -> PermissionEngine {
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
}

// --- Auto Mode: rejection logs are recorded ---

#[test]
fn test_rejection_logged_in_auto_mode() {
    let logger = Arc::new(TestLogger::new());
    let mode_query = Arc::new(MockModeQuery {
        mode: Some(SessionMode::Auto),
    });
    let engine = deny_all_engine()
        .with_session_mode_query(mode_query)
        .with_rejection_logger(logger.clone());

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
    let entries = logger.entries();
    assert_eq!(entries.len(), 1, "expected 1 log entry in Auto Mode");

    let log = &entries[0];
    assert_eq!(log.agent_id, "agent-auto");
    assert_eq!(log.tool_name, "file");
    assert_eq!(log.operation, "write /repo/src/main.rs");
    assert!(!log.reason.is_empty());
    assert_eq!(log.risk_level, RiskLevel::Low);
    assert!(!log.timestamp.is_empty());
    assert_eq!(log.session_mode, Some(SessionMode::Auto));
}

#[test]
fn test_rejection_logged_on_command_deny_auto_mode() {
    let logger = Arc::new(TestLogger::new());
    let mode_query = Arc::new(MockModeQuery {
        mode: Some(SessionMode::Auto),
    });
    let engine = deny_all_engine()
        .with_session_mode_query(mode_query)
        .with_rejection_logger(logger.clone());

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "agent-2".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string(), "/tmp/foo".to_string()],
        }),
        None,
    );

    assert!(matches!(resp, PermissionResponse::Denied { .. }));
    let entries = logger.entries();
    assert_eq!(entries.len(), 1);

    let log = &entries[0];
    assert_eq!(log.agent_id, "agent-2");
    assert_eq!(log.tool_name, "command");
    assert_eq!(log.operation, "rm -rf /tmp/foo");
    assert!(!log.reason.is_empty());
}

#[test]
fn test_concurrent_evaluate_with_logger() {
    use std::thread;

    let logger = Arc::new(TestLogger::new());
    let mode_query = Arc::new(MockModeQuery {
        mode: Some(SessionMode::Auto),
    });
    let engine = Arc::new(
        deny_all_engine()
            .with_session_mode_query(mode_query)
            .with_rejection_logger(logger.clone()),
    );

    let handles: Vec<_> = (0..20)
        .map(|i| {
            let engine = engine.clone();
            thread::spawn(move || {
                let resp = engine.evaluate(
                    PermissionRequest::Bare(PermissionRequestBody::FileOp {
                        agent: format!("agent-{}", i % 5),
                        path: "/repo/src/main.rs".to_string(),
                        op: "write".to_string(),
                    }),
                    None,
                );
                assert!(matches!(resp, PermissionResponse::Denied { .. }));
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    let entries = logger.entries();
    assert_eq!(
        entries.len(),
        20,
        "expected 20 log entries from concurrent evaluate calls"
    );
}

#[test]
fn test_multiple_denials_log_each_auto_mode() {
    let logger = Arc::new(TestLogger::new());
    let mode_query = Arc::new(MockModeQuery {
        mode: Some(SessionMode::Auto),
    });
    let engine = deny_all_engine()
        .with_session_mode_query(mode_query)
        .with_rejection_logger(logger.clone());

    // Evaluate 3 different denied requests
    let requests = vec![
        PermissionRequestBody::FileOp {
            agent: "a".to_string(),
            path: "/x".to_string(),
            op: "write".to_string(),
        },
        PermissionRequestBody::CommandExec {
            agent: "a".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string()],
        },
        PermissionRequestBody::ConfigWrite {
            agent: "a".to_string(),
            config_file: "config.json".to_string(),
        },
    ];

    for body in requests {
        engine.evaluate(PermissionRequest::Bare(body), None);
    }

    let entries = logger.entries();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].tool_name, "file");
    assert_eq!(entries[1].tool_name, "command");
    assert_eq!(entries[2].tool_name, "config_write");
}

// --- Non-Auto Mode: rejection logs are NOT recorded ---

#[test]
fn test_rejection_not_logged_in_plan_mode() {
    let logger = Arc::new(TestLogger::new());
    let mode_query = Arc::new(MockModeQuery {
        mode: Some(SessionMode::Plan),
    });
    let engine = deny_all_engine()
        .with_session_mode_query(mode_query)
        .with_rejection_logger(logger.clone());

    // Write operation in Plan mode is denied by plan mode filter
    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "agent-plan".to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );

    assert!(matches!(resp, PermissionResponse::Denied { .. }));
    let entries = logger.entries();
    assert_eq!(entries.len(), 0, "no log entries expected in Plan Mode");
}

#[test]
fn test_rejection_not_logged_in_normal_mode() {
    let logger = Arc::new(TestLogger::new());
    let mode_query = Arc::new(MockModeQuery {
        mode: Some(SessionMode::Normal),
    });
    let engine = deny_all_engine()
        .with_session_mode_query(mode_query)
        .with_rejection_logger(logger.clone());

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "agent-normal".to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );

    assert!(matches!(resp, PermissionResponse::Denied { .. }));
    let entries = logger.entries();
    assert_eq!(entries.len(), 0, "no log entries expected in Normal Mode");
}

// --- No session_mode_query: rejection logs are NOT recorded ---

#[test]
fn test_rejection_not_logged_without_mode_query() {
    let logger = Arc::new(TestLogger::new());
    let engine = deny_all_engine().with_rejection_logger(logger.clone());

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "agent-none".to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );

    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "expected Denied"
    );
    let entries = logger.entries();
    assert_eq!(
        entries.len(),
        0,
        "no log entries expected without session_mode_query"
    );
}

#[test]
fn test_rejection_not_logged_without_mode_query_command() {
    let logger = Arc::new(TestLogger::new());
    let engine = deny_all_engine().with_rejection_logger(logger.clone());

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::CommandExec {
            agent: "agent-none-cmd".to_string(),
            cmd: "rm".to_string(),
            args: vec!["-rf".to_string(), "/tmp/foo".to_string()],
        }),
        None,
    );

    assert!(matches!(resp, PermissionResponse::Denied { .. }));
    let entries = logger.entries();
    assert_eq!(
        entries.len(),
        0,
        "no log entries expected without session_mode_query"
    );
}

// --- Unchanged tests: logger behavior edge cases ---

#[test]
fn test_no_logger_does_not_crash() {
    // Engine with no rejection logger should still work fine
    let engine = deny_all_engine();
    assert!(engine.rejection_logger().is_none());

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "agent-4".to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );

    assert!(
        matches!(resp, PermissionResponse::Denied { .. }),
        "expected Denied even without logger"
    );
}

#[test]
fn test_allowed_request_does_not_log() {
    let logger = Arc::new(TestLogger::new());
    let mode_query = Arc::new(MockModeQuery {
        mode: Some(SessionMode::Auto),
    });
    // All-allow engine
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Allow)
        .default_file_write(Effect::Allow)
        .default_command(Effect::Allow)
        .default_network(Effect::Allow)
        .default_inter_agent(Effect::Allow)
        .default_config(Effect::Allow)
        .build()
        .unwrap();
    let engine = PermissionEngine::new_with_default_data_root(ruleset)
        .with_session_mode_query(mode_query)
        .with_rejection_logger(logger.clone());

    let resp = engine.evaluate(
        PermissionRequest::Bare(PermissionRequestBody::FileOp {
            agent: "agent-5".to_string(),
            path: "/repo/src/main.rs".to_string(),
            op: "write".to_string(),
        }),
        None,
    );

    assert!(matches!(resp, PermissionResponse::Allowed { .. }));
    assert!(
        logger.entries().is_empty(),
        "no log entries expected for allowed request"
    );
}
