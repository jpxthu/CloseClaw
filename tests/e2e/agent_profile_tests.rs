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
//! Blocker history (both resolved on this branch, 2026-09-16):
//! - The 2026-08-22 `Handle::block_on` panic
//!   (`SkillListingProviderWrapper::collect_builtin_listings` calling
//!   `Handle::block_on` inside an async context — "Cannot start a runtime
//!   from within a runtime", crates/daemon/src/bridge.rs) was fixed by
//!   moving the builtin-registry awaits onto separate scoped threads
//!   (`block_on_join_thread`, joined before returning — the sync
//!   analogue of #3054's `spawn_blocking` isolation). fake_llm now
//!   receives the request.
//! - The follow-up wiring gap — non-streaming LLM results were written
//!   to `SessionMessageHandler::output_tx` whose daemon-side receiver
//!   (`_output_rx` in crates/daemon/src/lifecycle/mod.rs — field removed
//!   on this branch) was dropped, so chat RPC clients only observed a
//!   terminal `Error` frame — was fixed by consuming that receiver and
//!   delivering completed turns through the outbound chain to the chat
//!   client.
//!
//! With both fixes in place the chat → LLM → client round trip works
//! end-to-end: `e2e_agent_model_selection` asserts the full path (fake_llm
//! receives the request; the greeting text reaches the chat client). The
//! smoke case keeps its looser infrastructure-level assertions — see the
//! `e2e_agent_profile_smoke` case doc for details.
//!
//! Uses `#[cfg(feature = "fake-llm")]` to gate on the feature flag, per
//! STANDARDS.md §5.

#![cfg(feature = "fake-llm")]

use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::UnixStream as TokioUnixStream;
use tokio::process::Child;

use super::helpers;
use super::helpers::chat::{
    assert_single_terminal, chat_roundtrip, collect_content_text, read_frame,
};
use super::helpers::config::{write_config_tree, ConfigTreeOpts};
use super::helpers::fake_llm::start_fake_llm;

// Shared constants/helpers (`helpers::chat::CHAT_TURN_TIMEOUT`,
// `helpers::SHUTDOWN_TIMEOUT`, `chat_roundtrip`, `read_frame`,
// `assert_single_terminal`, `collect_content_text`, `sigterm_and_wait`,
// `start_fake_llm`) live under `helpers/` (chat/fake_llm extracted in
// Step 1.10; the config-tree scaffold `write_config_tree` shared with
// `gateway_restart_turn_tests` in Step 1.14; terminal/content/SIGTERM
// helpers shared in Step 1.23).

// ---------------------------------------------------------------------------
// Helper: daemon spawn + readiness
// ---------------------------------------------------------------------------

/// Owns the spawned daemon child. Sending SIGTERM on Drop guarantees no
/// residual process even when an assertion fails mid-test.
struct DaemonGuard(Child);

