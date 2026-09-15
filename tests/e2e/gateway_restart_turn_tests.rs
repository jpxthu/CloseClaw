//! E2E: gateway restart path keeps the chat turn-completion chain alive.
//!
//! Step 1.9 regression lock: `execute_gateway_restart` rebuilds the Gateway
//! and a new chat RPC server. Before the fix, the restart path dropped the
//! new SessionMessageHandler's output receiver (`let _ = output_rx;`), so
//! every post-restart LLM turn hung for `TURN_COMPLETION_TIMEOUT_SECS`
//! (120s) — `collect_responses` waits for the channel close that only the
//! turn-completion consumer (`recv → finish_turns`) triggers.
//!
//! Scenario (config-watcher route): touch a restart-class config file
//! (gateway.json) → hot-reload → restart signal → watchdog (sessions all
//! idle → immediate) → gateway rebuild. Then drive one chat turn through
//! the NEW chat.sock and assert it completes well under the 120s
//! completion timeout, carrying the fake-LLM greeting text.
//!
//! STANDARDS.md §1 e2e 判定：spawn 独立 daemon 进程 + 真实 Unix socket。

#![cfg(feature = "fake-llm")]

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream as TokioUnixStream;
use tokio::process::Child;
use tokio::time::timeout;

use super::helpers;

/// Upper bound for one full chat turn (request → Done/Error/EOF).
/// The regression this test locks would hit TURN_COMPLETION_TIMEOUT_SECS
/// (120s); 60s proves completion is driven by the consumer, not the
/// timeout (mirrors CHAT_TURN_TIMEOUT in agent_profile_tests.rs).
const CHAT_TURN_TIMEOUT: Duration = Duration::from_secs(60);
/// Upper bound for graceful shutdown after SIGTERM.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(40);
/// How long to wait for the restart to complete (watchdog poll ≤10s +
/// rebuild + margin).
const RESTART_WAIT: Duration = Duration::from_secs(30);

fn scenarios_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_llm/scenarios")
}

async fn start_fake_llm() -> std::net::SocketAddr {
    closeclaw_fake_llm::server::start_server_addr("127.0.0.1:0", Some(&scenarios_dir()))
        .await
        .expect("failed to start fake LLM server on 127.0.0.1:0")
}

