//! E2E: cache hit consistency check behavior (llm §F9) — fake_llm black box.
//!
//! 行为追溯需求: docs/requirements/llm.md §F9 缓存命中一致性校验子节。
//!
//! Drives the real daemon process (spawned binary + real chat Unix
//! socket) against the in-process fake LLM HTTP server, using scenario
//! fixtures (`tests/fixtures/fake_llm/scenarios/cache-consistency.json`)
//! that declare explicit `usage.cache_hit_tokens` per turn:
//!
//! - **Scenario A**（追加轮命中正常）: turn1 prompt_tokens=4000 (baseline),
//!   turn2 cache_hit_tokens=3900 → deviation 100 ≤ 1024 tolerance → no
//!   anomaly, both rounds answer normally.
//! - **Scenario B**（追加轮命中骤降）: turn2 cache_hit_tokens=1000 →
//!   deviation 3000 > 1024 → anomaly: the round STILL completes with the
//!   model reply visible (disposal never blocks the conversation chain),
//!   the structured `session.cache_anomaly.detected` event lands in the
//!   debug_log JSONL (Error level, source_module="session", payload with
//!   the four CacheAnomalyInfo fields), and NO system notification reaches
//!   the chat frame stream — the e2e chat sender is the process UID, never
//!   "owner" (crates/daemon/src/chat_rpc.rs `current_uid()`), so the
//!   Owner-directed notification is silently dropped at the Gateway.
//!   The "Owner receives the notification" positive path is covered by
//!   the Step 1.2 integration tests (controllable sender_id there); e2e
//!   cannot fake the Owner identity (repo-wide convention).
//! - **Scenario C**（字段缺失豁免）: turn2 `cache_fields_missing: true`
//!   → response carries no cache fields → the round is not checked, no
//!   anomaly side effects.
//!
//! STANDARDS.md §1 e2e 判定：spawn 独立 daemon 进程 + 真实 Unix socket。

#![cfg(feature = "fake-llm")]

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::helpers;
use super::helpers::chat::{assert_single_terminal, chat_roundtrip, collect_content_text};
use super::helpers::config::{write_config_tree, ConfigTreeOpts};
use super::helpers::daemon_guard::DaemonGuard;
use super::helpers::fake_llm::start_fake_llm;

/// Marker prefix of the cache-anomaly Owner system notification text
/// (`CacheAnomalyInfo::format_notification`, `crates/common/src/llm_stats.rs`).
const ANOMALY_NOTIFICATION_MARKER: &str = "[缓存命中异常]";

/// `event_type` of the structured debug_log event emitted at the unified
/// advancement point when the check flags an anomaly (Step 1.3,
/// `session_handler_cache_check.rs`).
const ANOMALY_EVENT_TYPE: &str = "session.cache_anomaly.detected";

// ---------------------------------------------------------------------------
// Daemon lifecycle + config helpers
// ---------------------------------------------------------------------------

/// Start the fake LLM server, write the config tree with `model_id` as the
/// single enabled model (scenario `match_.model_id` must equal it), enable
/// daemon debug logging (JSONL dir inside the temp root), spawn the daemon
/// (shared `helpers::daemon_guard`) and wait for socket readiness.
///
/// Returns `(daemon guard, chat socket path, debug log dir)`.
async fn spawn_scenario_daemon(
    config_root: &Path,
    model_id: &str,
) -> (DaemonGuard, PathBuf, PathBuf) {
    let fake_llm_addr = start_fake_llm().await;
    write_config_tree(
        config_root,
        ConfigTreeOpts::new(fake_llm_addr).with_models(&[model_id]),
    );
    disable_skill_listing_injection(config_root);
    let log_dir = enable_debug_log(config_root);
    let daemon = helpers::daemon_guard::spawn_ready_daemon(config_root).await;
    (daemon, config_root.join("chat.sock"), log_dir)
}

/// Disable the skill-listing injection so consecutive turns stay true
/// tail-append extension rounds at the wire level.
///
/// Why: the daemon injects the skill listing as a position-0 system
/// message on the FIRST turn of a session only (`skill_listing.rs`
/// incremental diff — later turns with an unchanged listing inject
/// `None`), so the default config's wire messages are
/// `[listing-system, user1]` → `[user1, assistant1, user2]` — the prefix
/// changes and the cache check (correctly) exempts the round as
/// PrefixMismatch. A non-matching agent skills whitelist makes the
/// listing empty on every turn, so no listing message is ever injected
/// and the message list grows strictly by tail appends.
fn disable_skill_listing_injection(config_root: &Path) {
    let path = config_root
        .join("agents")
        .join("master")
        .join("config.json");
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read agent config"))
            .expect("parse agent config");
    config["skills"] = serde_json::json!(["e2e-no-skill-matches"]);
    std::fs::write(
        &path,
        serde_json::to_string(&config).expect("serialize agent config"),
    )
    .expect("write agent config");
}