impl DaemonGuard {
    /// Send SIGTERM and wait for graceful exit
    /// (shared `helpers::sigterm_and_wait`, Step 1.23).
    async fn shutdown(mut self) -> std::process::ExitStatus {
        helpers::sigterm_and_wait(&mut self.0).await
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

/// Spawn the closeclaw daemon with the given config root.
///
/// Wraps `helpers::spawn_daemon` which sets `HOME` to `config_root` for
/// PID file isolation, then wraps the `Child` in a `DaemonGuard` for
/// SIGTERM-on-drop safety.
fn spawn_daemon(config_root: &Path) -> DaemonGuard {
    DaemonGuard(helpers::spawn_daemon(config_root))
}

// ---------------------------------------------------------------------------
// Helper: custom agent config override
// ---------------------------------------------------------------------------

/// Write a custom agent `config.json` into the config tree.
///
/// Overwrites the master agent config created by the shared
/// `write_config_tree` scaffold to set a specific `model` and/or
/// `workspace` field.
fn write_agent_config(config_root: &Path, model: &str, workspace: Option<&str>) {
    let agent_dir = config_root.join("agents").join("master");
    std::fs::create_dir_all(&agent_dir).expect("create agent dir");
    let mut config = serde_json::json!({
        "id": "master",
        "name": "Master",
        "model": model,
        "tools": ["*"],
        "skills": ["*"]
    });
    if let Some(ws) = workspace {
        config["workspace"] = serde_json::Value::String(ws.to_string());
    }
    std::fs::write(
        agent_dir.join("config.json"),
        serde_json::to_string(&config).expect("serialize agent config"),
    )
    .expect("write agent config");
}

// ---------------------------------------------------------------------------
// Helper: chat response assertions
// ---------------------------------------------------------------------------

// `assert_single_terminal` and `collect_content_text` live in
// `helpers::chat` (shared with `gateway_restart_turn_tests`, Step 1.23).

/// Write agent config with explicit tools and disallowed_tools lists.
fn write_agent_config_with_tools(
    config_root: &Path,
    model: &str,
    tools: &[&str],
    disallowed: &[&str],
) {
    let agent_dir = config_root.join("agents").join("master");
    std::fs::create_dir_all(&agent_dir).expect("create agent dir");
    let tools: Vec<serde_json::Value> = tools.iter().map(|t| serde_json::json!(t)).collect();
    let disallowed: Vec<serde_json::Value> =
        disallowed.iter().map(|t| serde_json::json!(t)).collect();
    std::fs::write(
        agent_dir.join("config.json"),
        serde_json::json!({
            "id": "master",
            "name": "Master",
            "model": model,
            "tools": tools,
            "disallowed_tools": disallowed,
            "skills": ["*"]
        })
        .to_string(),
    )
    .expect("write agent config with tools");
}

/// Assert admin AgentInfo response contains expected fields.
fn assert_admin_agent_info(response: &serde_json::Value) {
    assert_eq!(
        response.get("type").and_then(|t| t.as_str()),
        Some("agent_info_result"),
        "expected agent_info_result, got: {response:?}"
    );
    assert_eq!(response["id"], "master");
    assert_eq!(response["name"], "Master");
    assert_eq!(response["model"], "openai/gpt-4o-basic");
    assert!(
        response["skills"].is_array(),
        "skills should be an array, got: {:?}",
        response["skills"]
    );
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
/// History: the chat → LLM call path used to panic inside
/// `SkillListingProviderWrapper` (block_on in async context) before any
/// LLM request was issued. That panic and the follow-up result-return
/// wiring gap (dropped `output_tx` receiver) were both fixed on this
/// branch, so a non-empty text answer is now observable —
/// `e2e_agent_model_selection` asserts it end-to-end. This smoke case
/// deliberately keeps the looser infrastructure-level assertions above
/// (boot + protocol answer + graceful shutdown); tightening them to
/// assert ContentChunk content is optional follow-up, not blocked by any
/// known production bug.
#[tokio::test]
#[cfg(unix)]
#[serial_test::serial]
async fn e2e_agent_profile_smoke() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_root = temp_dir.path();

    let fake_llm_addr = start_fake_llm().await;
    write_config_tree(config_root, ConfigTreeOpts::new(fake_llm_addr));

    let mut daemon = spawn_daemon(config_root);
    helpers::wait_for_daemon_ready_with_timeout(config_root, Duration::from_secs(30)).await;

    if let Some(status) = daemon.0.try_wait().expect("try_wait daemon") {
        panic!("daemon exited prematurely during startup: {status:?}");
    }

    let frames = chat_roundtrip(&config_root.join("chat.sock"), "master", "hello world").await;

    assert!(
        !frames.is_empty(),
        "chat RPC should answer with at least one frame, got none"
    );
    assert_single_terminal(&frames);

    if let Some(status) = daemon.0.try_wait().expect("try_wait daemon after turn") {
        panic!("daemon died during chat turn: {status:?}");
    }

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
}

// ---------------------------------------------------------------------------
// Step 1.2 test cases
// ---------------------------------------------------------------------------

/// §F1 model selection: the models.json-driven fallback chain determines
/// the model name in the outbound LLM request.
///
/// Attribution (verified on this branch): `UnifiedFallbackClient::chat`
/// overwrites `request.model` with the chain entry's `model_id` before
/// dispatch, and daemon `llm_init` builds one chain entry per enabled
/// models.json model — so the wire model comes from models.json, not
/// directly from the agent config's `model` field. In this fixture both
/// name "gpt-4o-basic": models.json declares the model id, and the agent
/// config references `openai/gpt-4o-basic`.
///
/// The fake_llm scenario engine matches on `model_id`. The `greeting`
/// scenario in `basic-text.json` requires `model_id = "gpt-4o-basic"`
/// AND `message_contains = "hello"`, returning a distinct text.
/// Asserting that text proves the request reached fake_llm carrying the
/// models.json-declared model id and that the answer returned to the chat
/// client — i.e. the chat → LLM → client path is wired end-to-end.
#[tokio::test]
#[cfg(unix)]
#[serial_test::serial]
async fn e2e_agent_model_selection() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_root = temp_dir.path();

    let fake_llm_addr = start_fake_llm().await;
    write_config_tree(config_root, ConfigTreeOpts::new(fake_llm_addr));
    write_agent_config(config_root, "openai/gpt-4o-basic", None);

    let daemon = spawn_daemon(config_root);
    helpers::wait_for_daemon_ready_with_timeout(config_root, Duration::from_secs(30)).await;

    let frames = chat_roundtrip(&config_root.join("chat.sock"), "master", "hello").await;

    let text = collect_content_text(&frames);
    assert!(
        text.contains("Hi there!"),
        "response should contain greeting scenario text, got: {text}"
    );

    let status = daemon.shutdown().await;
    assert!(status.success(), "daemon should exit 0 after SIGTERM");
}

/// §F1/§F6 system prompt injection (authoritative requirement source:
/// `docs/requirements/system_prompt.md` §F1 — bootstrap files are
/// injected into the system prompt; §F6 — the assembled result is
/// cached on the session runtime and fetched on every API call):
/// agent bootstrap files are injected into the system prompt, which
/// the LLM request carries.
///
/// A bootstrap file containing a unique marker
/// ("IDENTITY_SECRET_7X9K2") is created in the agent's config dir.
/// The fake_llm scenario `injected-identity` in
/// `system-prompt-inject.json` matches
/// `message_contains = "IDENTITY_SECRET_7X9K2"` and returns
/// "INJECTED_OK". Asserting that response proves the bootstrap
/// content was included in the LLM request messages.
///
/// **Status (2026-09-19)**: original blocker #2436 resolved
/// (#3054 / #3061 merged); this round re-enables the case after
/// aligning OpenAI protocol serialization with
/// `docs/design/system_prompt/static-layer.md` (static system prompt
/// now ships on the OpenAI request path), clearing the
/// unignore-workflow debt for this scenario.
#[tokio::test]
#[cfg(unix)]
#[serial_test::serial]
async fn e2e_agent_system_prompt_injection() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_root = temp_dir.path();

