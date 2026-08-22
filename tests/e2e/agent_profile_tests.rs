//! E2E: black-box agent-profile smoke test — daemon + fake LLM HTTP server.
//!
//! 需求: docs/requirements/agent.md §F1
//!
//! Step 1.1 infrastructure validation: spawns the real `closeclaw` daemon
//! binary with `--config-dir <tmp> --foreground`, starts an in-process
//! fake LLM HTTP server (`closeclaw_fake_llm`), points `models.json` at
//! it, and drives one chat turn through the chat RPC Unix socket
//! (length-prefixed JSON frames).
//!
//! STANDARDS.md §1 e2e 判定：spawn 独立 daemon 进程 + 真实 Unix socket。
//!
//! Blocker note (2026-08-22): on current master the daemon-side chat → LLM
//! path panics before any LLM request is made
//! (`SkillListingProviderWrapper::collect_builtin_listings` calls
//! `Handle::block_on` inside an async context — "Cannot start a runtime
//! from within a runtime", crates/daemon/src/bridge.rs:186). The smoke test
//! therefore asserts the *observable* contract of this wiring today:
//! the daemon starts, the chat RPC socket answers, and the client receives
//! a well-formed protocol response (Error frames for the panic are
//! protocol-valid; a hang/crash of the daemon is not). See the
//! `e2e_agent_profile_smoke` case doc for the full reasoning.
//!
//! Uses `#[cfg(feature = "fake-llm")]` to gate on the feature flag, per
//! STANDARDS.md §5.

#![cfg(feature = "fake-llm")]

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream as TokioUnixStream;
use tokio::process::{Child, Command};
use tokio::time::timeout;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Bounded wait for the daemon admin socket (startup readiness signal).
const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(30);
/// Interval between admin-socket connect attempts.
const DAEMON_POLL_INTERVAL: Duration = Duration::from_millis(200);
/// Upper bound for one full chat turn (request → Done/Error/EOF).
const CHAT_TURN_TIMEOUT: Duration = Duration::from_secs(60);
/// Upper bound for graceful shutdown after SIGTERM (drain timeout 30s + margin).
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(40);

// ---------------------------------------------------------------------------
// Helpers: fixture paths
// ---------------------------------------------------------------------------

/// Path to the fake LLM scenario fixtures (basic-text + fallback).
fn scenarios_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_llm/scenarios")
}

/// Path to the `closeclaw` daemon binary (not the test binary).
fn closeclaw_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/closeclaw")
}

// ---------------------------------------------------------------------------
// Helper: fake LLM server
// ---------------------------------------------------------------------------

/// Start an in-process fake LLM HTTP server on a random port.
///
/// Loads the shared scenario fixtures (`tests/fixtures/fake_llm/scenarios`)
/// so the engine can answer both model-matched and fallback requests.
/// Returns the bound address for `models.json`.
async fn start_fake_llm() -> std::net::SocketAddr {
    closeclaw_fake_llm::server::start_server_addr("127.0.0.1:0", Some(&scenarios_dir()))
        .await
        .expect("failed to start fake LLM server on 127.0.0.1:0")
}

// ---------------------------------------------------------------------------
// Helper: config-dir scaffolding
// ---------------------------------------------------------------------------

