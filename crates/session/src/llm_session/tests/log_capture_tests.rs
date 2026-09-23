//! Behaviour pinning for the async log-capture helper
//! `capture_logs_async` (`log_capture.rs`, issue #3168 Step 1.3).
//!
//! Dimensions pinned here:
//!
//! - **normal path**: the async closure's return value passes through
//!   unchanged and events at the requested level land in the returned
//!   `String` (message, level marker, structured fields)
//! - **async path**: events emitted after an await point are still
//!   captured, and a `tokio::spawn`ed child's events are captured
//!   too — basis recorded in the test itself
//! - **level filter**: `level` is an upper bound, not an exact match
//! - **boundary**: an event-free scope captures an empty `String`; a
//!   spawned child released only *after* the scope ended (guard
//!   already dropped) has its events missed — the contrast pair of
//!   the awaited-spawn capture above
//! - **error path**: a panicking future unwinds with the
//!   `set_default` guard unloaded and does not break later captures;
//!   the install/unload state is pinned by a *pair* of assertions —
//!   a positive one while the guard is in place (inside the future)
//!   and the negative one after it drops — so the check really
//!   distinguishes "installed then unloaded" from "never installed"
//!
//! Every test carries `#[serial_test::serial]` per the helper's
//! threading contract (issue #3102 callsite-interest race); each
//! declares the helper's required current-thread flavor explicitly
//! as `#[tokio::test(flavor = "current_thread")]`, so the hard
//! contract no longer leans on the attribute's implicit default.

use super::{capture_logs_async, is_installed};

// ── normal path: return value passes through, level logs land in the String ──

/// The helper must be transparent for the future's output and a
/// faithful capture at the requested level: the closure's `T` comes
/// back unchanged, and a `WARN` event emitted at the requested level
/// appears in the returned `String` with its message, its level
/// marker and its structured field — i.e. the whole formatted line,
/// not just a fragment.
#[tokio::test(flavor = "current_thread")]
#[serial_test::serial]
async fn test_capture_logs_async_returns_value_and_captures_logs_at_level() {
    let ((answer, marker), logs) = capture_logs_async(
        || async {
            tracing::warn!(dropped = 40_000, "cache break threshold crossed");
            (42u32, "value-from-the-future")
        },
        tracing::Level::WARN,
    )
    .await;

    assert_eq!(
        answer, 42,
        "the closure's return value must pass through unchanged; captured logs: {logs}"
    );
    assert_eq!(
        marker, "value-from-the-future",
        "the returned value must be the future's own output; captured logs: {logs}"
    );
    assert!(
        logs.contains("cache break threshold crossed"),
        "the event message must be captured; captured logs: {logs}"
    );
    assert!(
        logs.contains("WARN"),
        "the event's level marker must be captured; captured logs: {logs}"
    );
    assert!(
        logs.contains("dropped=40000"),
        "the event's structured field must be captured; captured logs: {logs}"
    );
}

// ── async path: events after await points and inside spawned tasks ──────────

/// Events emitted after an await point must land in the same buffer
/// as pre-await events, in emission order; a `tokio::spawn`ed child
/// task awaited inside the capture scope must be captured as well.
///
/// **Spawn attribution basis (measured 2026-09-23, this test):**
/// `#[tokio::test(flavor = "current_thread")]` runs the
/// current-thread flavor, so the runtime polls both the test future
/// and the spawned child on the *same* thread; the `set_default`
/// guard is thread-local and stays installed for the whole
/// `f().await`, so the child's events reach the same capture
/// buffer (writer). With a multi-thread flavor — or a child
/// outliving the scope — the child could run on another thread (or
/// after the guard drops) and its events would be missed; that is
/// exactly what `capture_logs_async`'s threading contract forbids.
#[tokio::test(flavor = "current_thread")]
#[serial_test::serial]
async fn test_capture_logs_async_captures_events_after_await_and_in_spawned_tasks() {
    let (child_output, logs) = capture_logs_async(
        || async {
            tracing::warn!("event before the await point");
            tokio::task::yield_now().await;
            tracing::warn!("event after the await point");
            let child = tokio::spawn(async {
                tracing::warn!("event from the spawned child");
                7u8
            });
            child.await.expect("spawned child must join")
        },
        tracing::Level::WARN,
    )
    .await;

    assert_eq!(
        child_output, 7,
        "the child's output must flow back through the awaited handle; captured logs: {logs}"
    );
    let before = logs
        .find("event before the await point")
        .expect("pre-await events must be captured");
    let after = logs
        .find("event after the await point")
        .expect("events after an await point must still be captured");
    assert!(
        before < after,
        "capture must keep emission order across the await point; captured logs: {logs}"
    );
    assert!(
        logs.contains("event from the spawned child"),
        "child-task events must be captured on the current-thread runtime; \
         captured logs: {logs}"
    );
}

