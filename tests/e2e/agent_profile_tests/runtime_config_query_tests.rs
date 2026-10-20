//! E2E: agent runtime config query via admin RPC (§F6) — black-box
//! daemon + `admin.sock` length-prefixed JSON frames.
//!
//! Split out of `agent_profile_tests.rs` (CONTRIBUTING.md 1000-line
//! file cap); the module path keeps the `agent_profile_tests` segment
//! so `-E 'test(agent_profile_tests)'` selects these cases too.
//! Feature gating is inherited from the parent module's
//! `#![cfg(feature = "fake-llm")]`.

use std::path::Path;

use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::UnixStream as TokioUnixStream;

use crate::helpers;
use crate::helpers::chat::read_frame;
use crate::helpers::daemon_guard::spawn_default_ready_daemon;

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
// Admin RPC helper
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
// Test cases
// ---------------------------------------------------------------------------

/// §F6 runtime config query: daemon running, admin RPC `AgentInfo`
/// query returns fields matching the on-disk config. Querying twice
/// yields identical results (read-only, idempotent).
#[tokio::test]
#[cfg(unix)]
#[serial_test::serial]
async fn test_agent_runtime_config_query() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_root = temp_dir.path();

    let mut daemon = spawn_default_ready_daemon(config_root).await;

    let admin_sock = config_root.join("admin.sock");
    let request = serde_json::json!({"type": "agent_info", "id": "master"});

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
async fn test_agent_runtime_config_query_unknown() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_root = temp_dir.path();

    let mut daemon = spawn_default_ready_daemon(config_root).await;

    let admin_sock = config_root.join("admin.sock");
    let request = serde_json::json!({"type": "agent_info", "id": "no-such-agent"});

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
