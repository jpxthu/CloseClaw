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
//! - **boundary**: an event-free scope captures an empty `String`
//! - **error path**: a panicking future unwinds with the
//!   `set_default` guard unloaded and does not break later captures;
//!   the install/unload state is pinned by a *pair* of assertions —
//!   a positive one while the guard is in place (inside the future)
//!   and the negative one after it drops — so the check really
//!   distinguishes "installed then unloaded" from "never installed"
//!
//! Every test carries `#[serial_test::serial]` per the helper's
//! threading contract (issue #3102 callsite-interest race); each is
//! driven by the default current-thread `#[tokio::test]` flavor the
//! helper's thread-affinity contract requires.

use super::log_capture::capture_logs_async;

/// The concrete subscriber type installed by `log_capture::install`,
/// needed to ask the thread's current default dispatcher whether the
/// capture guard is still installed (error-path test below). Type
/// identity is what matters: the `VecWriter` writer parameter makes
/// this type unique to the helper — it is never installed globally —
/// so `is::<CaptureSubscriber>()` only ever matches a leaked guard.
type CaptureSubscriber = tracing_subscriber::fmt::Subscriber<
    tracing_subscriber::fmt::format::DefaultFields,
    tracing_subscriber::fmt::format::Format<tracing_subscriber::fmt::format::Full>,
    tracing_subscriber::filter::LevelFilter,
    super::log_capture::VecWriter,
>;

// ── normal path: return value passes through, level logs land in the String ──

/// The helper must be transparent for the future's output and a
/// faithful capture at the requested level: the closure's `T` comes
/// back unchanged, and a `WARN` event emitted at the requested level
/// appears in the returned `String` with its message, its level
/// marker and its structured field — i.e. the whole formatted line,
/// not just a fragment.
#[tokio::test]
#[serial_test::serial]
async fn test_capture_logs_async_returns_value_and_captures_logs_at_level() {
    let ((answer, marker), logs) = capture_logs_async(
        || async {
            tracing::warn!(dropped = 40_000, "cache break threshold crossed");
            (42u32, "value-from-after-the-await")
        },
        tracing::Level::WARN,
    )
    .await;

    assert_eq!(
        answer, 42,
        "the closure's return value must pass through unchanged; captured logs: {logs}"
    );
    assert_eq!(
        marker, "value-from-after-the-await",
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
/// `#[tokio::test]` runs the default current-thread flavor, so the
/// runtime polls both the test future and the spawned child on the
/// *same* thread; the `set_default` guard is thread-local and stays
/// installed for the whole `f().await`, so the child's events reach
/// the same `VecWriter`. With a multi-thread flavor — or a child
/// outliving the scope — the child could run on another thread (or
/// after the guard drops) and its events would be missed; that is
/// exactly what `capture_logs_async`'s threading contract forbids.
#[tokio::test]
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
#[tokio::test]
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
#[tokio::test]
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
/// captured future (while the guard is in place) a positive assertion
/// checks `Dispatch::is::<CaptureSubscriber>()` holds, and after each
/// scope ends the matching negative assertion checks it no longer
/// holds. The positive half is what makes the negative half
/// load-bearing — without it "guard was never installed" would pass
/// the same `!get_default(...)` check.
#[tokio::test]
#[serial_test::serial]
async fn test_capture_logs_async_unloads_guard_after_future_panic() {
    let outcome = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(async {
        capture_logs_async(
            || async {
                tracing::warn!("event emitted before the panic");
                // Positive half of the install/unload loop: the guard
                // is in place for the whole `f().await`, so this must
                // hold *inside* the future — it is what makes the
                // post-panic `!is::<CaptureSubscriber>()` check able
                // to tell "installed then unloaded" from "never
                // installed".
                assert!(
                    tracing::dispatcher::get_default(|d| d.is::<CaptureSubscriber>()),
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
        !tracing::dispatcher::get_default(|d| d.is::<CaptureSubscriber>()),
        "the set_default guard must be unloaded after the future panicked"
    );

    // A later event outside any capture scope must not surface in any
    // subsequent capture buffer, and the next capture must still be
    // complete and uncontaminated by the panicking one.
    tracing::warn!("event outside any capture scope");

    let (value, logs) = capture_logs_async(
        || async {
            // Positive half of the second loop: this scope's guard is
            // in place here, so the final `!is::<CaptureSubscriber>()`
            // check below negates an observed install, not an absence.
            assert!(
                tracing::dispatcher::get_default(|d| d.is::<CaptureSubscriber>()),
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
        !tracing::dispatcher::get_default(|d| d.is::<CaptureSubscriber>()),
        "the second capture's guard must be unloaded as well"
    );
}