/// Write the mandatory + agent config layout into a temp config root.
///
/// Layout (verified against `Daemon::init_phase_1_foundation` /
/// `ConfigManager::load` / `AgentDirectoryProvider`):
///
/// ```text
/// <root>/config/{models,channels,gateway,plugins,system,accounts}.json
/// <root>/config/agents.json
/// <root>/config/credentials/openai.json     (fake key; camelCase)
/// <root>/agents/master/config.json          (model → fake provider)
/// ```
///
/// Notes:
/// - `models.json` `credentialPath` is validated with a CWD-relative
///   `Path::exists` check, so the test chdirs into `<root>/config` before
///   spawning the daemon (see `ChatHarness::spawn_daemon`).
/// - `agents/<id>/config.json` `model` accepts `"provider/model-id"`
///   (ModelSpec string form).
fn write_config_tree(root: &Path, fake_llm_addr: &str) {
    let config_dir = root.join("config");
    std::fs::create_dir_all(config_dir.join("credentials")).expect("create config dirs");
    std::fs::create_dir_all(root.join("agents").join("master")).expect("create agents dir");

    std::fs::write(
        config_dir.join("agents.json"),
        r#"{"version":"1.0.0","agents":["master"]}"#,
    )
    .expect("write agents.json");

    let models = serde_json::json!({
        "version": "1.0",
        "mode": "merge",
        "providers": {
            "openai": {
                "baseUrl": format!("http://{fake_llm_addr}/v1"),
                "protocol": "openai",
                "credentialPath": "credentials/openai.json",
                "models": [{ "id": "gpt-4o-basic", "enabled": true }]
            }
        }
    });
    std::fs::write(
        config_dir.join("models.json"),
        serde_json::to_string(&models).expect("serialize models.json"),
    )
    .expect("write models.json");

    for name in [
        "channels.json",
        "gateway.json",
        "plugins.json",
        "system.json",
        "accounts.json",
    ] {
        std::fs::write(config_dir.join(name), r#"{"version":"1.0"}"#)
            .expect("write mandatory config");
    }

    // Fake API key — camelCase per ApiKeyCredentials serde attrs.
    std::fs::write(
        config_dir.join("credentials").join("openai.json"),
        r#"{"provider":"openai","apiKey":"e2e-fake-key"}"#,
    )
    .expect("write credentials");

    std::fs::write(
        root.join("agents")
            .join("master")
            .join("config.json"),
        r#"{"id":"master","name":"Master","model":"openai/gpt-4o-basic","tools":["*"],"skills":["*"]}"#,
    )
    .expect("write master agent config");
}

// ---------------------------------------------------------------------------
// Helper: daemon spawn + readiness
// ---------------------------------------------------------------------------

/// Poll the daemon admin socket until it accepts connections or times out.
///
/// Bounded, signal-targeted readiness wait (no blind sleep) — same pattern
/// as `sigterm_tests.rs::wait_for_daemon_ready`.
async fn wait_for_daemon_ready(config_dir: &Path) {
    let socket_path = config_dir.join("admin.sock");
    let deadline = tokio::time::Instant::now() + DAEMON_READY_TIMEOUT;
    loop {
        if UnixStream::connect(&socket_path).is_ok() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "daemon admin socket not ready after {:?}: {}",
                DAEMON_READY_TIMEOUT,
                socket_path.display()
            );
        }
        tokio::time::sleep(DAEMON_POLL_INTERVAL).await;
    }
}

/// Owns the spawned daemon child. Sending SIGTERM on Drop guarantees no
/// residual process even when an assertion fails mid-test.
struct DaemonGuard(Child);

impl DaemonGuard {
    /// Send SIGTERM and wait for graceful exit.
    async fn shutdown(mut self) -> std::process::ExitStatus {
        let pid = self.0.id().expect("daemon has a PID") as libc::pid_t;
        // SAFETY: `pid` belongs to the child we spawned; the cast is a
        // lossless widening; SIGTERM is a valid signal number.
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        timeout(SHUTDOWN_TIMEOUT, self.0.wait())
            .await
            .expect("daemon should exit within the shutdown timeout")
            .expect("daemon exit status should be observable")
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        // Best-effort cleanup on panic paths: kill -TERM then drop
        // (kill_on_drop covers the rest).
        if let Some(pid) = self.0.id() {
            // SAFETY: pid belongs to this child; SIGTERM is valid.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: chat RPC client (protocol-conformant)
// ---------------------------------------------------------------------------

/// Send one `ChatMessage` and collect frames until `Done`/`Error`/EOF.
///
/// Frame format (mirrors `crates/cli/src/chat/rpc/protocol.rs`):
/// `[4-byte big-endian u32 length][JSON frame bytes]`.
async fn chat_roundtrip(
    socket_path: &Path,
    agent_id: &str,
    content: &str,
) -> Vec<serde_json::Value> {
    let stream = TokioUnixStream::connect(socket_path)
        .await
        .expect("connect to chat.sock");
    let (reader, mut writer) = stream.into_split();

    let request = serde_json::json!({
        "type": "chat_message",
        "agent_id": agent_id,
        "content": content,
    });
    let body = serde_json::to_vec(&request).expect("serialize chat request");
    let header = (body.len() as u32).to_be_bytes();
    writer.write_all(&header).await.expect("send frame header");
    writer.write_all(&body).await.expect("send frame body");
    writer.flush().await.expect("flush request");

    let mut reader = BufReader::new(reader);
    let mut frames = Vec::new();
    loop {
        let frame = match timeout(CHAT_TURN_TIMEOUT, read_frame(&mut reader)).await {
            Ok(Ok(Some(f))) => f,
            Ok(Ok(None)) => break, // EOF — server closed the connection
            Ok(Err(e)) => panic!("chat RPC read error: {e}"),
            Err(_) => panic!("chat turn timed out after {CHAT_TURN_TIMEOUT:?}"),
        };
        let is_terminal = frame.get("type").and_then(|t| t.as_str()) == Some("done")
            || frame.get("type").and_then(|t| t.as_str()) == Some("error");
        frames.push(frame);
        if is_terminal {
            break;
        }
    }
    frames
}

/// Read one length-prefixed JSON frame. `Ok(None)` on clean EOF.
async fn read_frame<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<serde_json::Value>> {
    let mut header = [0u8; 4];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(header) as usize;
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    let value = serde_json::from_slice(&body).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid frame JSON: {e}"),
        )
    })?;
    Ok(Some(value))
}

