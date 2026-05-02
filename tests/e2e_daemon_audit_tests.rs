//! E2E tests for daemon audit logging lifecycle.
//!
//! Covers audit event generation, buffering, flush-on-shutdown, and
//! independence from external services (feishu/llm).

use std::path::PathBuf;
use std::sync::Arc;

use tempfile::TempDir;

use closeclaw::audit::{AuditEvent, AuditEventType, AuditLogger, AuditResult};
use closeclaw::daemon::Daemon;
use closeclaw::permission::{
    Action, Defaults, Effect, PermissionRequest, PermissionRequestBody, PermissionResponse, Rule,
    RuleSet, Subject,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a minimal agents.json in the given directory.
fn setup_agents_json(dir: &std::path::Path) -> std::io::Result<()> {
    let agents_content = serde_json::json!({
        "version": "1.0",
        "agents": [{
            "name": "guide",
            "model": "minimax/MiniMax-M2",
            "persona": "test persona",
            "max_iterations": 10,
            "timeout_minutes": 5
        }]
    });
    std::fs::write(dir.join("agents.json"), agents_content.to_string())
}

/// Build a PermissionRequest for the given agent with a command action.
fn make_permission_request(agent: &str) -> PermissionRequest {
    PermissionRequest::Bare(PermissionRequestBody::CommandExec {
        agent: agent.to_string(),
        cmd: "test-cmd".to_string(),
        args: vec![],
    })
}

/// Read all JSONL files in the audit directory and parse into AuditEvent list.
fn read_audit_events(audit_dir: &PathBuf) -> Vec<AuditEvent> {
    let mut events = Vec::new();
    let Ok(entries) = std::fs::read_dir(audit_dir) else {
        return events;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                if let Ok(evt) = serde_json::from_str::<AuditEvent>(line.trim()) {
                    events.push(evt);
                }
            }
        }
    }
    events
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// test_audit_config_reload_on_start: Daemon startup emits a ConfigReload
/// event with component="daemon" and a version field in details.
#[tokio::test]
async fn test_audit_config_reload_on_start() {
    let temp_dir = TempDir::new().expect("temp dir");
    let audit_dir = temp_dir.path().to_path_buf();
    setup_agents_json(temp_dir.path()).expect("setup agents.json");

    let audit_logger = Arc::new(AuditLogger::with_base_dir(audit_dir.clone()));
    let daemon = Daemon::start_with_audit_logger(
        temp_dir.path().to_str().unwrap(),
        Arc::clone(&audit_logger),
    )
    .await
    .expect("daemon start");

    // Wait for async startup event to be written
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let events = read_audit_events(&audit_dir);
    let cfg_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.event_type, AuditEventType::ConfigReload))
        .collect();

    assert!(
        !cfg_events.is_empty(),
        "ConfigReload event should exist after start"
    );
    assert_eq!(
        cfg_events[0]
            .details
            .get("component")
            .and_then(|v| v.as_str()),
        Some("daemon"),
        "ConfigReload details should contain component=daemon"
    );
    assert!(
        cfg_events[0].details.get("version").is_some(),
        "ConfigReload details should contain version"
    );
    assert_eq!(cfg_events[0].result, AuditResult::Allow);

    drop(daemon);
}

