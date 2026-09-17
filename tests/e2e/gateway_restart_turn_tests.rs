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

use std::path::Path;
use std::time::Duration;

use tokio::process::Child;
use tokio::time::timeout;

use super::helpers;
use super::helpers::chat::chat_roundtrip;
use super::helpers::config::{write_config_tree, ConfigTreeOpts};
use super::helpers::fake_llm::start_fake_llm;

/// How long to wait for the restart to complete (watchdog poll ≤10s +
/// rebuild + margin).
///
/// Chat-turn / shutdown upper bounds live in `helpers::chat::CHAT_TURN_TIMEOUT`
/// and `helpers::SHUTDOWN_TIMEOUT` (60s / 40s, shared with
/// `agent_profile_tests`).
const RESTART_WAIT: Duration = Duration::from_secs(30);

/// Trigger a gateway restart via the config-watcher route: touch a
/// restart-class file (gateway.json) → hot-reload (500ms debounce) →
/// restart signal → watchdog → `execute_gateway_restart` rebuilds
/// Gateway + chat RPC server. Panics if the daemon dies meanwhile.
///
/// Two touches: the watchdog subscribes to the restart-state watch
/// channel *after* the first `Pending` send, so its `changed()` needs
/// a second transition to observe the pending state (pre-existing
/// subscribe-after-send race in the restart state machine — the
/// second change re-signals and the idle check runs immediately).
///
/// Then a fixed wait before polling: the rebuild chain is touch →
/// 500ms debounce → restart signal → watchdog idle check → rebuild
/// (~2.5s observed). Waiting first ensures a subsequent poll exercises
/// the NEW chat RPC server, not the pre-restart one (the pre-restart
/// server also answers turns — polling too early would pass even with
/// the bug present).
async fn trigger_gateway_restart(config_root: &Path, daemon: &mut Child) {
    let gateway_json = config_root.join("config").join("gateway.json");
    let original = std::fs::read_to_string(&gateway_json).expect("read gateway.json");
    std::fs::write(&gateway_json, &original).expect("touch gateway.json (1st)");
    tokio::time::sleep(Duration::from_secs(2)).await;
    std::fs::write(&gateway_json, &original).expect("touch gateway.json (2nd)");

    tokio::time::sleep(Duration::from_secs(6)).await;
    helpers::assert_daemon_alive_with_context(daemon, Some("during gateway restart"));
}

/// Drive one turn through the rebuilt chat RPC server, polling until
/// it answers within [`RESTART_WAIT`].
///
/// With the turn-completion consumer wired, the turn completes
/// immediately; without it (the regression this test locks), every
/// attempt hangs for `TURN_COMPLETION_TIMEOUT_SECS` (120s) — far beyond
/// this budget, so the deadline fires and the test fails instead of
/// hanging. The 5s per-attempt timeout covers the connection-mid-restart
/// window (turn did not answer — keep polling until the deadline).
///
/// The caller asserts the returned frames carry the LLM answer and end
/// with exactly one terminal frame — proving the restart path's
/// turn-completion consumer finalized the turn instead of the 120s
/// timeout.
async fn wait_post_restart_turn(config_root: &Path) -> Vec<serde_json::Value> {
    let deadline = tokio::time::Instant::now() + RESTART_WAIT;
    loop {
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
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

/// Assert the daemon survived the final chat turn, then shut it down
/// gracefully (SIGTERM → exit 0 within `helpers::SHUTDOWN_TIMEOUT`).
async fn terminate_daemon(mut daemon: Child) {
    helpers::assert_daemon_alive_with_context(&mut daemon, Some("after restart chat turn"));

    let pid = daemon.id().expect("daemon has a PID") as libc::pid_t;
    // SAFETY: `pid` is the PID of the daemon child this test spawned and
    // verified is still running (the `try_wait` check just above); the
    // cast to `libc::pid_t` is a lossless widening conversion, and
    // SIGTERM is a valid signal number.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    let status = timeout(helpers::SHUTDOWN_TIMEOUT, daemon.wait())
        .await
        .expect("daemon should exit within the shutdown timeout")
        .expect("daemon exit status should be observable");
    assert!(
        status.success(),
        "daemon should exit 0 after SIGTERM, got: {status:?}"
    );
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
    write_config_tree(
        config_root,
        ConfigTreeOpts::with_root_gateway(fake_llm_addr),
    );

    let mut daemon: Child = helpers::spawn_daemon(config_root);
    helpers::wait_for_daemon_ready_with_timeout(config_root, Duration::from_secs(30)).await;
    helpers::assert_daemon_alive_with_context(&mut daemon, Some("during startup"));

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

    trigger_gateway_restart(config_root, &mut daemon).await;
    let post_frames = wait_post_restart_turn(config_root).await;

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

    terminate_daemon(daemon).await;
}