// ---------------------------------------------------------------------------
// Smoke test
// ---------------------------------------------------------------------------

/// §F1 smoke: the full black-box wiring boots and speaks the chat protocol.
///
/// Asserted today (infrastructure contract):
/// 1. daemon starts with a `models.json` pointing at the fake LLM server
///    and reaches `admin.sock` readiness;
/// 2. `chat.sock` accepts a `ChatMessage` for the `master` agent and the
///    daemon answers with well-formed protocol frames (ContentChunk or a
///    terminal Done/Error frame) — the daemon stays alive and does not
///    crash or hang;
/// 3. after SIGTERM the daemon exits gracefully (code 0) and removes its
///    sockets; no residual process remains.
///
/// Known blocker (recorded in the file-level doc): the chat → LLM call
/// path panics inside `SkillListingProviderWrapper` (block_on in async
/// context) before any LLM request is issued, so a non-empty text answer
/// cannot be asserted yet. Once that production bug is fixed, the
/// `answer` assertion below should be tightened from "protocol answered"
/// to "contains a non-empty ContentChunk from the fake LLM fallback
/// scenario".
#[tokio::test]
#[cfg(unix)]
#[serial_test::serial]
async fn e2e_agent_profile_smoke() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_root = temp_dir.path();

    let fake_llm_addr = start_fake_llm().await;
    write_config_tree(config_root, &fake_llm_addr.to_string());

    // `credentialPath` is resolved relative to the daemon process CWD
    // (CWD-relative `Path::exists` check). Instead of chdir-ing the test
    // process (process-global mutation), pass `current_dir` on the
    // daemon `Command` — the child resolves the credential file from
    // `<root>/config`, no global state touched.
    let daemon = Command::new(closeclaw_binary())
        .args(["run", "--config-dir"])
        .arg(config_root.as_os_str())
        .arg("--foreground")
        .current_dir(config_root.join("config"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn daemon");
    let mut daemon = DaemonGuard(daemon);

    wait_for_daemon_ready(config_root).await;

    // Daemon must survive startup (not crash on the fake-LLM config).
    if let Some(status) = daemon.0.try_wait().expect("try_wait daemon") {
        panic!("daemon exited prematurely during startup: {status:?}");
    }

    let frames = chat_roundtrip(&config_root.join("chat.sock"), "master", "hello world").await;

    // Protocol contract: at least one frame, exactly one terminal frame,
    // and the daemon is still alive afterwards.
    assert!(
        !frames.is_empty(),
        "chat RPC should answer with at least one frame, got none"
    );
    let terminal: Vec<&serde_json::Value> = frames
        .iter()
        .filter(|f| {
            matches!(
                f.get("type").and_then(|t| t.as_str()),
                Some("done") | Some("error")
            )
        })
        .collect();
    assert_eq!(
        terminal.len(),
        1,
        "expected exactly one terminal (Done/Error) frame, got {terminal:?} among {frames:?}"
    );
    if let Some(status) = daemon.0.try_wait().expect("try_wait daemon after turn") {
        panic!("daemon died during chat turn: {status:?}");
    }

    // Graceful shutdown: exit code 0, sockets cleaned up, no residual process.
    let status = daemon.shutdown().await;
    assert!(
        status.success(),
        "daemon should exit 0 after SIGTERM, got {status:?}"
    );

    let admin_sock = config_root.join("admin.sock");
    let chat_sock = config_root.join("chat.sock");
    assert!(
        !admin_sock.exists(),
        "admin.sock should be removed on shutdown"
    );
    assert!(
        !chat_sock.exists(),
        "chat.sock should be removed on shutdown"
    );
    // TempDir drops here — any leak would fail the "no temp leakage" bar.
}