// ── level filter: `level` is an upper bound, not an exact match ─────────────

/// The `level` parameter must actually drive the filter: with the
/// capture level set to `INFO`, `debug!` events stay out of the
/// buffer while `info!` (at the level) and `warn!` (above it) land
/// in it. Pinning both directions means neither a fixed-WARN filter
/// (config's `capture_warn_logs` shape) nor an exact-match filter
/// can pass this test.
#[tokio::test(flavor = "current_thread")]
#[serial_test::serial]
async fn test_capture_logs_async_filters_events_below_the_requested_level() {
    let (_, logs) = capture_logs_async(
        || async {
            tracing::debug!("debug detail below the capture level");
            tracing::info!("info event at the capture level");
            tracing::warn!("warn event above the capture level");
        },
        tracing::Level::INFO,
    )
    .await;

    assert!(
        logs.contains("info event at the capture level"),
        "events at the capture level must be kept; captured logs: {logs}"
    );
    assert!(
        logs.contains("warn event above the capture level"),
        "events above the capture level must be kept; captured logs: {logs}"
    );
    assert!(
        !logs.contains("debug detail below the capture level"),
        "events below the capture level must be filtered out; captured logs: {logs}"
    );
}

// ── boundary: an event-free scope yields an empty String ────────────────────

/// With no events emitted inside the scope the returned `String`
/// must be empty (the fmt subscriber writes nothing until an event
/// fires) while the future's value still comes back — the empty
/// buffer is a valid capture, not a failure.
#[tokio::test(flavor = "current_thread")]
#[serial_test::serial]
async fn test_capture_logs_async_returns_empty_string_when_no_events() {
    let (value, logs) = capture_logs_async(
        || async { "value without any events" },
        tracing::Level::WARN,
    )
    .await;

    assert_eq!(
        value, "value without any events",
        "the value must pass through even when nothing is logged"
    );
    assert!(
        logs.is_empty(),
        "an event-free scope must capture nothing; got: {logs:?}"
    );
}

// ── boundary: a child outliving the capture scope → its events missed ───────

/// The contrast pair of
/// `test_capture_logs_async_captures_events_after_await_and_in_spawned_tasks`
/// (awaited inside the scope → captured): a `tokio::spawn`ed child that
/// is **not** awaited in the scope, and is released by handshake only
/// after `capture_logs_async` returned — i.e. once the `set_default`
/// guard (and its buffer) has already dropped — must have its event
/// **absent** from the returned `String` (risk (b) of the helper's
/// threading contract). Each half of the check is load-bearing: the
/// parked-send proves the handshake was still unconsumed when the
/// scope ended — the child never got past it, so it cannot have
/// emitted inside the scope — the done-signal proves it really
/// emitted after the release (so the negative assertion cannot pass
/// vacuously), and the in-scope event proves this capture's own
/// buffer worked.
#[tokio::test(flavor = "current_thread")]
#[serial_test::serial]
async fn test_capture_logs_async_misses_events_from_child_outliving_scope() {
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let (emitted_tx, emitted_rx) = tokio::sync::oneshot::channel::<()>();

    let (_, logs) = capture_logs_async(
        move || async move {
            tokio::spawn(async move {
                // Park on the handshake: the child can only proceed —
                // and emit — after the test releases it, i.e. strictly
                // after the capture scope has ended.
                let _ = release_rx.await;
                tracing::warn!("event from a child outliving the scope");
                let _ = emitted_tx.send(());
            });
            tracing::warn!("event inside the capture scope");
        },
        tracing::Level::WARN,
    )
    .await;

    // The helper dropped the guard (and its buffer) on its way out;
    // release the child only now so its emission cannot race the scope.
    assert!(
        !is_installed(),
        "the capture guard must be unloaded once the scope has ended"
    );
    release_tx.send(()).expect(
        "the handshake must still be unconsumed when the scope ended — \
                 the child never got past it",
    );
    // Hang-guard only (STANDARDS §6's 30s hard cap for unit tests):
    // if the child never runs after the release, fail with an explicit
    // error instead of stalling the whole suite. The normal path
    // resolves immediately, so the generous bound changes no behaviour.
    tokio::time::timeout(std::time::Duration::from_secs(10), emitted_rx)
        .await
        .expect("the child never emitted within 10s of the release")
        .expect("the child must have emitted after the release");

    assert!(
        logs.contains("event inside the capture scope"),
        "positive control: the in-scope event must be captured; captured logs: {logs}"
    );
    assert!(
        !logs.contains("event from a child outliving the scope"),
        "a child released after the scope ended must not contribute to this \
         capture — its events are lost (risk (b)); captured logs: {logs}"
    );
}

