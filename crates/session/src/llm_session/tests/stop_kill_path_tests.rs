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
//!
//! Split out of `stop_tests.rs` to respect the 1000-line file cap
//! (CONTRIBUTING.md hard limits).

use super::super::KillHandle;
use super::capture_logs;
use super::*;
use closeclaw_common::shutdown::ShutdownMode;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

// ── test doubles ─────────────────────────────────────────────────────────

/// `KillHandle` that returns `Ok(())` immediately — the normal,
/// non-blocking path through `kill_tool_handles`.
struct FastKillHandle {
    kill_count: Arc<AtomicUsize>,
}

impl FastKillHandle {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            kill_count: Arc::new(AtomicUsize::new(0)),
        })
    }
}

impl KillHandle for FastKillHandle {
    fn kill(&self) -> io::Result<()> {
        self.kill_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// `KillHandle` whose `kill()` blocks until the test releases it —
/// models a kill that outlives the stop budget (a process ignoring
/// SIGKILL, say). A sync channel replaces the old fixed
/// `park_timeout(60s)`: dropping the sender releases a blocked
/// `kill()` instantly (drop also runs on unwind, so a failing
/// assertion can never strand the blocking-pool thread), and
/// `recv_timeout` bounds the wait as a last-resort fallback.
struct BlockingKillHandle {
    /// Incremented when `kill()` starts blocking.
    entered: Arc<AtomicUsize>,
    /// Incremented when `kill()` returns (0 while still blocked).
    finished: Arc<AtomicUsize>,
    rx: Mutex<std::sync::mpsc::Receiver<()>>,
}

impl BlockingKillHandle {
    /// Returns the handle plus its release sender; dropping (or
    /// sending on) the sender lets a blocked `kill()` finish.
    fn new() -> (Arc<Self>, std::sync::mpsc::Sender<()>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = Arc::new(Self {
            entered: Arc::new(AtomicUsize::new(0)),
            finished: Arc::new(AtomicUsize::new(0)),
            rx: Mutex::new(rx),
        });
        (handle, tx)
    }
}

impl KillHandle for BlockingKillHandle {
    fn kill(&self) -> io::Result<()> {
        self.entered.fetch_add(1, Ordering::SeqCst);
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

fn make_session(id: &str) -> Arc<RwLock<ConversationSession>> {
    Arc::new(RwLock::new(ConversationSession::new(
        id.to_string(),
        "gpt-4o".to_string(),
        tmp_path(),
    )))
}

// ── normal path: fast kill returns without paying the budget ────────────

/// A fast `kill()` must not make stop wait out the kill budget
/// (default 5 s) — stop returns promptly, the handle fires exactly
/// once, and cleanup completes.
#[tokio::test]
async fn test_stop_with_fast_kill_returns_promptly() {
    let cs = make_session("s_kill_fast");
    let handle = FastKillHandle::new();
    let kill_count = Arc::clone(&handle.kill_count);
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
async fn test_stop_with_slow_kill_handle_does_not_wedge() {
    let cs = make_session("s_slow");
    // Small budget: waiting out the production 5 s budget would push
    // this case over the STANDARDS §6 5 s line by itself.
    let kill_budget = Duration::from_millis(500);
    // Loose upper bound (several × the budget): distinguishes
    // "budget enforced" from "wedged or silently fell back to the
    // 5 s budget" without a fragile tight time comparison.
    let loose_bound = Duration::from_secs(4);

    let (handle, release) = BlockingKillHandle::new();
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