/// test_audit_permission_check_allow_deny: evaluate_with_audit generates
/// PermissionCheck events with AuditResult::Allow and AuditResult::Deny.
#[tokio::test]
async fn test_audit_permission_check_allow_deny() {
    let temp_dir = TempDir::new().expect("temp dir");
    let audit_dir = temp_dir.path().to_path_buf();
    setup_agents_json(temp_dir.path()).expect("setup agents.json");

    // Build a rule set that allows "test-agent" but denies everything else
    let ruleset = RuleSet {
        version: "1.0.0".to_string(),
        rules: vec![Rule {
            name: "allow-test-agent".to_string(),
            subject: Subject::AgentOnly {
                agent: "test-agent".to_string(),
                match_type: closeclaw::permission::MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![Action::Command {
                command: "test-cmd".to_string(),
                args: closeclaw::permission::CommandArgs::Any,
            }],
            template: None,
            priority: 0,
        }],
        defaults: Defaults::default(),
        template_includes: vec![],
        agent_creators: std::collections::HashMap::new(),
    };
    let engine = Arc::new(closeclaw::permission::PermissionEngine::new(ruleset));
    let audit_logger = Arc::new(AuditLogger::with_base_dir(audit_dir.clone()));

    let daemon = Daemon::start_with_audit_logger_and_engine(
        temp_dir.path().to_str().unwrap(),
        audit_logger,
        engine,
    )
    .await
    .expect("daemon start");

    // Give the startup ConfigReload event time to flush
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Allowed request
    let resp_allowed = daemon
        .evaluate_with_audit(make_permission_request("test-agent"))
        .await;
    assert!(
        matches!(resp_allowed, PermissionResponse::Allowed { .. }),
        "test-agent should be allowed"
    );

    // Denied request
    let resp_denied = daemon
        .evaluate_with_audit(make_permission_request("other-agent"))
        .await;
    assert!(
        matches!(resp_denied, PermissionResponse::Denied { .. }),
        "other-agent should be denied"
    );

    // Wait for spawned audit log tasks to complete
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    daemon.shutdown_audit().await;

    let events = read_audit_events(&audit_dir);
    let perm_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.event_type, AuditEventType::PermissionCheck))
        .collect();

    assert!(
        perm_events.len() >= 2,
        "Should have at least 2 PermissionCheck events, got {}",
        perm_events.len()
    );
    assert!(
        perm_events.iter().any(|e| e.result == AuditResult::Allow),
        "Should have an Allow PermissionCheck event"
    );
    assert!(
        perm_events.iter().any(|e| e.result == AuditResult::Deny),
        "Should have a Deny PermissionCheck event"
    );

    drop(daemon);
}

/// test_audit_agent_lifecycle_events: log_agent_start / log_agent_stop /
/// log_agent_error produce events with the correct type and result.
#[tokio::test]
async fn test_audit_agent_lifecycle_events() {
    let temp_dir = TempDir::new().expect("temp dir");
    let audit_dir = temp_dir.path().to_path_buf();
    setup_agents_json(temp_dir.path()).expect("setup agents.json");

    let audit_logger = Arc::new(AuditLogger::with_base_dir(audit_dir.clone()));
    let daemon = Daemon::start_with_audit_logger(
        temp_dir.path().to_str().unwrap(),
        Arc::clone(&audit_logger),
    )
    .await
    .expect("daemon start");

    // Emit all three lifecycle events
    daemon.log_agent_start("guide", "minimax/MiniMax-M2").await;
    daemon.log_agent_stop("guide").await;
    daemon.log_agent_error("guide", "test error message").await;

    // Wait for spawned log tasks, then flush
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    daemon.shutdown_audit().await;

    let events = read_audit_events(&audit_dir);

    let has_start = events
        .iter()
        .any(|e| matches!(e.event_type, AuditEventType::AgentStart));
    let has_stop = events
        .iter()
        .any(|e| matches!(e.event_type, AuditEventType::AgentStop));
    let has_error = events
        .iter()
        .any(|e| matches!(e.event_type, AuditEventType::AgentError));

    assert!(has_start, "AgentStart event should exist");
    assert!(has_stop, "AgentStop event should exist");
    assert!(has_error, "AgentError event should exist");

    // Verify results
    let start_evt = events
        .iter()
        .find(|e| matches!(e.event_type, AuditEventType::AgentStart))
        .unwrap();
    assert_eq!(start_evt.result, AuditResult::Allow);

    let stop_evt = events
        .iter()
        .find(|e| matches!(e.event_type, AuditEventType::AgentStop))
        .unwrap();
    assert_eq!(stop_evt.result, AuditResult::Allow);

    let error_evt = events
        .iter()
        .find(|e| matches!(e.event_type, AuditEventType::AgentError))
        .unwrap();
    assert_eq!(error_evt.result, AuditResult::Error);

    drop(daemon);
}

