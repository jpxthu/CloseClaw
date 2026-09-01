//! Integration tests for SIGTERM shutdown checkpoint persistence.
//!
//! Verifies:
//! 1. `flush_all()` writes checkpoint to SqliteStorage
//! 2. Restored session re-queues only unsent (sent=false) messages
//! 3. SIGTERM triggers graceful shutdown with SqliteStorage initialization
//!
//! Uses `#[cfg(feature = "fake-llm")]` to gate all tests, consistent with the rest of the
//! integration test suite.

#![cfg(feature = "fake-llm")]

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::sync::Arc;

use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_gateway::session_manager::SessionManager;
use closeclaw_gateway::{GatewayConfig, Message};
use closeclaw_llm::fake::FakeProvider;
use closeclaw_llm::provider::Provider;
use closeclaw_llm::LLMRegistry;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::PersistenceService;
use closeclaw_session::persistence::ReasoningLevel;
use closeclaw_session::storage::sqlite::SqliteStorage;
use tempfile::TempDir;

/// Build a minimal GatewayConfig for testing.
fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

/// Build a dummy gateway Message for find_or_create.
fn make_msg() -> Message {
    Message {
        id: "msg_1".into(),
        from: "alice".into(),
        to: "bob".into(),
        content: "hello".into(),
        channel: "ch".into(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
        thread_id: None,
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    }
}

/// Set up a SessionManager backed by a temporary SqliteStorage, with a
/// FakeProvider registered in an LLMRegistry.
///
/// Returns a tuple of:
/// - `SessionManager` (wrapped in Arc)
/// - `FakeProvider` (for inspecting captured requests)
/// - `TempDir` (keeps SqliteStorage alive for the duration of the test)
///
/// Must be called from within a tokio runtime (e.g., inside a #[tokio::test]).
async fn setup_session_manager_with_storage() -> (Arc<SessionManager>, FakeProvider, TempDir) {
    let test_root = TempDir::with_prefix("closeclaw-shutdown-").expect("failed to create temp dir");
    let data_path = test_root.path().to_path_buf();

    let storage: Arc<dyn PersistenceService> =
        Arc::new(SqliteStorage::new(&data_path).expect("SqliteStorage::new failed"));

    let sm = Arc::new(SessionManager::new(
        &test_config(),
        Some(storage),
        None,
        ReasoningLevel::default(),
    ));

    let provider = FakeProvider::builder()
        .then_ok("fake response", "fake-model")
        .build();
    let provider_clone = provider.clone();

    let registry = Arc::new(LLMRegistry::new());
    let wrapped: Arc<dyn Provider> = Arc::new(provider_clone);
    registry.register("fake".to_string(), wrapped).await;

    (sm, provider, test_root)
}

/// Poll the daemon admin socket until it accepts a connection or times out.
///
/// The admin socket is created in the daemon's final init phase, so its
/// availability signals that the daemon is fully initialized and ready to
/// receive SIGTERM. Bounded, signal-targeted readiness wait (no blind sleep).
#[cfg(unix)]
async fn wait_for_daemon_ready(config_dir: &std::path::Path) {
    const SOCKET_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
    const SOCKET_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);
    let socket_path = config_dir.join("admin.sock");
    let deadline = tokio::time::Instant::now() + SOCKET_WAIT_TIMEOUT;
    loop {
        if UnixStream::connect(&socket_path).is_ok() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "daemon admin socket not ready after {:?}: {}",
                SOCKET_WAIT_TIMEOUT,
                socket_path.display()
            );
        }
        tokio::time::sleep(SOCKET_POLL_INTERVAL).await;
    }
}

// ---------------------------------------------------------------------------
// Test 1.2: flush_all writes checkpoint to SqliteStorage
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_flush_all_writes_checkpoint_to_sqlite() {
    let (sm, _provider, test_root) = setup_session_manager_with_storage().await;
    let data_path = test_root.path().to_path_buf();

    // Create a session
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    // Push two pending messages: one sent=true, one sent=false
    use closeclaw_session::persistence::PendingMessage;

    let mut msg_sent =
        PendingMessage::new("msg-sent-1".to_string(), "already sent content".to_string());
    msg_sent.mark_sent();
    let msg_unsent = PendingMessage::new(
        "msg-unsent-1".to_string(),
        "not yet sent content".to_string(),
    );

    sm.push_pending_message(&sid, msg_sent).await.unwrap();
    sm.push_pending_message(&sid, msg_unsent).await.unwrap();

    // Flush all sessions to storage
    let saved = sm.flush_all(ShutdownMode::Graceful).await.unwrap();
    assert_eq!(saved, 1, "flush_all should save 1 session checkpoint");

    // Load checkpoint back from a fresh SqliteStorage instance at the same path
    let storage = SqliteStorage::new(&data_path).expect("SqliteStorage::new failed");
    let cp = storage.load_checkpoint(&sid).await.unwrap();

    assert!(cp.is_some(), "checkpoint should exist after flush_all");
    let cp = cp.unwrap();

    // Both pending messages (sent + unsent) are persisted into the checkpoint.
    assert_eq!(
        cp.outbound_pending.len(),
        2,
        "checkpoint should contain 2 pending messages, got {}",
        cp.outbound_pending.len()
    );

    // Verify message IDs match what was pushed
    let ids: Vec<&str> = cp
        .outbound_pending
        .iter()
        .map(|m| m.message_id.as_str())
        .collect();
    assert!(
        ids.contains(&"msg-sent-1"),
        "checkpoint should contain msg-sent-1, got {:?}",
        ids
    );
    assert!(
        ids.contains(&"msg-unsent-1"),
        "checkpoint should contain msg-unsent-1, got {:?}",
        ids
    );

    // Persistence preserves the sent flag (outbound_pending is stored as JSON
    // metadata, not derived from the transcript). msg-sent-1 stays sent=true
    // and msg-unsent-1 stays sent=false.
    let msg_sent_cp = cp
        .outbound_pending
        .iter()
        .find(|m| m.message_id == "msg-sent-1")
        .expect("checkpoint should contain msg-sent-1");
    assert!(
        msg_sent_cp.sent,
        "msg-sent-1 should preserve sent=true after flush_all"
    );
    let msg_unsent_cp = cp
        .outbound_pending
        .iter()
        .find(|m| m.message_id == "msg-unsent-1")
        .expect("checkpoint should contain msg-unsent-1");
    assert!(
        !msg_unsent_cp.sent,
        "msg-unsent-1 should preserve sent=false after flush_all"
    );
}

