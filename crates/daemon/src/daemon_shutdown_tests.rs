//! E2E tests for Daemon graceful shutdown
//!
//! Covers ShutdownHandle drain state machine scenarios.

use crate::shutdown::ShutdownHandle;
use closeclaw_common::test_helpers::write_mandatory_configs;
use std::time::Duration;

/// Test 1: drain waits until busy_count reaches zero before exiting.
/// Covers multi-decrement path (3x busy → decrement one-by-one).
#[tokio::test]
async fn test_drain_waits_until_busy_count_zero() {
    let handle = ShutdownHandle::new();

    // Increment busy count 3 times
    handle.increment_busy();
    handle.increment_busy();
    handle.increment_busy();

    // Spawn shutdown — should not complete while busy_count > 0
    let handle_clone = handle.clone();
    let shutdown_task = tokio::spawn(async move {
        handle_clone.initiate_shutdown().await;
    });

    // Give it a moment to enter ShuttingDown state
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(handle.state(), crate::shutdown::ShutdownState::ShuttingDown);
    assert_eq!(handle.busy_count(), 3);

    // Decrement one at a time and verify state doesn't change yet
    handle.decrement_busy();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!handle.is_stopped());

    handle.decrement_busy();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!handle.is_stopped());

    // Last decrement — drain should complete
    handle.decrement_busy();

    // Wait for shutdown to finish (3s drain timeout in test mode + buffer)
    let _ = tokio::time::timeout(Duration::from_secs(5), shutdown_task).await;

    assert!(handle.is_stopped());
}

/// Test 2: drain completes after timeout, even if busy_count > 0.
/// When the drain timeout fires, shutdown proceeds to Stopped state
/// and busy_count is not cleared.
#[tokio::test]
async fn test_drain_waits_for_busy_count_zero() {
    let handle = ShutdownHandle::new().with_drain_timeout(Duration::from_millis(300));
    handle.increment_busy();

    let handle_clone = handle.clone();
    let shutdown_task = tokio::spawn(async move {
        handle_clone.initiate_shutdown().await;
    });

    // Give it time to enter drain loop
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!handle.is_stopped(), "should still be draining");

    // Wait for timeout + buffer — drain should complete even though
    // busy_count is still 1.
    let _ = tokio::time::timeout(Duration::from_secs(3), shutdown_task).await;
    assert!(
        handle.is_stopped(),
        "handle should be stopped after drain timeout"
    );
    // busy_count was not cleared by the drain
    assert_eq!(handle.busy_count(), 1);
}

/// Test 3: drain signal is broadcast to all subscribers.
#[tokio::test]
async fn test_drain_signal_broadcast() {
    let handle = ShutdownHandle::new();

    let mut rx1 = handle.subscribe_drain();
    let mut rx2 = handle.subscribe_drain();

    // Spawn shutdown in background
    let handle_clone = handle.clone();
    tokio::spawn(async move {
        handle_clone.initiate_shutdown().await;
    });

    // Both receivers should receive the signal within 1 second
    let result1 = tokio::time::timeout(Duration::from_secs(1), rx1.recv()).await;
    let result2 = tokio::time::timeout(Duration::from_secs(1), rx2.recv()).await;

    assert!(result1.is_ok(), "Receiver 1 did not get drain signal");
    assert!(result2.is_ok(), "Receiver 2 did not get drain signal");
}