/// Same mandatory-config scaffold as agent_profile_tests (master agent,
/// openai provider pointed at the fake LLM, greeting scenario model).
///
/// Two pre-existing restart-path quirks the fixture must accommodate
/// (documented, not fixed in this step):
/// - `resolve_config_dir()` resolves the admin-socket parent = `<root>`,
///   so the restart reads `<root>/gateway.json` — provide it there too.
/// - `load_gateway_config` parses the file as `GatewayConfig`
///   (`name` required, `max_message_size` defaults to 0 → every message
///   rejected as "消息过长") — write explicit values so the rebuilt
///   Gateway accepts messages like the startup one (16384).
fn write_config_tree(root: &Path, fake_llm_addr: &str) {
    let config_dir = root.join("config");
    std::fs::create_dir_all(config_dir.join("credentials")).expect("create config dirs");

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

    let gateway_config = r#"{"name":"e2e","max_message_size":16384}"#;
    std::fs::write(config_dir.join("gateway.json"), gateway_config).expect("write gateway.json");
    // The restart path reads <root>/gateway.json (resolve_config_dir
    // returns the admin-socket parent, not the config subdir).
    std::fs::write(root.join("gateway.json"), gateway_config)
        .expect("write root gateway.json for restart path");

    for name in [
        "channels.json",
        "plugins.json",
        "system.json",
        "accounts.json",
    ] {
        std::fs::write(config_dir.join(name), r#"{"version":"1.0"}"#)
            .expect("write mandatory config");
    }

    std::fs::create_dir_all(root.join("agents").join("master")).expect("create agents dir");
    std::fs::write(
        root.join("agents")
            .join("master")
            .join("config.json"),
        r#"{"id":"master","name":"Master","model":"openai/gpt-4o-basic","tools":["*"],"skills":["*"]}"#,
    )
    .expect("write master agent config");

    std::fs::write(
        config_dir.join("credentials").join("openai.json"),
        r#"{"provider":"openai","apiKey":"e2e-fake-key"}"#,
    )
    .expect("write credentials");
}

/// Send one `ChatMessage` and collect frames until `Done`/`Error`/EOF.
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
            Ok(Ok(None)) => break, // EOF
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

/// Step 1.9: after a config-triggered gateway restart, a chat LLM turn
/// completes with content — not by riding out the 120s completion timeout.
#[tokio::test]
#[cfg(unix)]
#[serial_test::serial]
async fn e2e_gateway_restart_llm_turn_completes() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_root = temp_dir.path();

    let fake_llm_addr = start_fake_llm().await;
    write_config_tree(config_root, &fake_llm_addr.to_string());

    let mut daemon: Child = helpers::spawn_daemon(config_root);
    helpers::wait_for_daemon_ready_with_timeout(config_root, Duration::from_secs(30)).await;
    if let Some(status) = daemon.try_wait().expect("try_wait daemon") {
        panic!("daemon exited prematurely during startup: {status:?}");
    }

    // Pre-restart sanity: one chat turn completes with the greeting text.
    let frames = chat_roundtrip(&config_root.join("chat.sock"), "master", "hello").await;
    let text: String = frames
        .iter()
        .filter_map(|f| f.get("content").and_then(|c| c.as_str()))
        .collect();
    assert!(
        text.contains("Hi there!"),
        "pre-restart turn should carry greeting text, got: {text}"
    );

    // Trigger the gateway restart via the config-watcher route: touch a
    // restart-class file (gateway.json). Hot-reload (500ms debounce) →
    // restart signal → watchdog → execute_gateway_restart rebuilds
    // Gateway + chat RPC server.
    //
    // Two touches: the watchdog subscribes to the restart-state watch
    // channel *after* the first `Pending` send, so its `changed()` needs
    // a second transition to observe the pending state (pre-existing
    // subscribe-after-send race in the restart state machine — the
    // second change re-signals and the idle check runs immediately).
    let gateway_json = config_root.join("config").join("gateway.json");
    let original = std::fs::read_to_string(&gateway_json).expect("read gateway.json");
    std::fs::write(&gateway_json, &original).expect("touch gateway.json (1st)");
    tokio::time::sleep(Duration::from_secs(2)).await;
    std::fs::write(&gateway_json, &original).expect("touch gateway.json (2nd)");

    // Let the restart complete before polling: the rebuild chain is
    // touch → 500ms debounce → restart signal → watchdog idle check →
    // rebuild (~2.5s observed). Waiting first ensures the poll below
    // exercises the NEW chat RPC server, not the pre-restart one (the
    // pre-restart server also answers turns — polling too early would
    // pass even with the bug present).
    tokio::time::sleep(Duration::from_secs(6)).await;
    if let Some(status) = daemon.try_wait().expect("try wait daemon") {
        panic!("daemon died during gateway restart: {status:?}");
    }

    // Drive one turn through the rebuilt chat RPC server, polling until
    // it answers within RESTART_WAIT. With the turn-completion consumer
    // wired, the turn completes immediately; without it (the regression
    // this test locks), every attempt hangs for
    // TURN_COMPLETION_TIMEOUT_SECS (120s) — far beyond this budget, so
    // the deadline fires and the test fails instead of hanging.
    let deadline = tokio::time::Instant::now() + RESTART_WAIT;
    let post_frames = loop {
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "post-restart chat turn did not complete within {RESTART_WAIT:?} \
                 (turn-completion consumer missing? = 120s hang)"
            );
        }
        match timeout(Duration::from_secs(5), async {
            chat_roundtrip(&config_root.join("chat.sock"), "master", "hello").await
        })
        .await
        {
            Ok(frames) => break frames,
            Err(_) => {
                // Turn did not answer within 5s (connection mid-restart,
                // or hanging turn) — keep polling until the deadline.
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    };

    // The post-restart turn must carry the LLM answer and end with a
    // terminal frame — proving the restart path's turn-completion
    // consumer finalized it instead of the 120s timeout.
    let post_text: String = post_frames
        .iter()
        .filter_map(|f| f.get("content").and_then(|c| c.as_str()))
        .collect();
    assert!(
        post_text.contains("Hi there!"),
        "post-restart turn should carry greeting text, got: {post_text}"
    );
    let terminal = post_frames
        .iter()
        .filter(|f| {
            matches!(
                f.get("type").and_then(|t| t.as_str()),
                Some("done") | Some("error")
            )
        })
        .count();
    assert_eq!(terminal, 1, "expected exactly one terminal frame");

    if let Some(status) = daemon
        .try_wait()
        .expect("try_wait daemon after restart turn")
    {
        panic!("daemon died after restart chat turn: {status:?}");
    }

    // Graceful shutdown still works after the restart.
    let pid = daemon.id().expect("daemon has a PID") as libc::pid_t;
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    let status = timeout(SHUTDOWN_TIMEOUT, daemon.wait())
        .await
        .expect("daemon should exit within the shutdown timeout")
        .expect("daemon exit status should be observable");
    assert!(
        status.success(),
        "daemon should exit 0 after SIGTERM, got: {status:?}"
    );
}
