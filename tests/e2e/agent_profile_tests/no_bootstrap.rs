//! E2E: `noBootstrap` consumption (issue #3475) — black-box daemon +
//! fake LLM verification that an agent declaring `noBootstrap: true`
//! gets a system prompt free of bootstrap-file and MEMORY.md content,
//! while an undeclared control agent keeps injecting them.
//!
//! Split out of `agent_profile_tests.rs` (CONTRIBUTING.md 1000-line
//! file cap); the module path keeps the `agent_profile_tests` segment
//! so `-E 'test(agent_profile_tests)'` selects these cases too.
//! Feature gating is inherited from the parent module's
//! `#![cfg(feature = "fake-llm")]`.

use std::time::Duration;

use crate::helpers;
use crate::helpers::chat::{chat_roundtrip, collect_content_text};
use crate::helpers::config::{write_agent_config, write_config_tree, ConfigTreeOpts};
use crate::helpers::daemon_guard::spawn_daemon_guarded;
use crate::helpers::fake_llm::start_fake_llm;

/// issue #3475 acceptance ①: an agent declaring `noBootstrap: true` runs
/// as a session without bootstrap files — the system prompt carries
/// neither bootstrap-file content (`IDENTITY.md`) nor long-term memory
/// (`MEMORY.md`), although both exist on disk with unique markers
/// (`docs/design/system_prompt/static-layer.md` §兜底与变体:
/// `noBootstrap=true` behaves like a missing agentDir and takes
/// precedence over `bootstrapMode`).
///
/// Discriminative observation via the `no-bootstrap.json` scenario
/// split (all `model_id = "gpt-4o-nobootstrap"`; the system prompt
/// ships as a request message — proven by
/// `e2e_agent_system_prompt_injection`):
/// - `nobootstrap-leak-identity` matches the IDENTITY.md marker
///   "NOBOOTSTRAP_IDENTITY_7X9K2" in the request messages → answers
///   "LEAKED_IDENTITY";
/// - `nobootstrap-leak-memory` matches the MEMORY.md marker
///   "NOBOOTSTRAP_MEMORY_4P8Q1" → "LEAKED_MEMORY";
/// - `nobootstrap-clean` matches only the plain chat text
///   ("keep a secret") → "NO_BOOTSTRAP_OK".
/// A single leaked file would flip the answer to a LEAKED_* text and
/// fail the assertion; "NO_BOOTSTRAP_OK" proves both files stayed out.
///
/// MEMORY.md sits next to IDENTITY.md in the agent's bootstrap dir
/// (`{config_root}/agents/master/`): `SystemPromptBuilderAdapter`
/// derives `{workspace_dir}/agents/{agent_id}` as the bootstrap dir and
/// `MemoryFragmentProvider::resolve_path` reads
/// `bootstrap_dir/MEMORY.md` — the same directory the
/// `BootstrapFragmentProvider` loads IDENTITY.md from.
#[tokio::test]
#[cfg(unix)]
#[serial_test::serial]
async fn e2e_agent_no_bootstrap_excludes_files() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_root = temp_dir.path();

    let fake_llm_addr = start_fake_llm().await;
    // 链首条=出站 model，见 ConfigTreeOpts::with_models doc
    write_config_tree(
        config_root,
        ConfigTreeOpts::new(fake_llm_addr).with_models(&["gpt-4o-nobootstrap"]),
    );

    // Bootstrap files with unique markers in the agent dir: a leak of
    // either into the system prompt is observable on the wire.
    let agent_dir = config_root.join("agents").join("master");
    std::fs::write(
        agent_dir.join("IDENTITY.md"),
        "You are GhostBot. Secret phrase NOBOOTSTRAP_IDENTITY_7X9K2.",
    )
    .expect("write bootstrap IDENTITY.md");
    std::fs::write(
        agent_dir.join("MEMORY.md"),
        "Long-term memory: remember NOBOOTSTRAP_MEMORY_4P8Q1.",
    )
    .expect("write MEMORY.md");

    // Declare noBootstrap: true (on-disk camelCase per AgentConfig).
    write_agent_config(config_root, "openai/gpt-4o-nobootstrap", None, Some(true));

    let mut daemon = spawn_daemon_guarded(config_root);
    helpers::wait_for_daemon_ready_with_timeout(config_root, Duration::from_secs(30)).await;

    let frames = chat_roundtrip(&config_root.join("chat.sock"), "master", "keep a secret").await;

    let text = collect_content_text(&frames);
    assert!(
        text.contains("NO_BOOTSTRAP_OK"),
        "response should contain NO_BOOTSTRAP_OK proving no bootstrap/memory injection, got: {text}"
    );
    assert!(
        !text.contains("LEAKED"),
        "response must not indicate a bootstrap/memory leak, got: {text}"
    );

    if let Some(status) = daemon.0.try_wait().expect("try_wait daemon after turn") {
        panic!("daemon died during chat turn: {status:?}");
    }

    let status = daemon.shutdown().await;
    assert!(status.success(), "daemon should exit 0 after SIGTERM");
}

/// issue #3475 acceptance ② (control): an agent that does NOT declare
/// `noBootstrap` keeps the existing behavior — the `IDENTITY.md`
/// bootstrap content is injected into the system prompt (same assertion
/// pattern as `e2e_agent_system_prompt_injection`).
///
/// The `control-identity-injected` scenario matches
/// `model_id = "gpt-4o-bootstrap-control"` AND the IDENTITY.md marker
/// "CONTROL_IDENTITY_2M5N8" — both must hold for the turn to answer
/// "CONTROL_INJECTED_OK". If injection regressed, no scenario matches →
/// HTTP 500 "no scenario matched" → error turn → assertion failure.
#[tokio::test]
#[cfg(unix)]
#[serial_test::serial]
async fn e2e_agent_no_bootstrap_control_injects_identity() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_root = temp_dir.path();

    let fake_llm_addr = start_fake_llm().await;
    write_config_tree(
        config_root,
        ConfigTreeOpts::new(fake_llm_addr).with_models(&["gpt-4o-bootstrap-control"]),
    );

    let agent_dir = config_root.join("agents").join("master");
    std::fs::write(
        agent_dir.join("IDENTITY.md"),
        "You are ControlBot. Phrase: CONTROL_IDENTITY_2M5N8.",
    )
    .expect("write bootstrap IDENTITY.md");

    // Control: `noBootstrap` undeclared — the key is omitted from
    // config.json and the resolved config defaults to false.
    write_agent_config(config_root, "openai/gpt-4o-bootstrap-control", None, None);

    let daemon = spawn_daemon_guarded(config_root);
    helpers::wait_for_daemon_ready_with_timeout(config_root, Duration::from_secs(30)).await;

    let frames = chat_roundtrip(&config_root.join("chat.sock"), "master", "who are you").await;

    let text = collect_content_text(&frames);
    assert!(
        text.contains("CONTROL_INJECTED_OK"),
        "control agent should still inject IDENTITY.md, got: {text}"
    );

    let status = daemon.shutdown().await;
    assert!(status.success(), "daemon should exit 0 after SIGTERM");
}