    let fake_llm_addr = start_fake_llm().await;
    // Three-way model alignment: models.json declares and enables
    // `gpt-4o-system-prompt` (chain index 0 → the model on the wire,
    // since the fallback client overwrites `request.model` per entry),
    // the agent config references `openai/gpt-4o-system-prompt`, and
    // the `injected-identity` fixture matches `model_id` of the same id.
    write_config_tree(
        config_root,
        ConfigTreeOpts::new(fake_llm_addr).with_models(&["gpt-4o-system-prompt"]),
    );
    write_agent_config(config_root, "openai/gpt-4o-system-prompt", None);

    // Create bootstrap file with a unique marker in the agent's config
    // directory. The system_prompt builder loads bootstrap files from
    // `{config_dir}/agents/{agent_id}/` and injects their content into
    // the system prompt, which is included in the LLM request.
    let bootstrap_dir = config_root.join("agents").join("master");
    std::fs::write(
        bootstrap_dir.join("IDENTITY.md"),
        "You are TestBot. Use the secret phrase IDENTITY_SECRET_7X9K2.",
    )
    .expect("write bootstrap file");

    let daemon = spawn_daemon(config_root);
    helpers::wait_for_daemon_ready_with_timeout(config_root, Duration::from_secs(30)).await;

    let frames = chat_roundtrip(&config_root.join("chat.sock"), "master", "tell me a joke").await;

    let text = collect_content_text(&frames);
    assert!(
        text.contains("INJECTED_OK"),
        "response should contain INJECTED_OK proving bootstrap injection, got: {text}"
    );

    let status = daemon.shutdown().await;
    assert!(status.success(), "daemon should exit 0 after SIGTERM");
}

