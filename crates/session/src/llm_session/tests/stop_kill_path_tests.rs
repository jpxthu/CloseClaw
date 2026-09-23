//! Kill-path behaviour of `ConversationSession::stop` /
//! `kill_tool_handles` (issue #3161; §10 of the stop test matrix,
//! issue #858).
//!
//! Three paths of the per-handle kill loop:
//!
//! - **normal**: a fast `kill()` completes without paying the kill
//!   budget — stop returns promptly and cleans up
//! - **over budget**: a synchronously-blocked `kill()` cannot wedge
//!   stop — the wall-clock budget is genuinely enforced
//! - **error**: `kill()` returning `Err` warns and stop still
//!   completes its cleanup
//! - **panic**: a panicking `kill()` propagates out of stop (it is
//!   re-raised from the blocking task's `JoinError`)
//!
//! Plus two budget boundary cases (issue #3161 Step 1.3): a
//! degenerate near-zero budget cuts the wait off immediately, and a
//! sufficient (production) budget awaits an in-flight kill that
//! completes within it.
//!
//! Split out of `stop_tests.rs` to respect the 1000-line file cap
//! (CONTRIBUTING.md hard limits).

use super::super::KillHandle;
use super::capture_logs;
use super::kill_doubles::{make_session, MockKillHandle};
use closeclaw_common::shutdown::ShutdownMode;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ── test doubles ─────────────────────────────────────────────────────────

// The fast/counting double and `make_session` are shared with
// `stop_tests.rs` via `kill_doubles.rs` (issue #3161 Step 1.4); the
// doubles declared below are specific to this file's kill paths.

/// `KillHandle` whose `kill()` blocks until the test releases it —
/// models a kill that outlives the stop budget (a process ignoring
/// SIGKILL, say). A sync channel replaces the old fixed
/// `park_timeout(60s)`: dropping the sender releases a blocked
/// `kill()` instantly (drop also runs on unwind, so a failing
/// assertion can never strand the blocking-pool thread), and
/// `recv_timeout` bounds the wait as a last-resort fallback.
///
/// A second channel (best-effort) notifies the test as soon as
/// `kill()` starts, so a test can release the block at a controlled
/// point of the stop flow without any polling wait.
///
/// The receiver is kept behind a `Mutex` because
/// `std::sync::mpsc::Receiver` is **not** `Sync` (verified with this
/// repo's toolchain, rustc 1.94.0 / MSRV 1.80 — the "`Receiver: Sync`
/// since Rust 1.72" premise does not hold, issue #3161 Step 1.4),
/// while the double must be `Sync` to be held as `Arc<dyn KillHandle>`;
/// drop the `Mutex` only if that ever changes.
struct BlockingKillHandle {
    /// Incremented when `kill()` starts blocking.
    entered: Arc<AtomicUsize>,
    /// Incremented when `kill()` returns (0 while still blocked).
    finished: Arc<AtomicUsize>,
    rx: Mutex<std::sync::mpsc::Receiver<()>>,
    /// Fired (best-effort) when `kill()` starts blocking.
    entered_tx: std::sync::mpsc::Sender<()>,
}

impl BlockingKillHandle {
    /// Returns the handle, its release sender, and the "kill started"
    /// receiver; dropping (or sending on) the release sender lets a
    /// blocked `kill()` finish.
    fn new() -> (
        Arc<Self>,
        std::sync::mpsc::Sender<()>,
        std::sync::mpsc::Receiver<()>,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let handle = Arc::new(Self {
            entered: Arc::new(AtomicUsize::new(0)),
            finished: Arc::new(AtomicUsize::new(0)),
            rx: Mutex::new(rx),
            entered_tx,
        });
        (handle, tx, entered_rx)
    }
}

impl KillHandle for BlockingKillHandle {
    fn kill(&self) -> io::Result<()> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let _ = self.entered_tx.send(());
        // Wait for the test to release. The bounded timeout is a
        // safety fallback only — the sender's drop always releases,
        // so no fixed multi-second wait ever happens by design.
        let rx = self.rx.lock().expect("release channel poisoned");
        let _ = rx.recv_timeout(Duration::from_secs(5));
        self.finished.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// `KillHandle` whose `kill()` fails — models an adapter that cannot
/// deliver the termination request.
struct FailingKillHandle {
    kill_count: Arc<AtomicUsize>,
}

impl FailingKillHandle {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            kill_count: Arc::new(AtomicUsize::new(0)),
        })
    }
}

