//! Daemon unit tests.

use std::sync::Arc;

// Re-export items needed for tests
use crate::daemon::Daemon;
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