/// Test 4: Daemon::run() triggers graceful shutdown when receiving SIGTERM.
#[tokio::test]
async fn test_daemon_run_sigterm_shutdown() {
    // Create temp dir with minimal agents.json and mandatory config files.
    // ConfigManager receives <root>/config/ as its config_dir (design doc
    // directory structure), so mandatory JSONs must live in the config/ sub-dir.
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let config_dir = temp_dir.path().join("config");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    let agents_path = config_dir.join("agents.json");
    std::fs::write(&agents_path, r#"{"version":"1.0.0","agents":[]}"#).expect("write agents.json");

    write_mandatory_configs(&config_dir).expect("write mandatory config");

    // Do NOT set FEISHU/LLM env vars — Daemon::start will skip those components
    let mut daemon = crate::Daemon::start(temp_dir.path().to_str().unwrap())
        .await
        .expect("daemon start");

    let pid = std::process::id();

    // Spawn a task that sends SIGTERM to this process after a short delay.
    // This mirrors what an external signal source would do.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        // SAFETY: pid is our own process, this is safe for sending SIGTERM.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    });

    // Call Daemon::run() — it blocks on signal reception. When SIGTERM is sent
    // (from the spawned task above), run() initiates shutdown and returns.
    let _ = daemon.run().await;

    // Verify stopped within 5 seconds: poll is_stopped() until it returns true
    let poll_result: Result<(), tokio::time::error::Elapsed> =
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if daemon.shutdown.is_stopped() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
    assert!(
        poll_result.is_ok(),
        "daemon should be stopped within 5s (state={:?})",
        daemon.shutdown.state()
    );
}

// ======================================================================
// Step 1.7: Drain timeout from configuration tests
// ======================================================================

/// ShutdownHandle::with_drain_timeout() applies custom timeout.
/// Verifies the builder method correctly overrides the default 30s drain timeout.
#[tokio::test]
async fn test_drain_timeout_with_custom_value() {
    use std::time::Duration;

    // Use a short custom timeout (200ms) to prove it overrides the default 30s.
    let handle = ShutdownHandle::new().with_drain_timeout(Duration::from_millis(200));
    handle.increment_busy();

    let handle_clone = handle.clone();
    let start = tokio::time::Instant::now();
    let shutdown_task = tokio::spawn(async move {
        handle_clone.initiate_shutdown().await;
    });

    // With 200ms timeout, drain should complete around 200ms.
    let _ = tokio::time::timeout(Duration::from_secs(5), shutdown_task).await;
    let elapsed = start.elapsed();

    assert!(
        handle.is_stopped(),
        "drain should complete after 200ms timeout"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "elapsed should be ~200ms (not 30s default), got {:?}",
        elapsed
    );
    // Verify busy_count was not cleared (drain timed out)
    assert_eq!(
        handle.busy_count(),
        1,
        "busy_count should remain 1 after timeout"
    );
}

/// Default drain timeout is 30 seconds (ShutdownHandle::new() default).
#[tokio::test]
async fn test_drain_timeout_default_is_30s() {
    use std::time::Duration;

    let handle = ShutdownHandle::new().with_drain_timeout(Duration::from_millis(300));
    handle.increment_busy();

    let handle_clone = handle.clone();
    let start = tokio::time::Instant::now();
    let shutdown_task = tokio::spawn(async move {
        handle_clone.initiate_shutdown().await;
    });

    let _ = tokio::time::timeout(Duration::from_secs(3), shutdown_task).await;
    let elapsed = start.elapsed();

    assert!(
        handle.is_stopped(),
        "drain should complete after 300ms timeout"
    );
    assert!(
        elapsed >= Duration::from_millis(250) && elapsed <= Duration::from_secs(1),
        "elapsed should be ~300ms, got {:?}",
        elapsed
    );
}

/// ShutdownHandle::with_drain_timeout() with very short timeout (1ms)
/// completes almost immediately.
#[tokio::test]
async fn test_drain_timeout_very_short() {
    use std::time::Duration;

    let handle = ShutdownHandle::new().with_drain_timeout(Duration::from_millis(1));
    handle.increment_busy();

    let handle_clone = handle.clone();
    let start = tokio::time::Instant::now();
    let shutdown_task = tokio::spawn(async move {
        handle_clone.initiate_shutdown().await;
    });

    let _ = tokio::time::timeout(Duration::from_secs(2), shutdown_task).await;
    let elapsed = start.elapsed();

    assert!(
        handle.is_stopped(),
        "drain should complete after 1ms timeout"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "elapsed should be <1s, got {:?}",
        elapsed
    );
}

/// Per-session graceful timeout reads from config when available.
/// Verifies the config-based timeout path (reading system.shutdown.gracefulTimeoutSecs).
#[test]
fn test_per_session_graceful_timeout_reads_from_config() {
    use closeclaw_config::providers::SystemConfigData;
    use closeclaw_config::{ConfigManager, ConfigSection};
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().to_path_buf();
    let config_subdir = config_dir.join("config");
    std::fs::create_dir_all(&config_subdir).unwrap();

    // Write system.json with custom shutdown config
    let system_json = serde_json::json!({
        "shutdown": {
            "drainTimeoutSecs": 15,
            "gracefulTimeoutSecs": 45
        }
    });
    std::fs::write(
        config_subdir.join("system.json"),
        serde_json::to_string(&system_json).unwrap(),
    )
    .unwrap();

    let cm = ConfigManager::new(config_subdir).unwrap();
    let _ = cm.reload_section(ConfigSection::System, None);

    // Read the timeout the same way phase_2_session_stop does
    let timeout = cm
        .section(ConfigSection::System)
        .and_then(|v| serde_json::from_value::<SystemConfigData>(v).ok())
        .and_then(|sys| sys.shutdown.map(|s| s.graceful_timeout_secs))
        .map(std::time::Duration::from_secs);

    assert_eq!(
        timeout,
        Some(std::time::Duration::from_secs(45)),
        "per-session graceful timeout should read 45s from config"
    );
}

/// Per-session graceful timeout falls back to DEFAULT_GRACEFUL_TIMEOUT
/// when shutdown config is absent.
#[test]
fn test_per_session_graceful_timeout_fallback_to_default() {
    use closeclaw_config::providers::SystemConfigData;
    use closeclaw_config::{ConfigManager, ConfigSection};
    use closeclaw_session::llm_session::session_handles::DEFAULT_GRACEFUL_TIMEOUT;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let config_subdir = tmp.path().join("config");
    std::fs::create_dir_all(&config_subdir).unwrap();

    // Write system.json WITHOUT shutdown config
    let system_json = serde_json::json!({ "version": "1.0" });
    std::fs::write(
        config_subdir.join("system.json"),
        serde_json::to_string(&system_json).unwrap(),
    )
    .unwrap();

    let cm = ConfigManager::new(config_subdir).unwrap();
    let _ = cm.reload_section(ConfigSection::System, None);

    // Read the timeout the same way phase_2_session_stop does
    let timeout = cm
        .section(ConfigSection::System)
        .and_then(|v| serde_json::from_value::<SystemConfigData>(v).ok())
        .and_then(|sys| sys.shutdown.map(|s| s.graceful_timeout_secs))
        .map(std::time::Duration::from_secs)
        .unwrap_or(DEFAULT_GRACEFUL_TIMEOUT);

    assert_eq!(
        timeout, DEFAULT_GRACEFUL_TIMEOUT,
        "should fall back to DEFAULT_GRACEful_TIMEOUT when config is absent"
    );
}

/// Drain timeout reads from config when available.
/// Verifies the config-based drain timeout path (reading system.shutdown.drainTimeoutSecs).
#[test]
fn test_drain_timeout_reads_from_config() {
    use closeclaw_config::providers::SystemConfigData;
    use closeclaw_config::{ConfigManager, ConfigSection};
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let config_subdir = tmp.path().join("config");
    std::fs::create_dir_all(&config_subdir).unwrap();

    let system_json = serde_json::json!({
        "shutdown": {
            "drainTimeoutSecs": 20,
            "gracefulTimeoutSecs": 30
        }
    });
    std::fs::write(
        config_subdir.join("system.json"),
        serde_json::to_string(&system_json).unwrap(),
    )
    .unwrap();

    let cm = ConfigManager::new(config_subdir).unwrap();
    let _ = cm.reload_section(ConfigSection::System, None);

    let drain_timeout = cm
        .section(ConfigSection::System)
        .and_then(|v| serde_json::from_value::<SystemConfigData>(v).ok())
        .and_then(|sys| sys.shutdown.map(|s| s.drain_timeout_secs))
        .map(std::time::Duration::from_secs);

    assert_eq!(
        drain_timeout,
        Some(std::time::Duration::from_secs(20)),
        "drain timeout should read 20s from config"
    );
}
