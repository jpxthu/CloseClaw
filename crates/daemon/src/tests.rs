//! Daemon unit tests.

use super::*;
use crate::test_helpers::write_mandatory_configs;
use closeclaw_common::test_helpers::write_mandatory_without_models;
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

/// Create a minimal agents.json, the 5 mandatory config files
/// (channels.json, gateway.json, plugins.json, system.json,
/// accounts.json) and the optional models.json in the given directory
/// so that `ConfigManager::load()` succeeds.
fn setup_agents_json(dir: &std::path::Path) -> std::io::Result<()> {
    write_agents_json(dir)?;
    // Mandatory configs go into the config/ subdirectory
    // (ConfigManager now receives <root>/config/ as config_dir)
    let config_dir = dir.join("config");
    write_mandatory_configs(&config_dir)?;
    Ok(())
}

/// Same as [`setup_agents_json`] but without models.json — the Models
/// section is optional and must not gate startup (design doc daemon
/// README: 「models.json 缺失…系统仍正常启动」).
fn setup_agents_json_without_models(dir: &std::path::Path) -> std::io::Result<()> {
    write_agents_json(dir)?;
    let config_dir = dir.join("config");
    write_mandatory_without_models(&config_dir)?;
    Ok(())
}

// =====================================================================
// Step 1.2 — daemon startup integration: load() gating tests
// =====================================================================

/// Test: daemon fails to start when mandatory config files are missing.
/// Step 1.1 added `config_manager.load()` before hot-reload registration;
/// a missing mandatory file (e.g. channels.json — models.json is optional
/// and does not gate startup) must cause daemon startup to fail with an
/// error mentioning "mandatory config sections".
#[tokio::test]
async fn test_daemon_start_fails_without_mandatory_config() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    // Create only agents.json — mandatory sections (channels.json etc.) are absent
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

/// Test: daemon starts successfully WITHOUT models.json — system-level
/// gating (Step 1.17): the Models section is optional at load, the LLM
/// registry builds an empty fallback chain, and startup completes through
/// all phases without panicking. Design doc `docs/design/daemon/README.md`
/// 「LLM 能力缺失时的行为」: models.json 缺失 → 系统仍正常启动.
#[tokio::test]
async fn test_daemon_start_succeeds_without_models_json() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    setup_agents_json_without_models(temp_dir.path()).expect("configs without models.json");

    let daemon = Daemon::start(temp_dir.path().to_str().unwrap())
        .await
        .expect("daemon must start with models.json missing");
    assert!(
        daemon.llm_registry.list().await.is_empty(),
        "no provider registered without models.json"
    );
    assert_eq!(
        daemon.fallback_client.chain().len(),
        0,
        "LLM fallback chain must be empty"
    );
}

/// Test: daemon refuses to start when models.json is corrupt and no
/// backup exists — F3 (design doc config README 启动加载 step 1 +
/// requirements config §F3): recovery failure (no usable backup) →
/// refuse startup, not a silent empty LLM config.
#[tokio::test]
async fn test_daemon_start_fails_with_corrupt_models_json_no_backup() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    setup_agents_json_without_models(temp_dir.path()).expect("configs without models.json");
    // Corrupt models.json — no backup has ever been written.
    std::fs::write(
        temp_dir.path().join("config").join("models.json"),
        "not valid json {{",
    )
    .unwrap();

    let result = Daemon::start(temp_dir.path().to_str().unwrap()).await;
    let err = result
        .err()
        .expect("corrupt models.json without backup must refuse startup");
    let msg = err.to_string();
    assert!(
        msg.contains("models.json"),
        "error must name the corrupt file: {msg}"
    );
}

/// Test: corrupt models.json with a usable backup → F3 rollback →
/// startup succeeds with the restored file and an empty LLM chain.
#[tokio::test]
async fn test_daemon_start_recovers_corrupt_models_json_from_backup() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    setup_agents_json(temp_dir.path()).expect("setup agents.json");
    let config_dir = temp_dir.path().join("config");
    // Create a real backup of models.json via the config write path.
    let cm = ConfigManager::new(config_dir.clone()).unwrap();
    cm.update(
        ConfigSection::Models,
        serde_json::json!({"version": "2.0"}),
        |_| Ok(()),
    )
    .expect("update must back up the current models.json");
    // Corrupt models.json — rollback must restore the backed-up content.
    std::fs::write(config_dir.join("models.json"), "not valid json {{").unwrap();

    let daemon = Daemon::start(temp_dir.path().to_str().unwrap())
        .await
        .expect("daemon must start via backup rollback");
    assert!(
        daemon.llm_registry.list().await.is_empty(),
        "restored placeholder defines no provider"
    );
    assert_eq!(
        daemon.fallback_client.chain().len(),
        0,
        "LLM fallback chain must be empty"
    );
    let restored = std::fs::read_to_string(config_dir.join("models.json")).unwrap();
    assert!(
        restored.contains("1.0"),
        "rollback restored the backup content: {restored}"
    );
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
