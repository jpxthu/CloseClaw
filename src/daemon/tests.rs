//! Daemon unit tests.

use std::path::PathBuf;
use std::sync::Arc;

use tempfile::TempDir;

use crate::audit::{AuditEvent, AuditEventType, AuditLogger, AuditResult};
use crate::daemon::Daemon;
use crate::permission::{
    Action, Caller, Defaults, Effect, PermissionRequest, PermissionRequestBody, PermissionResponse,
    Rule, RuleSet, Subject,
};
use crate::session::persistence::{PersistenceService, SessionStatus};

/// Create a minimal agents.json in the given directory.
fn setup_agents_json(dir: &std::path::Path) -> std::io::Result<()> {
    let agents_content = serde_json::json!({
        "version": "1.0",
        "agents": [
            {
                "name": "guide",
                "model": "minimax/MiniMax-M2",
                "persona": "test persona",
                "max_iterations": 10,
                "timeout_minutes": 5
            }
        ]
    });
    std::fs::write(dir.join("agents.json"), agents_content.to_string())?;
    Ok(())
}

#[tokio::test]
async fn test_daemon_start_with_sqlite_storage() {
    // Create a temp directory with minimal config
    let temp_dir = tempfile::tempdir().expect("tempdir");
    setup_agents_json(temp_dir.path()).expect("setup agents.json");

    // Start daemon
    let result = Daemon::start(temp_dir.path().to_str().unwrap()).await;
    if result.is_err() {
        panic!(
            "daemon should start successfully: {:?}",
            result.as_ref().err()
        );
    }

    let daemon = result.unwrap();
    // Verify storage was initialized and is functional
    assert!(daemon
        .storage
        .load_checkpoint("nonexistent_session")
        .await
        .unwrap()
        .is_none());
    // Verify sweeper shutdown sender exists
    assert!(!daemon.sweeper_shutdown_tx.is_closed());
    // Clean up
    drop(daemon);
    drop(temp_dir);
}

#[tokio::test]
async fn test_daemon_start_storage_failure() {
    // Use a path that cannot be created (not writable)
    let result = Daemon::start("/sys/cannot_create_storage_here").await;
    assert!(
        result.is_err(),
        "daemon should fail to start when SqliteStorage cannot be initialized"
    );
    let err_msg = if let Err(ref e) = result {
        e.to_string()
    } else {
        String::new()
    };
    assert!(
        err_msg.contains("SqliteStorage") || err_msg.contains("failed to initialize"),
        "error should mention SqliteStorage initialization failure: {err_msg}"
    );
}

#[tokio::test]
async fn test_daemon_start_missing_session_config() {
    // Create a temp dir with agents.json but NO session_config.json
    let temp_dir = tempfile::tempdir().expect("tempdir");
    setup_agents_json(temp_dir.path()).expect("setup agents.json");
    // Explicitly ensure session_config.json does NOT exist
    assert!(
        !temp_dir.path().join("session_config.json").exists(),
        "session_config.json should not exist for this test"
    );

    // Daemon should start with a WARN (not error/panic)
    let result = Daemon::start(temp_dir.path().to_str().unwrap()).await;
    if result.is_err() {
        panic!(
            "daemon should start even without session_config.json: {:?}",
            result.as_ref().err()
        );
    }

    drop(result);
    drop(temp_dir);
}

#[tokio::test]
async fn test_sweeper_shutdown_on_daemon_stop() {
    // Create a temp dir with minimal config
    let temp_dir = tempfile::tempdir().expect("tempdir");
    setup_agents_json(temp_dir.path()).expect("setup agents.json");

    // Start daemon
    let daemon = Daemon::start(temp_dir.path().to_str().unwrap())
        .await
        .expect("daemon should start");

    // Verify sweeper shutdown channel is open
    let is_closed_before = daemon.sweeper_shutdown_tx.is_closed();
    assert!(
        !is_closed_before,
        "sweeper shutdown channel should be open before shutdown"
    );

    // Send shutdown signal as Daemon::run() would
    let send_result = daemon.sweeper_shutdown_tx.send(());
    assert!(
        send_result.is_ok(),
        "shutdown signal should be sent successfully"
    );

    // After send, the receiver side should be notified (channel is not closed yet until drop)
    // Verify we can still send (channel not closed until last sender drops)
    let _ = daemon.sweeper_shutdown_tx.send(());

    // Drop the daemon (simulating end of life)
    drop(daemon);
    drop(temp_dir);
}

