//! SIGTERM/SIGINT graceful shutdown tests — two variants:
//!
//! 1. Out-of-process: `closeclaw run --foreground` spawned as a child
//!    process, signal delivered to the child (real binary composition).
//! 2. In-process: `Daemon::start()` full runtime (real Unix sockets) in
//!    this process, signal delivered to the current process itself —
//!    migrated from the daemon crate's `daemon_shutdown_tests.rs`
//!    (issue #3244).
//!
//! Both variants verify that SIGTERM/SIGINT triggers graceful shutdown
//! instead of hard-killing the daemon.

use closeclaw_common::test_helpers::write_mandatory_configs;
use std::time::Duration;
use tokio::time::timeout;

use super::helpers;

/// Verifies that SIGTERM triggers graceful shutdown (not hard kill).
/// The daemon should exit with code 0 after drain timeout.
#[tokio::test]
#[cfg(unix)]
async fn test_sigterm_triggers_graceful_shutdown() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_dir = temp_dir.path();

    let agents_dir = config_dir.join("config");
    std::fs::create_dir_all(&agents_dir).expect("create config dir");
    std::fs::write(
        agents_dir.join("agents.json"),
        r#"{"version":"1.0.0","agents":[]}"#,
    )
    .expect("failed to write test agents.json");
    write_mandatory_configs(&agents_dir).expect("write mandatory config");

    // Spawn daemon with HOME isolation
    let mut daemon = helpers::spawn_daemon(config_dir);

    // Wait for daemon admin socket to be ready
    helpers::wait_for_daemon_ready(config_dir).await;

    // Verify daemon is still running (not crashed on startup)
    helpers::assert_daemon_alive(&mut daemon);

    let pid = daemon.id().expect("daemon has a PID");

    // Send SIGTERM
    // SAFETY: `pid` is the PID of the daemon child we spawned above and
    // verified is still running; the cast to `libc::pid_t` is a lossless
    // widening conversion, and SIGTERM is a valid signal number.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }

    // Wait for daemon to exit (drain timeout is 30s, add buffer)
    let result = timeout(Duration::from_secs(35), daemon.wait())
        .await
        .expect("daemon should exit within 35s");

    let status = result.expect("daemon should exit");

    assert!(
        status.success(),
        "daemon should exit successfully after graceful shutdown, got: {:?}",
        status
    );
}

/// Verifies that SIGINT (Ctrl+C) also triggers graceful shutdown.
#[tokio::test]
#[cfg(unix)]
async fn test_sigint_triggers_graceful_shutdown() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_dir = temp_dir.path();

    let agents_dir = config_dir.join("config");
    std::fs::create_dir_all(&agents_dir).expect("create config dir");
    std::fs::write(
        agents_dir.join("agents.json"),
        r#"{"version":"1.0.0","agents":[]}"#,
    )
    .expect("failed to write test agents.json");
    write_mandatory_configs(&agents_dir).expect("write mandatory config");

    // Spawn daemon with HOME isolation
    let mut daemon = helpers::spawn_daemon(config_dir);

    // Wait for daemon admin socket to be ready
    helpers::wait_for_daemon_ready(config_dir).await;

    // Verify daemon is still running
    helpers::assert_daemon_alive(&mut daemon);

    let pid = daemon.id().expect("daemon has a PID");

    // Send SIGINT
    // SAFETY: `pid` is the PID of the daemon child we spawned above and
    // verified is still running; the cast to `libc::pid_t` is a lossless
    // widening conversion, and SIGINT is a valid signal number.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGINT);
    }

    let result = timeout(Duration::from_secs(35), daemon.wait())
        .await
        .expect("daemon should exit within 35s");

    let status = result.expect("daemon should exit");

    assert!(
        status.success(),
        "daemon should exit successfully after graceful shutdown, got: {:?}",
        status
    );
}

// ======================================================================
// In-process variant: Daemon::start() full runtime + SIGTERM to self
// (migrated from crates/daemon/src/daemon_shutdown_tests.rs, issue #3244)
// ======================================================================

/// Builds the temp config tree for `Daemon::start`: `<root>/config/` holds
/// `agents.json` plus all mandatory configs — ConfigManager receives
/// `<root>/config/` as its config_dir (design-doc directory structure).
/// Inlined from the daemon crate's `daemon_test_temp_config` (cfg(test)
/// private, not reachable across crates).
fn daemon_test_temp_config() -> tempfile::TempDir {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let config_dir = temp_dir.path().join("config");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agents.json"),
        r#"{"version":"1.0.0","agents":[]}"#,
    )
    .expect("write agents.json");
    write_mandatory_configs(&config_dir).expect("write mandatory config");
    temp_dir
}

/// Test: `Daemon::run()` triggers graceful shutdown when receiving SIGTERM.
///
/// In-process variant of the SIGTERM scenario: `Daemon::start()` boots the
/// full runtime (real Unix sockets) inside this process, then SIGTERM is
/// delivered to the current process itself.
#[serial_test::serial]
#[tokio::test(flavor = "current_thread")]
async fn test_daemon_run_sigterm_shutdown() {
    let temp_dir = daemon_test_temp_config();

    // Do NOT set FEISHU/LLM env vars — Daemon::start will skip those components
    let mut daemon = closeclaw_daemon::Daemon::start(temp_dir.path().to_str().unwrap())
        .await
        .expect("daemon start");

    // Spawn a task that sends SIGTERM to this process — mirrors an external
    // signal source. Deterministic ordering (no sleep gamble): the explicit
    // `flavor = "current_thread"` test attribute pins a single-threaded runtime,
    // so this task cannot run before the main task's first yield — the next
    // await is run(), whose first poll synchronously registers the signal
    // handler before parking in Phase 0.
    let kill_task = tokio::spawn(async {
        // SAFETY: the target pid is `std::process::id()`, i.e. this process
        // itself, so the signal is delivered only to the calling process and
        // never to another one; SIGTERM is one of the graceful shutdown
        // signals, so the delivered signal's effect on this process is
        // exactly what the surrounding test exercises.
        let ret = unsafe { libc::kill(std::process::id() as libc::pid_t, libc::SIGTERM) };
        assert_eq!(
            ret,
            0,
            "kill self failed: {}",
            std::io::Error::last_os_error()
        );
    });

    // Call Daemon::run() — it blocks on signal reception. When SIGTERM is sent
    // (from the spawned task above), run() initiates shutdown and returns.
    let _ = daemon.run().await;

    // Join the kill task so its `ret == 0` assertion cannot be silently
    // swallowed: as a detached task, a panic inside it would go unnoticed.
    kill_task
        .await
        .expect("kill task should complete normally (SIGTERM delivered)");

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