/// §F1 workspace: agent `config.json` workspace field sets the
/// agent's working directory (CWD for tool execution).
///
/// **Actual chain** (verified 2026-08-22): `AgentRegistry::query_agent_workspace`
/// (`resolve.rs:747`) resolves the workspace path and passes it as the
/// session's working directory to the gateway. When tools execute, their
/// CWD is the workspace directory — NOT the config root. The system prompt
/// bootstrap loading (`adapter.rs:125`) reads from
/// `{config_root}/agents/{agent_id}/` and is independent of the workspace
/// field.
///
/// **Observation method**: workspace directory → CWD → tool execution →
/// read relative path (e.g. `./bootstrap_marker.txt`) → tool result
/// contains marker content. The fake_llm scenario `workspace-marker`
/// returns a `tool_call` for `Read` targeting `bootstrap_marker.txt`
/// (relative path). In the expected-pass state, the tool executes in
/// the workspace CWD and reads the file successfully.
///
/// **Status (2026-09-16)**: the original blocker #2436 — the
/// `SkillListingProviderWrapper` panic in `bridge.rs` before any LLM
/// request — was fixed on this branch (together with the result-return
/// wiring gap), so the recorded reason for `#[ignore]` no longer applies
/// as-is. Un-ignoring still requires re-verifying the tool_call chain
/// end-to-end and tightening the assertion to check the tool result
/// contains the marker content (unignore-workflow debt); this branch only
/// updates the comment, not the ignore state.
#[tokio::test]
#[cfg(unix)]
#[ignore]
#[serial_test::serial]
async fn e2e_agent_workspace() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_root = temp_dir.path();

    // Create a dedicated workspace directory with a marker file.
    // The daemon's tool execution CWD is set to this directory.
    let workspace_dir = temp_dir.path().join("agent_workspace");
    std::fs::create_dir_all(&workspace_dir).expect("create workspace dir");
    std::fs::write(
        workspace_dir.join("bootstrap_marker.txt"),
        "WORKSPACE_CWD_VERIFIED",
    )
    .expect("write workspace marker file");

    let fake_llm_addr = start_fake_llm().await;
    write_config_tree(config_root, ConfigTreeOpts::new(fake_llm_addr));
    write_agent_config(
        config_root,
        "openai/gpt-4o-workspace",
        Some(
            workspace_dir
                .to_str()
                .expect("workspace path is valid UTF-8"),
        ),
    );

    let daemon = spawn_daemon(config_root);
    helpers::wait_for_daemon_ready_with_timeout(config_root, Duration::from_secs(30)).await;

    // Send a message — the workspace-marker scenario returns a tool_call
    // for Read("./bootstrap_marker.txt"). In the expected-pass state
    // (Blocker #2436 resolved), the tool executes in the workspace CWD
    // and returns the marker content.
    let frames = chat_roundtrip(&config_root.join("chat.sock"), "master", "read the file").await;

    let text = collect_content_text(&frames);
    assert!(
        text.contains("WORKSPACE_CWD_OK"),
        "response should contain WORKSPACE_CWD_OK proving workspace CWD, got: {text}"
    );

    let status = daemon.shutdown().await;
    assert!(status.success(), "daemon should exit 0 after SIGTERM");
}

// ---------------------------------------------------------------------------
// Step 1.3 test case
// ---------------------------------------------------------------------------

/// §F1 tool allow/deny: agent config.json `tools` whitelist +
/// `disallowed_tools` blacklist constrain which tools the agent may use.
///
/// Design:
/// - Agent config sets `tools: ["Read", "Write"]` (whitelist) and
///   `disallowed_tools: ["Bash"]` (blacklist). Tools outside the whitelist
///   are never sent to the LLM; tools on the blacklist are explicitly denied
///   even if they appear in the whitelist.
/// - Fake LLM scenario `tool-allow-deny-call` (model_id
///   `gpt-4o-tool-allow-deny`) returns a `tool_call` response containing
///   calls to both `Read` (whitelisted) and `Bash` (blacklisted).
/// - In the expected-pass state (Blocker B resolved), the daemon would:
///     1. Build the system prompt with only `Read` and `Write` tools
///        (via `get_tool_descriptors` filtering) — the LLM never sees
///        `Bash` in the tool schema.
///     2. Receive the tool_call response; `Read` executes (file read via
///        tempfile path), `Bash` is rejected by `check_tool_permission`
///        (disallowed_tools check).
///     3. The rejection is observable in the chat response (tool result
///        containing a deny/error message) and/or daemon stderr.
///
/// Degradation history: this case originally asserted only the observable
/// infrastructure contract (agent config loaded without error; daemon
/// answers chat protocol frames) because Blocker B — the
/// `SkillListingProviderWrapper` panic in bridge.rs (`Handle::block_on`
/// in async context) — prevented any LLM request from being issued. That
/// panic was fixed on this branch, so the recorded reason for `#[ignore]`
/// no longer applies as-is; the assertions still need tightening to
/// verify tool execution (Read result in response) and tool rejection
/// (Bash denied in response) before the ignore can be lifted
/// (unignore-workflow debt). This branch only updates the comment, not
/// the ignore state.
#[tokio::test]
#[cfg(unix)]
#[ignore]
#[serial_test::serial]
async fn e2e_agent_tool_allow_deny() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_root = temp_dir.path();

    // Create a target file for the Read tool to read (pure file I/O,
    // no external dependencies).
    let target_file = temp_dir.path().join("e2e-tool-test.txt");
    std::fs::write(&target_file, "tool-test-content").expect("write target file for Read tool");

    let fake_llm_addr = start_fake_llm().await;
    write_config_tree(config_root, ConfigTreeOpts::new(fake_llm_addr));
    write_agent_config_with_tools(
        config_root,
        "openai/gpt-4o-tool-allow-deny",
        &["Read", "Write"],
        &["Bash"],
    );

    let mut daemon = spawn_daemon(config_root);
    helpers::wait_for_daemon_ready_with_timeout(config_root, Duration::from_secs(30)).await;
    helpers::assert_daemon_alive(&mut daemon.0);

    let frames = chat_roundtrip(&config_root.join("chat.sock"), "master", "use tools please").await;

    assert!(
        !frames.is_empty(),
        "chat RPC should answer with at least one frame, got none"
    );
    assert_single_terminal(&frames);
    helpers::assert_daemon_alive(&mut daemon.0);

    let status = daemon.shutdown().await;
    assert!(status.success(), "daemon should exit 0 after SIGTERM");
}