/// Enable daemon debug logging by writing `<config_root>/config/debug_log.json`
/// (read once at daemon startup, `daemon/lifecycle/bootstrap.rs`); the log
/// directory lives inside the test temp root so TempDir drop cleans it.
/// Returns the JSONL log directory path.
fn enable_debug_log(config_root: &Path) -> PathBuf {
    let log_dir = config_root.join("debug-logs");
    let config = serde_json::json!({
        "min_level": "debug",
        "log_dir": log_dir.to_string_lossy(),
        "retention_days": 1,
    });
    std::fs::write(
        config_root.join("config").join("debug_log.json"),
        config.to_string(),
    )
    .expect("write debug_log.json");
    log_dir
}

// ---------------------------------------------------------------------------
// Debug-log JSONL + frame assertions
// ---------------------------------------------------------------------------

/// Read all debug_log JSONL events under `log_dir` (missing dir → empty).
fn read_debug_events(log_dir: &Path) -> Vec<serde_json::Value> {
    let mut events = Vec::new();
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return events;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let content = std::fs::read_to_string(&path).expect("read debug log jsonl");
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            events.push(serde_json::from_str(line).expect("parse debug log jsonl line"));
        }
    }
    events
}

/// Filter events down to the cache anomaly event type.
fn anomaly_events(events: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    events
        .into_iter()
        .filter(|e| e.get("event_type").and_then(|v| v.as_str()) == Some(ANOMALY_EVENT_TYPE))
        .collect()
}

/// Poll the JSONL until the cache anomaly event appears (deadline 5s).
///
/// The emit path is `tokio::spawn`ed and the writer flushes per event, but
/// the write may land a beat after the terminal chat frame — hence the
/// bounded poll instead of a single read.
async fn wait_for_anomaly_event(log_dir: &Path) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let found = anomaly_events(read_debug_events(log_dir)).pop();
        if let Some(event) = found {
            return event;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "expected a {ANOMALY_EVENT_TYPE} event in the debug log JSONL within 5s"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Assert no cache anomaly event exists in the JSONL after a 1s settle
/// window (writes flush immediately once the spawned emit task runs, so an
/// anomaly during the completed turn would have landed well within 1s).
async fn assert_no_anomaly_event(log_dir: &Path) {
    tokio::time::sleep(Duration::from_secs(1)).await;
    let anomalies = anomaly_events(read_debug_events(log_dir));
    assert!(
        anomalies.is_empty(),
        "unexpected {ANOMALY_EVENT_TYPE} events: {anomalies:?}"
    );
}

/// Assert the chat frame stream is clean of the anomaly notification —
/// both in the rendered content text and in the raw frames.
fn assert_no_notification_in_frames(frames: &[serde_json::Value]) {
    let text = collect_content_text(frames);
    assert!(
        !text.contains(ANOMALY_NOTIFICATION_MARKER),
        "chat frames must not carry the cache anomaly notification, got: {text}"
    );
    let raw = format!("{frames:?}");
    assert!(
        !raw.contains(ANOMALY_NOTIFICATION_MARKER),
        "raw frame stream must not carry the cache anomaly notification: {raw}"
    );
}

// ---------------------------------------------------------------------------
// Scenario B: anomaly round (hit collapses 4000 → 1000, deviation 3000)
// ---------------------------------------------------------------------------

/// §F9: the anomaly round completes normally with the reply visible, the
/// structured debug_log event records the detection, and the Owner-directed
/// notification does NOT reach the non-Owner chat frame stream.
#[tokio::test]
#[cfg(unix)]
#[serial_test::serial]
async fn test_cache_anomaly_round_completes_without_notification() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_root = temp_dir.path();
    let (daemon, chat_sock, log_dir) =
        spawn_scenario_daemon(config_root, "gpt-4o-cache-drop").await;

    // Turn 1 — baseline round (no previous snapshot → new baseline).
    let frames1 = chat_roundtrip(&chat_sock, "master", "cache round one").await;
    let text1 = collect_content_text(&frames1);
    assert!(
        text1.contains("Baseline round done."),
        "turn1 should carry the baseline reply, got: {text1}"
    );
    assert_single_terminal(&frames1);
    assert_no_notification_in_frames(&frames1);

    // Turn 2 — append-extension round with declared hit 1000 vs reference
    // 4000 → deviation 3000 > 1024 tolerance → anomaly disposal runs, but
    // the round completes with the model reply visible.
    let frames2 = chat_roundtrip(&chat_sock, "master", "cache round two").await;
    let text2 = collect_content_text(&frames2);
    assert!(
        text2.contains("Reply after cache anomaly."),
        "anomaly round must still complete with a visible reply, got: {text2}"
    );
    assert_single_terminal(&frames2);
    assert_no_notification_in_frames(&frames2);

    // Positive detection evidence: structured debug_log event with the
    // CacheAnomalyInfo payload (previous 4000, hit 1000, deviation 3000,
    // tolerance 1024), Error level, session-attributed.
    let event = wait_for_anomaly_event(&log_dir).await;
    assert_eq!(event["level"], "error", "anomaly event level: {event}");
    assert_eq!(
        event["source_module"], "session",
        "anomaly event source: {event}"
    );
    assert_eq!(
        event["payload"]["previous_prompt_tokens"], 4000,
        "payload: {event}"
    );
    assert_eq!(
        event["payload"]["current_cache_read_tokens"], 1000,
        "payload: {event}"
    );
    assert_eq!(
        event["payload"]["deviation_tokens"], 3000,
        "payload: {event}"
    );
    assert_eq!(
        event["payload"]["tolerance_tokens"], 1024,
        "payload: {event}"
    );
    assert!(
        event.get("session_key").is_some_and(|k| k.is_string()),
        "anomaly event must carry the session correlation key: {event}"
    );

    helpers::shutdown_ok(daemon).await;
}