impl KillHandle for FailingKillHandle {
    fn kill(&self) -> io::Result<()> {
        self.kill_count.fetch_add(1, Ordering::SeqCst);
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "kill target unavailable",
        ))
    }
}

// ── helpers ──────────────────────────────────────────────────────────────

// (`make_session` is shared via `kill_doubles.rs`.)

// ── normal path: fast kill returns without paying the budget ────────────

/// A fast `kill()` must not make stop wait out the kill budget
/// (default 5 s) — stop returns promptly, the handle fires exactly
/// once, and cleanup completes.
#[tokio::test]
#[serial_test::serial]
async fn test_stop_with_fast_kill_returns_promptly() {
    let cs = make_session("s_kill_fast");
    let handle = Arc::new(MockKillHandle::new());
    let kill_count = handle.kill_count();
    cs.read()
        .await
        .register_tool_handle("call-fast", handle as Arc<dyn KillHandle>);

    let start = Instant::now();
    cs.read()
        .await
        .stop(false, ShutdownMode::Forceful, Duration::ZERO)
        .await;
    let elapsed = start.elapsed();

    let s = cs.read().await;
    assert!(s.is_stopped(), "stopped flag must be set");
    assert!(
        s.tool_handles
            .read()
            .expect("tool_handles lock poisoned")
            .is_empty(),
        "tool_handles map must be cleared after stop"
    );
    assert_eq!(
        kill_count.load(Ordering::SeqCst),
        1,
        "kill() must run exactly once"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "fast kill must not wait out the kill budget; stop took {elapsed:?}"
    );
}

// ── over-budget path: a blocked kill must not wedge stop ────────────────

/// A `kill()` that blocks far beyond the budget must not hold up
/// stop: with a small injected budget, stop returns within the
/// budget's order of magnitude **while the kill is still blocked**
/// and still reaches a clean state. Wall-clock budget asserted for
/// real — the old version's 30 s `timeout` only saw the inner future
/// become ready after the kill's fixed 60 s park, so it passed while
/// wall time was 60 s (issue #3161).
#[tokio::test]
#[serial_test::serial]
async fn test_stop_with_slow_kill_handle_does_not_wedge() {
    let cs = make_session("s_slow");
    // Small budget: waiting out the production 5 s budget would push
    // this case over the STANDARDS §6 5 s line by itself.
    let kill_budget = Duration::from_millis(500);
    // Loose upper bound (several × the budget): distinguishes
    // "budget enforced" from "wedged or silently fell back to the
    // 5 s budget" without a fragile tight time comparison.
    let loose_bound = Duration::from_secs(4);

    let (handle, release, _entered_rx) = BlockingKillHandle::new();
    let entered = Arc::clone(&handle.entered);
    let finished = Arc::clone(&handle.finished);
    cs.read()
        .await
        .register_tool_handle("slow", handle as Arc<dyn KillHandle>);

    tokio::time::timeout(
        loose_bound,
        cs.read().await.stop_with_kill_budget(
            false,
            ShutdownMode::Forceful,
            Duration::ZERO,
            kill_budget,
        ),
    )
    .await
    .expect("stop() must return within the loose bound while kill() is blocked");

    // The kill must have started and must still be blocked: stop
    // ended the wait via the wall-clock budget, not because kill()
    // returned.
    assert_eq!(
        entered.load(Ordering::SeqCst),
        1,
        "kill() must have started before stop() returned"
    );
    assert_eq!(
        finished.load(Ordering::SeqCst),
        0,
        "kill() must still be blocked when stop() returns"
    );

    let s = cs.read().await;
    assert!(s.is_stopped(), "stopped flag must be set");
    assert!(
        s.tool_handles
            .read()
            .expect("tool_handles lock poisoned")
            .is_empty(),
        "tool_handles map must be cleared after stop"
    );
    drop(s);

    // Release the blocked kill so the blocking-pool thread exits
    // before the runtime tears down.
    drop(release);
}

