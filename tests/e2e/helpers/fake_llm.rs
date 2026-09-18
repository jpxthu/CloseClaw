//! Shared fake-LLM server startup for e2e tests.

use std::path::PathBuf;

/// Path to the fake LLM scenario fixtures (basic-text + fallback).
pub fn scenarios_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_llm/scenarios")
}

/// Start an in-process fake LLM HTTP server on a random port.
///
/// Loads the shared scenario fixtures (`tests/fixtures/fake_llm/scenarios`)
/// so the engine can answer both model-matched and fallback requests.
/// Returns the bound address for `models.json`.
pub async fn start_fake_llm() -> std::net::SocketAddr {
    closeclaw_fake_llm::server::start_server_addr("127.0.0.1:0", Some(&scenarios_dir()))
        .await
        .expect("failed to start fake LLM server on 127.0.0.1:0")
}