// ---------------------------------------------------------------------------
// Scenario A: append round with hit within tolerance (3900 vs 4000)
// ---------------------------------------------------------------------------

/// §F9: a normal append-extension round (deviation 100 ≤ 1024) answers
/// normally with no anomaly side effects — no notification in the frames
/// and no anomaly event in the debug log.
#[tokio::test]
#[cfg(unix)]
#[serial_test::serial]
async fn test_cache_normal_hit_round_no_anomaly() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_root = temp_dir.path();
    let (daemon, chat_sock, log_dir) = spawn_scenario_daemon(config_root, "gpt-4o-cache-ok").await;

    let frames1 = chat_roundtrip(&chat_sock, "master", "cache round one").await;
    let text1 = collect_content_text(&frames1);
    assert!(
        text1.contains("Cache baseline round done."),
        "turn1 should carry the baseline reply, got: {text1}"
    );
    assert_single_terminal(&frames1);
    assert_no_notification_in_frames(&frames1);

    let frames2 = chat_roundtrip(&chat_sock, "master", "cache round two").await;
    let text2 = collect_content_text(&frames2);
    assert!(
        text2.contains("Cache hit within tolerance."),
        "turn2 should carry the normal-hit reply, got: {text2}"
    );
    assert_single_terminal(&frames2);
    assert_no_notification_in_frames(&frames2);

    assert_no_anomaly_event(&log_dir).await;

    helpers::shutdown_ok(daemon).await;
}

// ---------------------------------------------------------------------------
// Scenario C: cache fields missing → exemption round
// ---------------------------------------------------------------------------

/// §F9: a round whose response carries no cache fields
/// (`cache_fields_missing: true`) is exempt from the numeric check — both
/// rounds answer normally with no anomaly side effects.
#[tokio::test]
#[cfg(unix)]
#[serial_test::serial]
async fn test_cache_fields_missing_round_exempt() {
    let temp_dir = tempfile::tempdir().expect("temp dir for test");
    let config_root = temp_dir.path();
    let (daemon, chat_sock, log_dir) =
        spawn_scenario_daemon(config_root, "gpt-4o-cache-missing").await;

    let frames1 = chat_roundtrip(&chat_sock, "master", "cache round one").await;
    let text1 = collect_content_text(&frames1);
    assert!(
        text1.contains("Missing-field baseline round done."),
        "turn1 should carry the baseline reply, got: {text1}"
    );
    assert_single_terminal(&frames1);
    assert_no_notification_in_frames(&frames1);

    let frames2 = chat_roundtrip(&chat_sock, "master", "cache round two").await;
    let text2 = collect_content_text(&frames2);
    assert!(
        text2.contains("Exempt round completed normally."),
        "turn2 should carry the exempt-round reply, got: {text2}"
    );
    assert_single_terminal(&frames2);
    assert_no_notification_in_frames(&frames2);

    assert_no_anomaly_event(&log_dir).await;

    helpers::shutdown_ok(daemon).await;
}