// ── error path: kill() Err warns and stop still cleans up ───────────────

/// `kill()` returning `Err` must not abort stop: the error branch
/// warns (existing semantics) and cleanup still completes — stopped
/// flag set, handle map cleared, kill ran exactly once.
#[test]
#[serial_test::serial]
fn test_stop_with_failing_kill_handle_warns_and_cleans_up() {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let cs = make_session("s_kill_err");
    let handle = FailingKillHandle::new();
    let kill_count = Arc::clone(&handle.kill_count);
    rt.block_on(async {
        cs.read()
            .await
            .register_tool_handle("call-err", handle as Arc<dyn KillHandle>);
    });

    let ((stopped, handles_len), logs) = capture_logs(
        || {
            rt.block_on(async {
                cs.read()
                    .await
                    .stop(false, ShutdownMode::Forceful, Duration::ZERO)
                    .await;
                let s = cs.read().await;
                let stopped = s.is_stopped();
                let handles_len = s
                    .tool_handles
                    .read()
                    .expect("tool_handles lock poisoned")
                    .len();
                (stopped, handles_len)
            })
        },
        tracing::Level::WARN,
    );

    assert!(stopped, "stop() must complete even when kill() returns Err");
    assert_eq!(
        handles_len, 0,
        "tool_handles must be cleared after a failing kill"
    );
    assert_eq!(
        kill_count.load(Ordering::SeqCst),
        1,
        "kill() must run exactly once"
    );
    assert!(
        logs.contains("handle.kill() returned error"),
        "kill() error must be warned about; captured logs: {logs}"
    );
}

// ── boundary: near-zero kill budget ─────────────────────────────────────

/// Boundary: a degenerate (zero) kill budget cuts the wait off at
/// once — stop returns immediately through the budget-expiry warn
/// branch while the kill is still blocked, and cleanup still
/// completes. Complements the 500 ms over-budget case above by
/// pinning the extreme of the budget scale (issue #3161 Step 1.3).
#[test]
#[serial_test::serial]
fn test_stop_with_near_zero_kill_budget_returns_immediately() {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let cs = make_session("s_kill_zero_budget");
    let (handle, release, _entered_rx) = BlockingKillHandle::new();
    let finished = Arc::clone(&handle.finished);
    rt.block_on(async {
        cs.read()
            .await
            .register_tool_handle("zero-budget", handle as Arc<dyn KillHandle>);
    });

    let ((elapsed, stopped, handles_len), logs) = capture_logs(
        || {
            rt.block_on(async {
                let start = Instant::now();
                cs.read()
                    .await
                    .stop_with_kill_budget(
                        false,
                        ShutdownMode::Forceful,
                        Duration::ZERO,
                        Duration::ZERO,
                    )
                    .await;
                let elapsed = start.elapsed();
                let s = cs.read().await;
                let stopped = s.is_stopped();
                let handles_len = s
                    .tool_handles
                    .read()
                    .expect("tool_handles lock poisoned")
                    .len();
                (elapsed, stopped, handles_len)
            })
        },
        tracing::Level::WARN,
    );

    assert!(
        elapsed < Duration::from_secs(1),
        "a zero kill budget must not hold up stop; took {elapsed:?}"
    );
    assert!(stopped, "stopped flag must be set");
    assert_eq!(handles_len, 0, "tool_handles must be cleared");
    assert_eq!(
        finished.load(Ordering::SeqCst),
        0,
        "kill() must still be blocked when stop() returns at a zero budget"
    );
    assert!(
        logs.contains("timed out after"),
        "budget expiry must go through the warn branch; captured logs: {logs}"
    );

    // Release the abandoned kill so the blocking-pool thread exits
    // before the runtime tears down.
    drop(release);
}

// ── boundary: sufficient budget awaits the in-flight kill ──────────────