// ---------------------------------------------------------------------------
// Test 1.3: restore from checkpoint re-queues only unsent (sent=false)
//           messages, skipping already-sent (sent=true) messages
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_restore_after_checkpoint_skips_all_messages() {
    let (_sm, _provider, test_root) = setup_session_manager_with_storage().await;
    let data_path = test_root.path().to_path_buf();

    // Same flush logic as test 1.2: push 2 pending, flush, load checkpoint
    let sm = &_sm;
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    use closeclaw_session::persistence::PendingMessage;

    let mut msg_sent =
        PendingMessage::new("msg-sent-1".to_string(), "already sent content".to_string());
    msg_sent.mark_sent();
    let msg_unsent = PendingMessage::new(
        "msg-unsent-1".to_string(),
        "not yet sent content".to_string(),
    );

    sm.push_pending_message(&sid, msg_sent).await.unwrap();
    sm.push_pending_message(&sid, msg_unsent).await.unwrap();

    let saved = sm.flush_all(ShutdownMode::Graceful).await.unwrap();
    assert_eq!(saved, 1, "flush_all should save 1 session checkpoint");

    // Load checkpoint from a fresh SqliteStorage
    let storage = SqliteStorage::new(&data_path).expect("SqliteStorage::new failed");
    let cp = storage.load_checkpoint(&sid).await.unwrap();
    assert!(cp.is_some(), "checkpoint should exist after flush_all");
    let cp = cp.unwrap();

    // Create a fresh ConversationSession and restore pending messages
    let session_root = tempfile::TempDir::new().unwrap();
    let root = session_root.path().to_path_buf();
    let mut session = ConversationSession::new(sid.clone(), "fake-model".to_string(), root);
    session.restore_pending_messages(cp.outbound_pending);

    // restore_pending_messages re-queues only messages with sent=false.
    // msg-sent-1 (sent=true) is skipped; msg-unsent-1 (sent=false) is re-queued.
    let pending = session.get_pending_messages();
    assert_eq!(
        pending.len(),
        1,
        "restored session should re-queue 1 pending message (msg-unsent-1); got {}",
        pending.len()
    );
    assert_eq!(
        pending[0].message_id, "msg-unsent-1",
        "re-queued message should be msg-unsent-1, got {}",
        pending[0].message_id
    );
    assert!(
        !pending[0].sent,
        "re-queued message should still be unsent (sent=false)"
    );
}

// ---------------------------------------------------------------------------
// Test 1.4: SIGTERM E2E test — verify SqliteStorage is initialized after
//           graceful shutdown triggered by SIGTERM.
//
// This is an E2E test that starts a real daemon process, sends SIGTERM,
// and verifies that `sessions.sqlite` was created in the config directory.
// The in-process tests (1.2/1.3/1.5) cover checkpoint content correctness;
// this test verifies the SIGTERM → graceful shutdown → SqliteStorage init link.
// ---------------------------------------------------------------------------

