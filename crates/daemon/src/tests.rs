//! Daemon unit tests.

use super::*;
use crate::test_helpers::write_mandatory_configs;
use closeclaw_session::persistence::PersistenceService;

/// Create only `config/agents.json` (without mandatory config files)
/// in the given directory.
fn write_agents_json(dir: &std::path::Path) -> std::io::Result<()> {
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
    let config_dir = dir.join("config");
    std::fs::create_dir_all(&config_dir)?;
    std::fs::write(config_dir.join("agents.json"), agents_content.to_string())?;
    Ok(())
}

/// Create a minimal agents.json and all 5 mandatory config files
/// (models.json, channels.json, gateway.json, plugins.json, system.json)
/// in the given directory so that `ConfigManager::load()` succeeds.
fn setup_agents_json(dir: &std::path::Path) -> std::io::Result<()> {
    write_agents_json(dir)?;
    // Mandatory configs go into the config/ subdirectory
    // (ConfigManager now receives <root>/config/ as config_dir)
    let config_dir = dir.join("config");
    write_mandatory_configs(&config_dir)?;
    Ok(())
}

// =====================================================================
// Step 1.2 — daemon startup integration: load() gating tests
// =====================================================================

/// Test: daemon fails to start when mandatory config files are missing.
/// Step 1.1 added `config_manager.load()` before hot-reload registration;
/// a missing mandatory file (e.g. models.json) must cause daemon startup to
/// fail with an error mentioning "mandatory config sections".
#[tokio::test]
async fn test_daemon_start_fails_without_mandatory_config() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    // Create only agents.json — mandatory sections (models.json etc.) are absent
    write_agents_json(temp_dir.path()).unwrap();

    let result = Daemon::start(temp_dir.path().to_str().unwrap()).await;
    assert!(
        result.is_err(),
        "daemon should fail when mandatory config files are missing"
    );
    let err_msg = match result {
        Err(e) => e.to_string(),
        _ => unreachable!(),
    };
    assert!(
        err_msg.contains("mandatory"),
        "error should mention mandatory: {err_msg}"
    );
}

/// Test: daemon fails to start when config directory has no config files
/// at all (empty directory).
#[tokio::test]
async fn test_daemon_start_fails_with_empty_config_dir() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    // Empty directory — no config files, no agents.json

    let result = Daemon::start(temp_dir.path().to_str().unwrap()).await;
    assert!(result.is_err(), "daemon should fail with empty config dir");
    let err_msg = match result {
        Err(e) => e.to_string(),
        _ => unreachable!(),
    };
    assert!(
        err_msg.contains("mandatory"),
        "error should mention mandatory: {err_msg}"
    );
}

/// Test: daemon starts successfully when all mandatory config files exist.
/// This verifies the happy path: load() populates sections, then hot-reload
/// is registered (gate passes).
#[tokio::test]
async fn test_daemon_start_succeeds_with_all_mandatory_configs() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    setup_agents_json(temp_dir.path()).expect("setup agents.json");

    let result = Daemon::start(temp_dir.path().to_str().unwrap()).await;
    assert!(
        result.is_ok(),
        "daemon should start with all mandatory configs: {:?}",
        result.err()
    );
    drop(result);
    drop(temp_dir);
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
        err_msg.contains("Permission denied")
            || err_msg.contains("SqliteStorage")
            || err_msg.contains("ConfigManager")
            || err_msg.contains("failed to initialize"),
        "error should mention initialization failure (storage or config): {err_msg}"
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