// ---------------------------------------------------------------------------
// Step 1.5: admin RPC helpers
// ---------------------------------------------------------------------------

/// Send one `AdminRequest` and return the single response frame.
///
/// Frame format mirrors chat RPC: `[4-byte big-endian length][JSON]`.
/// Each admin connection handles exactly one request/response cycle,
/// then the server closes the connection (connection-per-request).
async fn admin_roundtrip(socket_path: &Path, request: &serde_json::Value) -> serde_json::Value {
    let stream = TokioUnixStream::connect(socket_path)
        .await
        .expect("connect to admin.sock");
    let (reader, mut writer) = stream.into_split();

    let body = serde_json::to_vec(request).expect("serialize admin request");
    let header = (body.len() as u32).to_be_bytes();
    writer.write_all(&header).await.expect("send frame header");
    writer.write_all(&body).await.expect("send frame body");
    writer.flush().await.expect("flush admin request");
    // Drop writer so the server sees EOF after our request.
    drop(writer);

    let mut reader = BufReader::new(reader);
    read_frame(&mut reader)
        .await
        .expect("read admin response frame")
        .expect("admin response should not be EOF")
}

// ---------------------------------------------------------------------------
// Step 1.5 test cases
// ---------------------------------------------------------------------------

/// §F6 runtime config query: daemon running, admin RPC `AgentInfo`
/// query returns fields matching the on-disk config. Querying twice
/// yields identical results (read-only, idempotent).
#[tokio::test]
#[cfg(unix)]
#[serial_test::serial]
async fn e2e_agent_runtime_config_query() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_root = temp_dir.path();

    let fake_llm_addr = start_fake_llm().await;
    write_config_tree(config_root, ConfigTreeOpts::new(fake_llm_addr));
    write_agent_config(config_root, "openai/gpt-4o-basic", None);

    let mut daemon = spawn_daemon(config_root);
    helpers::wait_for_daemon_ready_with_timeout(config_root, Duration::from_secs(30)).await;

    let admin_sock = config_root.join("admin.sock");
    let request = serde_json::json!({"type": "agent_info", "name": "master"});

    let response1 = admin_roundtrip(&admin_sock, &request).await;
    assert_admin_agent_info(&response1);

    let response2 = admin_roundtrip(&admin_sock, &request).await;
    assert_eq!(
        response1, response2,
        "consecutive queries should be identical"
    );

    helpers::assert_daemon_alive(&mut daemon.0);

    let status = daemon.shutdown().await;
    assert!(status.success(), "daemon should exit 0 after SIGTERM");
}

/// §F6 runtime config query (unknown agent): querying a non-existent
/// agent returns an `Error` response (not a crash), and the daemon
/// remains alive.
#[tokio::test]
#[cfg(unix)]
#[serial_test::serial]
async fn e2e_agent_runtime_config_query_unknown() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_root = temp_dir.path();

    let fake_llm_addr = start_fake_llm().await;
    write_config_tree(config_root, ConfigTreeOpts::new(fake_llm_addr));

    let mut daemon = spawn_daemon(config_root);
    helpers::wait_for_daemon_ready_with_timeout(config_root, Duration::from_secs(30)).await;

    let admin_sock = config_root.join("admin.sock");
    let request = serde_json::json!({"type": "agent_info", "name": "no-such-agent"});

    let response = admin_roundtrip(&admin_sock, &request).await;
    assert_eq!(
        response.get("type").and_then(|t| t.as_str()),
        Some("error"),
        "unknown agent should return error, got: {response:?}"
    );
    assert!(
        response["message"]
            .as_str()
            .map(|m| m.contains("not found"))
            .unwrap_or(false),
        "error message should indicate agent not found, got: {:?}",
        response["message"]
    );

    helpers::assert_daemon_alive(&mut daemon.0);

    let status = daemon.shutdown().await;
    assert!(status.success(), "daemon should exit 0 after SIGTERM");
}