/// Boundary: with the production budget, a kill that blocks for a
/// while but completes within the budget is awaited to completion —
/// stop takes the Ok branch (no budget-expiry warn), returns as soon
/// as the kill finishes instead of waiting out the full budget, and
/// still cleans up (issue #3161 Step 1.3).
#[test]
#[serial_test::serial]
fn test_stop_with_sufficient_budget_awaits_kill_completion() {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let cs = make_session("s_kill_sufficient");
    let (handle, release, entered_rx) = BlockingKillHandle::new();
    let entered = Arc::clone(&handle.entered);
    let finished = Arc::clone(&handle.finished);

    // Release the kill only after it has actually started blocking —
    // models a slow-but-within-budget kill with no fixed wait (the
    // bounded recv is a failure fallback only).
    let releaser = std::thread::spawn(move || {
        let _ = entered_rx.recv_timeout(Duration::from_secs(5));
        drop(release);
    });

    rt.block_on(async {
        cs.read()
            .await
            .register_tool_handle("sufficient-budget", handle as Arc<dyn KillHandle>);
    });

    let ((elapsed, stopped, handles_len), logs) = capture_logs(
        || {
            rt.block_on(async {
                let start = Instant::now();
                cs.read()
                    .await
                    .stop(false, ShutdownMode::Forceful, Duration::ZERO)
                    .await;
                let elapsed = start.elapsed();
                let s = cs.read().await;
                let stopped = s.is_stopped();
                let handles_len = s
                    .tool_handles
                    .read()
                    .expect("tool_handles lock poisoned")
                    .len();
                (elapsed, stopped, handles_len)
            })
        },
        tracing::Level::WARN,
    );
    releaser.join().expect("releaser thread must exit");

    assert_eq!(
        entered.load(Ordering::SeqCst),
        1,
        "kill() must have started before stop() returned"
    );
    assert_eq!(
        finished.load(Ordering::SeqCst),
        1,
        "stop() must await a kill that finishes within the budget"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "stop() must return when the kill completes, not after the full \
         budget; took {elapsed:?}"
    );
    assert!(stopped, "stopped flag must be set");
    assert_eq!(handles_len, 0, "tool_handles must be cleared");
    assert!(
        !logs.contains("timed out after"),
        "a sufficient budget must not hit the expiry branch; \
         captured logs: {logs}"
    );
}

// ── panic path: kill() panic propagates out of stop ────────────────────

/// A panic inside `kill()` surfaces as a `JoinError` on the blocking
/// task and is re-raised via `resume_unwind` — it must escape `stop()`
/// instead of being swallowed like a failed kill, and the steps after
/// the kill loop (cancel token, `clear_exec_state`) must not have run.
///
/// The **non-panic** `JoinError` branch (blocking pool shut down
/// mid-kill) is exempt from testing: it needs a runtime tearing down
/// the pool while a kill is in flight, which cannot be constructed
/// deterministically in a unit test (reason also noted at the branch
/// in `session_handles.rs`, issue #3161 Step 1.4).
#[test]
#[serial_test::serial]
fn test_stop_with_panicking_kill_handle_propagates_panic() {
    struct PanickingKillHandle;

    impl KillHandle for PanickingKillHandle {
        fn kill(&self) -> io::Result<()> {
            panic!("kill() exploded");
        }
    }

    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let cs = make_session("s_kill_panic");
    rt.block_on(async {
        cs.read().await.register_tool_handle(
            "panic-tool",
            Arc::new(PanickingKillHandle) as Arc<dyn KillHandle>,
        );
    });

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rt.block_on(async {
            cs.read()
                .await
                .stop(false, ShutdownMode::Forceful, Duration::ZERO)
                .await;
        })
    }));

    let payload = outcome.expect_err("a panicking kill() must propagate out of stop()");
    let msg = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_string());
    assert!(
        msg.contains("kill() exploded"),
        "panic payload must cross stop() unchanged; got: {msg}"
    );

    // The escape aborted stop between "kill tools" and "cancel LLM →
    // cleanup": the stopped flag is set *before* the kill loop (design
    // order), so the observable effect is that cleanup never ran.
    let s = rt.block_on(async { cs.read().await });
    assert!(
        s.is_stopped(),
        "stopped flag is set before the kill loop, panic or not"
    );
    assert_eq!(
        s.tool_handles
            .read()
            .expect("tool_handles lock poisoned")
            .len(),
        1,
        "clear_exec_state must not have run after the kill() panic"
    );
}