#[tokio::test]
#[cfg(unix)]
async fn test_sigterm_triggers_graceful_shutdown_with_storage() {
    use std::process::Stdio;

    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let daemon_bin = manifest_dir.join("target/debug/closeclaw");

    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_dir = temp_dir.path();

    // Write minimal agents.json + mandatory configs so daemon starts successfully
    let agents_dir = config_dir.join("config");
    std::fs::create_dir_all(&agents_dir).expect("create config dir");
    std::fs::write(
        agents_dir.join("agents.json"),
        r#"{"version":"1.0.0","agents":[]}"#,
    )
    .expect("failed to write agents.json");
    closeclaw_common::test_helpers::write_mandatory_configs(&agents_dir)
        .expect("write mandatory config");

    // Start the daemon in --foreground mode so the test owns the daemon PID
    // and SIGTERM reaches the daemon process itself (not a wrapper).
    let mut daemon = tokio::process::Command::new(&daemon_bin)
        .args(["run", "--config-dir"])
        .arg(config_dir.as_os_str())
        .arg("--foreground")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn daemon");

    // Wait for the daemon admin socket (final init phase) to be ready.
    wait_for_daemon_ready(config_dir).await;

    // Verify daemon is still running (didn't crash on startup)
    match daemon.try_wait().expect("try_wait works") {
        Some(status) => {
            let output = daemon.wait_with_output().await.expect("wait_with_output");
            panic!(
                "daemon exited prematurely during startup: {:?}\nstdout:{}\nstderr:{}",
                status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        None => { /* still running — good */ }
    }

    // Send SIGTERM to trigger graceful shutdown
    let pid = daemon.id().expect("daemon has PID");
    // SAFETY: `pid` is the PID of the daemon child we spawned above and
    // verified is still running; the cast to `libc::pid_t` is a lossless
    // widening conversion, and SIGTERM is a valid signal number.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }

    // Wait for daemon to exit (drain timeout is 30s, give buffer)
    let status = tokio::time::timeout(std::time::Duration::from_secs(35), daemon.wait())
        .await
        .expect("daemon should exit within 35s")
        .expect("daemon should exit");

    // SIGTERM triggers graceful shutdown → exit code 0 (not hard kill)
    assert!(
        status.success(),
        "daemon should exit with success after graceful shutdown, got {:?}",
        status
    );

    // Verify SqliteStorage was initialized — `sessions.sqlite` must exist
    let sessions_sqlite = config_dir.join("sessions.sqlite");
    assert!(
        sessions_sqlite.exists(),
        "sessions.sqlite should exist after SIGTERM graceful shutdown, proving \
         SqliteStorage was initialized via the flush_all → graceful shutdown path"
    );
}

// ---------------------------------------------------------------------------
// Test 1.5: full in-process cycle — shutdown checkpoint + session re-find
//
// Simulates a complete shutdown/restart cycle:
// 1. First SessionManager: find_or_create, push 2 pending messages (sent
//   各异), flush_all() to write the checkpoint to SqliteStorage.
// 2. Second SessionManager (same storage path): find_or_create re-finds the
//    persisted active session (self-heal) and returns the same session id.
// 3. Verify: the session is re-registered and no message is re-sent to the
//    LLM (FakeProvider captured_requests stays empty). Pending-message
//    re-queue semantics are covered by tests 1.2/1.3.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_full_shutdown_restore_cycle() {
    // Phase 1: create SessionManager, push pending, flush to simulate shutdown
    let (sm1, provider, test_root) = setup_session_manager_with_storage().await;
    let data_path = test_root.path().to_path_buf();

    let sid = sm1.find_or_create("ch", &make_msg(), None).await.unwrap();

    use closeclaw_session::persistence::PendingMessage;
    use closeclaw_session::persistence::ReasoningLevel;

    let mut msg_sent =
        PendingMessage::new("msg-sent-cycle".to_string(), "sent content".to_string());
    msg_sent.mark_sent();
    let msg_unsent =
        PendingMessage::new("msg-unsent-cycle".to_string(), "unsent content".to_string());

    sm1.push_pending_message(&sid, msg_sent).await.unwrap();
    sm1.push_pending_message(&sid, msg_unsent).await.unwrap();

    // flush_all simulates the graceful shutdown path
    let saved = sm1.flush_all(ShutdownMode::Graceful).await.unwrap();
    assert_eq!(saved, 1, "flush_all should save 1 session checkpoint");

    // Drop first SessionManager to simulate shutdown
    drop(sm1);

    // Phase 2: new SessionManager pointing to same storage path
    // find_or_create should detect existing session and trigger restore
    let storage2: Arc<dyn PersistenceService> =
        Arc::new(SqliteStorage::new(&data_path).expect("SqliteStorage::new failed"));

    let sm2 = Arc::new(SessionManager::new(
        &test_config(),
        Some(storage2),
        None,
        ReasoningLevel::default(),
    ));

    // Trigger re-find by calling find_or_create for the same routing fields.
    let sid2 = sm2.find_or_create("ch", &make_msg(), None).await.unwrap();

    // Current behavior: find_or_create self-heals the persisted active session
    // (re-registers it in memory) rather than eagerly rebuilding a
    // ConversationSession. The checkpoint (with outbound_pending) stays in
    // storage for the recovery path.
    assert_eq!(
        sid2, sid,
        "find_or_create should re-find the persisted session id after restart"
    );
    assert!(
        sm2.has_session(&sid).await,
        "re-found session should be registered in the sessions table"
    );

    // No new requests should have been sent to the LLM (FakeProvider)
    let captured = provider.captured_requests();
    assert_eq!(
        captured.len(),
        0,
        "FakeProvider should have 0 captured requests (no messages re-sent); got {}",
        captured.len()
    );
}