/// test_audit_buffer_flushed_on_shutdown: after shutdown_audit the buffer is
/// empty and all events that were logged appear in the file.
#[tokio::test]
async fn test_audit_buffer_flushed_on_shutdown() {
    let temp_dir = TempDir::new().expect("temp dir");
    let audit_dir = temp_dir.path().to_path_buf();
    setup_agents_json(temp_dir.path()).expect("setup agents.json");

    let audit_logger = Arc::new(AuditLogger::with_base_dir(audit_dir.clone()));
    let daemon = Daemon::start_with_audit_logger(
        temp_dir.path().to_str().unwrap(),
        Arc::clone(&audit_logger),
    )
    .await
    .expect("daemon start");

    // Wait for startup ConfigReload
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Log some events
    daemon.log_agent_start("guide", "minimax/MiniMax-M2").await;
    daemon.log_agent_stop("guide").await;
    daemon.log_agent_error("guide", "flush test").await;

    // Allow spawned log tasks to buffer events
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Count buffered events before shutdown
    let buffered = audit_logger.buffer_len().await;
    assert!(
        buffered > 0,
        "buffer should have events before flush, got {}",
        buffered
    );

    // Flush on shutdown
    daemon.shutdown_audit().await;

    // Buffer must be empty after shutdown
    let after_shutdown = audit_logger.buffer_len().await;
    assert_eq!(
        after_shutdown, 0,
        "buffer should be empty after shutdown_audit, got {}",
        after_shutdown
    );

    // Events must appear in the file
    let events = read_audit_events(&audit_dir);
    let agent_events: Vec<_> = events
        .iter()
        .filter(|e| {
            matches!(
                e.event_type,
                AuditEventType::AgentStart | AuditEventType::AgentStop | AuditEventType::AgentError
            )
        })
        .collect();

    assert_eq!(
        agent_events.len(),
        3,
        "all 3 lifecycle events should be persisted, got {}",
        agent_events.len()
    );

    drop(daemon);
}

/// test_audit_no_external_dependencies: audit logging works with only a
/// temporary directory and an in-memory permission engine — no feishu/llm
/// network calls are made.
#[tokio::test]
async fn test_audit_no_external_dependencies() {
    let temp_dir = TempDir::new().expect("temp dir");
    let audit_dir = temp_dir.path().to_path_buf();
    setup_agents_json(temp_dir.path()).expect("setup agents.json");

    // Engine with no rules — all requests denied
    let ruleset = RuleSet {
        version: "1.0.0".to_string(),
        rules: vec![],
        defaults: Defaults::default(),
        template_includes: vec![],
        agent_creators: std::collections::HashMap::new(),
    };
    let engine = Arc::new(closeclaw::permission::PermissionEngine::new(ruleset));
    let audit_logger = Arc::new(AuditLogger::with_base_dir(audit_dir.clone()));

    // No FEISHU_*/OPENCLAW_* env vars — Daemon::start_with_audit_logger_and_engine
    // will skip feishu/llm initialisation but still emit ConfigReload on startup.
    let daemon = Daemon::start_with_audit_logger_and_engine(
        temp_dir.path().to_str().unwrap(),
        audit_logger,
        engine,
    )
    .await
    .expect("daemon start");

    // Wait for startup ConfigReload
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Log an event
    daemon.log_agent_start("guide", "minimax/MiniMax-M2").await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    daemon.shutdown_audit().await;

    // Verify the audit file was created and contains our event
    let events = read_audit_events(&audit_dir);
    assert!(
        !events.is_empty(),
        "audit log should contain events (no external deps needed)"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event_type, AuditEventType::ConfigReload)),
        "startup ConfigReload should be present"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event_type, AuditEventType::AgentStart)),
        "AgentStart should be present"
    );

    drop(daemon);
}