// ── error path: a panicking future unloads the guard without fallout ────────

/// A panicking future must not strand the thread-local subscriber
/// guard: `catch_unwind` observes the panic payload crossing the
/// helper unchanged, the `set_default` guard has been dropped on the
/// way out (the thread's current default is no longer the helper's
/// subscriber), and a subsequent capture still gets a clean,
/// complete buffer of its own events only. (The interrupted
/// capture's partial buffer is dropped together with its future — a
/// panicking capture returns no `(T, String)` — so "buffer still
/// retrievable" is pinned by the next capture working, not by
/// reading back the aborted one.)
///
/// The unload state is pinned as a **closed loop**: inside each
/// captured future (while the guard is in place) a positive
/// assertion checks `is_installed()` holds, and after each
/// scope ends the matching negative assertion checks it no longer
/// holds. The positive half is what makes the negative half
/// load-bearing — without it "guard was never installed" would pass
/// the same `!is_installed()` check.
#[tokio::test(flavor = "current_thread")]
#[serial_test::serial]
async fn test_capture_logs_async_unloads_guard_after_future_panic() {
    let outcome = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(async {
        capture_logs_async(
            || async {
                tracing::warn!("event emitted before the panic");
                // Positive half of the install/unload loop: the guard
                // is in place for the whole `f().await`, so this must
                // hold *inside* the future — it is what makes the
                // post-panic `!is_installed()` check able to tell
                // "installed then unloaded" from "never installed".
                assert!(
                    is_installed(),
                    "the capture guard must be installed while the future runs"
                );
                panic!("capture future exploded");
            },
            tracing::Level::WARN,
        )
        .await;
    }))
    .await;

    let payload = outcome.expect_err("a panicking future must escape capture_logs_async");
    let msg = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_string());
    assert!(
        msg.contains("capture future exploded"),
        "the panic payload must cross the helper unchanged; got: {msg}"
    );
    assert!(
        !is_installed(),
        "the set_default guard must be unloaded after the future panicked"
    );

    // A later event outside any capture scope must not surface in any
    // subsequent capture buffer, and the next capture must still be
    // complete and uncontaminated by the panicking one.
    tracing::warn!("event outside any capture scope");

    let (value, logs) = capture_logs_async(
        || async {
            // Positive half of the second loop: this scope's guard is
            // in place here, so the final `!is_installed()` check
            // below negates an observed install, not an absence.
            assert!(
                is_installed(),
                "the capture guard must be installed while the second capture runs"
            );
            tracing::warn!("post-panic capture marker");
            5u8
        },
        tracing::Level::WARN,
    )
    .await;

    assert_eq!(
        value, 5,
        "capture after a panic must still return the future's value"
    );
    assert!(
        logs.contains("post-panic capture marker"),
        "capture after a panic must get its own complete buffer; captured logs: {logs}"
    );
    assert!(
        !logs.contains("event emitted before the panic"),
        "the panicking capture's partial buffer must not leak into later captures; \
         captured logs: {logs}"
    );
    assert!(
        !logs.contains("event outside any capture scope"),
        "events outside a capture scope must not surface in a later capture; \
         captured logs: {logs}"
    );
    assert!(
        !is_installed(),
        "the second capture's guard must be unloaded as well"
    );
}