/// Helper: read all JSONL files in the audit directory and parse events.
fn read_audit_events(audit_dir: &std::path::PathBuf) -> Vec<crate::audit::AuditEvent> {
    let mut events = Vec::new();
    let Ok(entries) = std::fs::read_dir(audit_dir) else {
        return events;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in content.lines() {
            if let Ok(evt) = serde_json::from_str::<crate::audit::AuditEvent>(line.trim()) {
                events.push(evt);
            }
        }
    }
    events
}

/// Helper: build a bare permission request for the permission engine.
fn make_permission_request(agent: &str) -> crate::permission::PermissionRequest {
    crate::permission::PermissionRequest::Bare(
        crate::permission::PermissionRequestBody::CommandExec {
            agent: agent.to_string(),
            cmd: "test-cmd".to_string(),
            args: vec![],
        },
    )
}

#[tokio::test]
async fn test_audit_lifecycle() {
    // 1. Create temp directory for audit logs
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let audit_dir = temp_dir.path().to_path_buf();
    setup_agents_json(temp_dir.path()).expect("setup agents.json");

    // Create AuditLogger with temp directory
    let audit_logger = Arc::new(crate::audit::AuditLogger::with_base_dir(audit_dir.clone()));

    // 2. Start daemon with injected audit logger
    let daemon = crate::daemon::Daemon::start_with_audit_logger(
        temp_dir.path().to_str().unwrap(),
        audit_logger,
    )
    .await
    .expect("daemon should start");

    // Wait for the startup ConfigReload event to be flushed
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // 3. Verify ConfigReload event was written
    let events_after_start = read_audit_events(&audit_dir);
    let config_reload_events: Vec<_> = events_after_start
        .iter()
        .filter(|e| matches!(e.event_type, crate::audit::AuditEventType::ConfigReload))
        .collect();
    assert!(
        !config_reload_events.is_empty(),
        "ConfigReload event should exist after start"
    );
    // Verify details contain component: "daemon" and version
    let cfg = &config_reload_events[0];
    assert_eq!(
        cfg.details.get("component").and_then(|v| v.as_str()),
        Some("daemon")
    );
    assert!(
        cfg.details.get("version").is_some(),
        "ConfigReload details should contain version"
    );
    assert_eq!(cfg.result, crate::audit::AuditResult::Allow);

    // 4. Call evaluate_with_audit twice (Allowed and Denied)
    // Build a permission engine with a rule that allows test-agent but denies others
    let ruleset = crate::permission::RuleSet {
        version: "1.0.0".to_string(),
        rules: vec![crate::permission::Rule {
            name: "allow-test-agent".to_string(),
            subject: crate::permission::Subject::AgentOnly {
                agent: "test-agent".to_string(),
                match_type: crate::permission::MatchType::Exact,
            },
            effect: crate::permission::Effect::Allow,
            actions: vec![crate::permission::Action::Command {
                command: "test-cmd".to_string(),
                args: crate::permission::CommandArgs::Any,
            }],
            template: None,
            priority: 0,
        }],
        defaults: crate::permission::Defaults::default(),
        template_includes: vec![],
        agent_creators: std::collections::HashMap::new(),
    };
    let engine_with_rules = crate::permission::PermissionEngine::new(ruleset);

    // Restart daemon with the custom engine using the new test API
    let daemon = crate::daemon::Daemon::start_with_audit_logger_and_engine(
        temp_dir.path().to_str().unwrap(),
        Arc::new(crate::audit::AuditLogger::with_base_dir(audit_dir.clone())),
        Arc::new(engine_with_rules),
    )
    .await
    .expect("daemon should start with custom engine");

    // Wait for the startup ConfigReload event to be flushed
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Verify the second ConfigReload event
    let all_events_2 = read_audit_events(&audit_dir);
    let config_reload_events_2: Vec<_> = all_events_2
        .iter()
        .filter(|e| matches!(e.event_type, crate::audit::AuditEventType::ConfigReload))
        .collect();
    assert!(
        config_reload_events_2.len() >= 2,
        "Should have at least 2 ConfigReload events after restart, got {}",
        config_reload_events_2.len()
    );

    // Now do permission checks: Allowed (test-agent) and Denied (other-agent)
    let req_allowed = make_permission_request("test-agent");
    let resp_allowed = daemon.evaluate_with_audit(req_allowed).await;
    assert!(
        matches!(
            resp_allowed,
            crate::permission::PermissionResponse::Allowed { .. }
        ),
        "test-agent should be allowed: {:?}",
        resp_allowed
    );
    let req_denied = make_permission_request("other-agent");
    let resp_denied = daemon.evaluate_with_audit(req_denied).await;
    assert!(
        matches!(
            resp_denied,
            crate::permission::PermissionResponse::Denied { .. }
        ),
        "other-agent should be denied: {:?}",
        resp_denied
    );

    // Wait for async audit writes, then flush
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    daemon.shutdown_audit().await;

    // Verify PermissionCheck events: at least 2, with both Allow and Deny results
    let events_after_perm = read_audit_events(&audit_dir);
    let perm_check_events: Vec<_> = events_after_perm
        .iter()
        .filter(|e| matches!(e.event_type, crate::audit::AuditEventType::PermissionCheck))
        .collect();
    assert!(
        perm_check_events.len() >= 2,
        "Should have at least 2 PermissionCheck events, got {}",
        perm_check_events.len()
    );
    let has_allowed = perm_check_events
        .iter()
        .any(|e| e.result == crate::audit::AuditResult::Allow);
    let has_denied = perm_check_events
        .iter()
        .any(|e| e.result == crate::audit::AuditResult::Deny);
    assert!(has_allowed, "Should have an Allow PermissionCheck event");
    assert!(has_denied, "Should have a Deny PermissionCheck event");

    // 5. Log AgentStart, AgentStop, AgentError events
    daemon.log_agent_start("guide", "minimax/MiniMax-M2").await;
    daemon.log_agent_stop("guide").await;
    daemon.log_agent_error("guide", "test error message").await;
    // Wait for spawned tasks to complete, then flush
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    daemon.shutdown_audit().await;

    // Verify AgentStart, AgentStop, AgentError events
    let events_after_agent = read_audit_events(&audit_dir);
    let has_agent_start = events_after_agent
        .iter()
        .any(|e| matches!(e.event_type, crate::audit::AuditEventType::AgentStart));
    let has_agent_stop = events_after_agent
        .iter()
        .any(|e| matches!(e.event_type, crate::audit::AuditEventType::AgentStop));
    let has_agent_error = events_after_agent
        .iter()
        .any(|e| matches!(e.event_type, crate::audit::AuditEventType::AgentError));
    assert!(has_agent_start, "AgentStart event should exist");
    assert!(has_agent_stop, "AgentStop event should exist");
    assert!(has_agent_error, "AgentError event should exist");

    // Verify AgentStart result is Allow
    let agent_start_events: Vec<_> = events_after_agent
        .iter()
        .filter(|e| matches!(e.event_type, crate::audit::AuditEventType::AgentStart))
        .collect();
    assert_eq!(
        agent_start_events[0].result,
        crate::audit::AuditResult::Allow
    );

    // Verify AgentStop result is Allow
    let agent_stop_events: Vec<_> = events_after_agent
        .iter()
        .filter(|e| matches!(e.event_type, crate::audit::AuditEventType::AgentStop))
        .collect();
    assert_eq!(
        agent_stop_events[0].result,
        crate::audit::AuditResult::Allow
    );

    // Verify AgentError result is Error
    let agent_error_events: Vec<_> = events_after_agent
        .iter()
        .filter(|e| matches!(e.event_type, crate::audit::AuditEventType::AgentError))
        .collect();
    assert_eq!(
        agent_error_events[0].result,
        crate::audit::AuditResult::Error
    );

    // 6. Call shutdown_audit and verify all buffer events are flushed
    daemon.shutdown_audit().await;

    // After shutdown, event count should be same (all flushed, no new writes)
    let events_after_shutdown = read_audit_events(&audit_dir);
    assert_eq!(events_after_shutdown.len(), events_after_agent.len());

    drop(daemon);
    drop(temp_dir);
}
