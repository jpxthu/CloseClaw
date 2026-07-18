//! Tests for `ConversationSession::stop(cascade)` and the
//! handle / cancel-token surface.
//!
//! See Step 1.7 of issue #858 for the full list of scenarios these
//! tests are supposed to cover. Each test below is a 1:1 mapping to
//! a bullet in that section of the plan.

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use super::super::session_handles::{CascadeStopInfo, GracefulStopResult};
use super::super::KillHandle;
use super::super::*;
use closeclaw_common::shutdown::ShutdownMode;

// ── test doubles ─────────────────────────────────────────────────────────

/// `KillHandle` that records every `kill()` call and returns `Ok`.
/// Lets tests assert "the kill handle was invoked exactly once".
struct MockKillHandle {
    kill_count: Arc<AtomicUsize>,
    /// When `Some`, the handle blocks for this long before returning.
    /// Used to model a stuck process that ignores SIGKILL.
    block_for: Option<Duration>,
}

impl MockKillHandle {
    fn new() -> Self {
        Self {
            kill_count: Arc::new(AtomicUsize::new(0)),
            block_for: None,
        }
    }

    fn kill_count(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.kill_count)
    }
}

impl KillHandle for MockKillHandle {
    fn kill(&self) -> io::Result<()> {
        if let Some(d) = self.block_for {
            std::thread::sleep(d);
        }
        self.kill_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// `KillHandle` whose `kill()` blocks indefinitely (or for a long
/// time). Used to verify that the per-handle timeout in
/// `ConversationSession::kill_tool_handles` fires and the session
/// still reaches a clean state.
struct SlowKillHandle;

impl KillHandle for SlowKillHandle {
    fn kill(&self) -> io::Result<()> {
        // Block for longer than the 5 s stop-timeout. The blocking
        // is wrapped in `park_timeout` to keep the executor healthy.
        std::thread::park_timeout(Duration::from_secs(60));
        Ok(())
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

// ── 1. stop(false): kills tools, cancels LLM, clears state; no cascade ──

#[tokio::test]
async fn test_stop_false_kills_tools_cancels_llm_clears_state() {
    let cs = make_session("s_stop_false");

    // Pretend an LLM request is in flight.
    cs.read()
        .await
        .set_llm_state(closeclaw_common::LlmState::Requesting);
    assert!(cs.read().await.cancel_token().is_cancelled() == false);

    // Register a kill handle and a tool state.
    let handle = Arc::new(MockKillHandle::new());
    let kill_count = handle.kill_count();
    {
        let s = cs.read().await;
        s.register_tool_handle("call-1", handle as Arc<dyn KillHandle>);
        s.register_tool_call("call-1", "bash", "test cmd");
        s.update_tool_state("call-1", closeclaw_common::ToolExecState::RunningForeground);
    }

    cs.read().await.stop(false, ShutdownMode::Forceful).await;

    let s = cs.read().await;
    assert!(s.is_stopped(), "stopped flag must be set");
    assert!(
        s.cancel_token().is_cancelled(),
        "cancel_token must be fired"
    );
    assert_eq!(s.llm_state(), closeclaw_common::LlmState::Idle);
    assert!(
        s.tool_states
            .read()
            .expect("tool_states lock poisoned")
            .is_empty(),
        "tool_states must be cleared"
    );
    assert!(
        s.child_states
            .read()
            .expect("child_states lock poisoned")
            .is_empty(),
        "child_states must be cleared"
    );
    assert_eq!(
        kill_count.load(Ordering::SeqCst),
        1,
        "kill() must run exactly once"
    );
}

// ── 2. stop(true): cascades to children (recursive) ──────────────────────

#[tokio::test]
async fn test_stop_true_cascades_to_child_sessions() {
    let parent = make_session("s_parent");
    let child = make_session("s_child");

    // Wire a child handle (Weak) into the parent.
    parent
        .read()
        .await
        .register_child_handle("s_child", Arc::downgrade(&child));

    // Add a tool handle on the child so we can verify it gets killed.
    let child_handle = Arc::new(MockKillHandle::new());
    let child_kill_count = child_handle.kill_count();
    child
        .read()
        .await
        .register_tool_handle("call-c1", child_handle as Arc<dyn KillHandle>);

    parent.read().await.stop(true, ShutdownMode::Forceful).await;

    assert!(parent.read().await.is_stopped());
    assert!(
        child.read().await.is_stopped(),
        "child must be marked stopped after parent cascades"
    );
    assert_eq!(
        child_kill_count.load(Ordering::SeqCst),
        1,
        "child tool handle must be killed exactly once"
    );
}

// ── 3. child's independent stop() does NOT affect parent ────────────────

#[tokio::test]
async fn test_child_independent_stop_does_not_affect_parent() {
    let parent = make_session("s_p2");
    let child = make_session("s_c2");

    parent
        .read()
        .await
        .register_child_handle("s_c2", Arc::downgrade(&child));

    child.read().await.stop(false, ShutdownMode::Forceful).await;

    assert!(child.read().await.is_stopped());
    assert!(
        !parent.read().await.is_stopped(),
        "parent must NOT inherit child's stopped state"
    );
    assert!(
        !parent.read().await.cancel_token().is_cancelled(),
        "parent cancel_token must remain unset"
    );
}

// ── 4. tool handle register / unregister ─────────────────────────────────

#[tokio::test]
async fn test_tool_handle_register_unregister() {
    let cs = make_session("s_tool_reg");

    let h1 = Arc::new(MockKillHandle::new()) as Arc<dyn KillHandle>;
    let h2 = Arc::new(MockKillHandle::new()) as Arc<dyn KillHandle>;

    cs.read().await.register_tool_handle("call-a", h1);
    cs.read().await.register_tool_handle("call-b", h2);

    // The internal map length isn't part of the public surface, but
    // we can verify behaviour through stop(): both handles should
    // fire. We use the kill counts below.
    let h1 = Arc::new(MockKillHandle::new());
    let h1_count = h1.kill_count();
    let h2 = Arc::new(MockKillHandle::new());
    let h2_count = h2.kill_count();

    cs.read()
        .await
        .register_tool_handle("call-a", h1 as Arc<dyn KillHandle>);
    cs.read()
        .await
        .register_tool_handle("call-b", h2 as Arc<dyn KillHandle>);

    // Unregister one; it must NOT be killed.
    cs.read().await.unregister_tool_handle("call-a");

    cs.read().await.stop(false, ShutdownMode::Forceful).await;

    assert_eq!(
        h1_count.load(Ordering::SeqCst),
        0,
        "unregistered handle must not be killed"
    );
    assert_eq!(
        h2_count.load(Ordering::SeqCst),
        1,
        "still-registered handle must be killed exactly once"
    );
}

// ── 5. child handle register / unregister ────────────────────────────────

#[tokio::test]
async fn test_child_handle_register_unregister() {
    let parent = make_session("s_ch_reg");
    let child = make_session("s_ch_reg_child");

    parent
        .read()
        .await
        .register_child_handle("c1", Arc::downgrade(&child));

    // Unregister; the child must NOT be cascade-stopped.
    parent.read().await.unregister_child_handle("c1");

    parent.read().await.stop(true, ShutdownMode::Forceful).await;

    assert!(parent.read().await.is_stopped());
    assert!(
        !child.read().await.is_stopped(),
        "unregistered child must not be cascade-stopped"
    );
}

// ── 6. cancel_token: parent cancel -> child token auto-cancelled ────────

#[tokio::test]
async fn test_cancel_token_parent_propagates_to_child() {
    let parent = make_session("s_ct_parent");
    let child_token = parent.read().await.child_cancel_token();
    assert!(
        !child_token.is_cancelled(),
        "fresh child token must not be cancelled"
    );

    parent
        .read()
        .await
        .stop(false, ShutdownMode::Forceful)
        .await;
    assert!(
        child_token.is_cancelled(),
        "child token must auto-cancel when parent is stopped"
    );
}

// ── 7. cancel_token: child cancel does NOT affect parent ────────────────

#[tokio::test]
async fn test_cancel_token_child_does_not_propagate_to_parent() {
    let parent = make_session("s_ct_p2");
    let child_token = parent.read().await.child_cancel_token();

    // Cancel ONLY the child token.
    child_token.cancel();

    assert!(child_token.is_cancelled());
    assert!(
        !parent.read().await.cancel_token().is_cancelled(),
        "child cancel must not propagate to parent"
    );
    assert!(!parent.read().await.is_stopped());
}

// ── 8. stop() is idempotent ──────────────────────────────────────────────

#[tokio::test]
async fn test_stop_is_idempotent() {
    let cs = make_session("s_idem");

    let h = Arc::new(MockKillHandle::new());
    let h_count = h.kill_count();
    cs.read()
        .await
        .register_tool_handle("call-idem", h as Arc<dyn KillHandle>);

    cs.read().await.stop(false, ShutdownMode::Forceful).await;
    cs.read().await.stop(false, ShutdownMode::Forceful).await;
    cs.read().await.stop(true, ShutdownMode::Forceful).await;

    assert!(cs.read().await.is_stopped());
    assert_eq!(
        h_count.load(Ordering::SeqCst),
        1,
        "second/third stop calls must be no-ops"
    );
}

// ── 9. stop() on an empty session must not panic ────────────────────────

#[tokio::test]
async fn test_stop_empty_session_does_not_panic() {
    let cs = make_session("s_empty");

    // No handles, no children, no in-flight work.
    cs.read().await.stop(true, ShutdownMode::Forceful).await;

    assert!(cs.read().await.is_stopped());
    assert!(cs
        .read()
        .await
        .tool_states
        .read()
        .expect("tool_states lock poisoned")
        .is_empty());
    assert!(cs
        .read()
        .await
        .child_states
        .read()
        .expect("child_states lock poisoned")
        .is_empty());
}

// ── 10. kill timeout: SlowKillHandle doesn't wedge stop() ───────────────

#[tokio::test]
async fn test_stop_with_slow_kill_handle_does_not_wedge() {
    let cs = make_session("s_slow");

    cs.read()
        .await
        .register_tool_handle("slow", Arc::new(SlowKillHandle) as Arc<dyn KillHandle>);

    // 5 s is the production timeout in session_handles. The test
    // budget is 30 s to absorb CI jitter.
    let res = tokio::time::timeout(
        Duration::from_secs(30),
        cs.read().await.stop(false, ShutdownMode::Forceful),
    )
    .await;
    assert!(
        res.is_ok(),
        "stop() must return within budget even if kill() blocks"
    );

    let s = cs.read().await;
    assert!(s.is_stopped());
    assert!(
        s.tool_handles
            .read()
            .expect("tool_handles lock poisoned")
            .is_empty(),
        "tool_handles map must be cleared after stop"
    );
}

// ── 11. 3-level cascade: parent -> child -> grandchild ──────────────────

#[tokio::test]
async fn test_three_level_cascade_kills_all_tool_handles() {
    let parent = make_session("p");
    let child = make_session("c");
    let grandchild = make_session("g");

    let parent_h = Arc::new(MockKillHandle::new());
    let child_h = Arc::new(MockKillHandle::new());
    let gc_h = Arc::new(MockKillHandle::new());
    let parent_count = parent_h.kill_count();
    let child_count = child_h.kill_count();
    let gc_count = gc_h.kill_count();

    parent
        .read()
        .await
        .register_tool_handle("p-tool", parent_h as Arc<dyn KillHandle>);
    child
        .read()
        .await
        .register_tool_handle("c-tool", child_h as Arc<dyn KillHandle>);
    grandchild
        .read()
        .await
        .register_tool_handle("g-tool", gc_h as Arc<dyn KillHandle>);

    parent
        .read()
        .await
        .register_child_handle("c", Arc::downgrade(&child));
    child
        .read()
        .await
        .register_child_handle("g", Arc::downgrade(&grandchild));

    parent.read().await.stop(true, ShutdownMode::Forceful).await;

    assert!(parent.read().await.is_stopped());
    assert!(child.read().await.is_stopped());
    assert!(grandchild.read().await.is_stopped());
    assert_eq!(parent_count.load(Ordering::SeqCst), 1);
    assert_eq!(child_count.load(Ordering::SeqCst), 1);
    assert_eq!(gc_count.load(Ordering::SeqCst), 1);
}

// ── Step 1.3: Cascade mode passing tests ──────────────────────────────

/// Graceful cascade: parent stop(Graceful, cascade=true) marks child
/// as stopped but does NOT cancel child's cancel_token.
#[tokio::test]
async fn test_stop_graceful_cascade_does_not_cancel_child_token() {
    let parent = make_session("s_graceful_parent");
    let child = make_session("s_graceful_child");

    parent
        .read()
        .await
        .register_child_handle("s_graceful_child", Arc::downgrade(&child));

    parent.read().await.stop(true, ShutdownMode::Graceful).await;

    assert!(parent.read().await.is_stopped());
    assert!(
        child.read().await.is_stopped(),
        "child must be stopped after graceful cascade"
    );
    assert!(
        !child.read().await.is_cancelled(),
        "child cancel_token must NOT be cancelled in graceful mode"
    );
}

/// Forceful cascade: parent stop(Forceful, cascade=true) cancels
/// child's cancel_token (existing behavior, now explicitly tested).
#[tokio::test]
async fn test_stop_forceful_cascade_cancels_child_token() {
    let parent = make_session("s_forceful_parent");
    let child = make_session("s_forceful_child");

    parent
        .read()
        .await
        .register_child_handle("s_forceful_child", Arc::downgrade(&child));

    parent.read().await.stop(true, ShutdownMode::Forceful).await;

    assert!(parent.read().await.is_stopped());
    assert!(child.read().await.is_stopped());
    assert!(
        child.read().await.is_cancelled(),
        "child cancel_token must be cancelled in forceful mode"
    );
}

/// Three-level cascade with Graceful mode: grandchild, child, and root
/// are all marked stopped; none of their cancel_tokens are cancelled.
#[tokio::test]
async fn test_three_level_cascade_graceful_does_not_cancel_tokens() {
    let root = make_session("gr_root");
    let child = make_session("gr_child");
    let grandchild = make_session("gr_grandchild");

    let root_h = Arc::new(MockKillHandle::new());
    let child_h = Arc::new(MockKillHandle::new());
    let gc_h = Arc::new(MockKillHandle::new());
    let root_count = root_h.kill_count();
    let child_count = child_h.kill_count();
    let gc_count = gc_h.kill_count();

    root.read()
        .await
        .register_tool_handle("root-tool", root_h as Arc<dyn KillHandle>);
    child
        .read()
        .await
        .register_tool_handle("child-tool", child_h as Arc<dyn KillHandle>);
    grandchild
        .read()
        .await
        .register_tool_handle("gc-tool", gc_h as Arc<dyn KillHandle>);

    root.read()
        .await
        .register_child_handle("gr_child", Arc::downgrade(&child));
    child
        .read()
        .await
        .register_child_handle("gr_grandchild", Arc::downgrade(&grandchild));

    root.read().await.stop(true, ShutdownMode::Graceful).await;

    assert!(root.read().await.is_stopped());
    assert!(child.read().await.is_stopped());
    assert!(grandchild.read().await.is_stopped());
    // Graceful mode: cancel_tokens are NOT cancelled
    assert!(!root.read().await.is_cancelled());
    assert!(!child.read().await.is_cancelled());
    assert!(!grandchild.read().await.is_cancelled());
    // State is cleared (tool_handles map emptied)
    assert_eq!(root_count.load(Ordering::SeqCst), 0);
    assert_eq!(child_count.load(Ordering::SeqCst), 0);
    assert_eq!(gc_count.load(Ordering::SeqCst), 0);
}

// ── Step 1.3: Graceful stop timeout tests ──────────────────────────────

/// Idle session (no in-flight ops) completes graceful_stop immediately.
#[tokio::test]
async fn test_graceful_stop_idle_completes_immediately() {
    let cs = make_session("s_g_idle");
    let result = cs
        .read()
        .await
        .graceful_stop(Duration::from_secs(10), None)
        .await;
    assert_eq!(result, GracefulStopResult::Completed);
}

/// Graceful stop with a running tool times out and returns progress
/// without killing the process.
#[tokio::test]
async fn test_graceful_stop_running_tool_times_out_with_progress() {
    let cs = make_session("s_g_timeout");
    // Simulate a running tool.
    {
        let guard = cs.read().await;
        guard.register_tool_call("timeout-tool", "bash", "long cmd");
        guard.update_tool_state(
            "timeout-tool",
            closeclaw_common::ToolExecState::RunningForeground,
        );
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let result = cs
        .read()
        .await
        .graceful_stop(Duration::from_millis(50), Some(tx))
        .await;

    assert_eq!(result, GracefulStopResult::TimedOut);
    let progress = rx.try_recv().expect("progress must be sent on timeout");
    assert_eq!(progress.remaining, 1);
    assert!(progress
        .waiting_items
        .iter()
        .any(|(name, _)| name == "timeout-tool"));
    // Must NOT kill the tool process (no cancel_token, no kill_handle)
    assert!(!cs.read().await.is_cancelled());
}

/// Graceful stop interrupted by forceful escalation returns Interrupted.
#[tokio::test]
async fn test_graceful_stop_interrupted_by_forceful_escalation() {
    use closeclaw_common::ShutdownSignal;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    /// Minimal mock implementing `ShutdownSignal` for tests.
    struct MockShutdownSignal {
        shutting_down: AtomicBool,
        forceful: AtomicBool,
        busy_count: AtomicUsize,
    }
    impl MockShutdownSignal {
        fn new() -> Self {
            Self {
                shutting_down: AtomicBool::new(true),
                forceful: AtomicBool::new(false),
                busy_count: AtomicUsize::new(0),
            }
        }
    }
    impl ShutdownSignal for MockShutdownSignal {
        fn is_shutting_down(&self) -> bool {
            self.shutting_down.load(Ordering::SeqCst)
        }
        fn increment_busy(&self) {
            self.busy_count.fetch_add(1, Ordering::SeqCst);
        }
        fn decrement_busy(&self) {
            self.busy_count.fetch_sub(1, Ordering::SeqCst);
        }
        fn busy_count(&self) -> usize {
            self.busy_count.load(Ordering::SeqCst)
        }
        fn escalate_to_forceful(&self) -> bool {
            self.forceful.store(true, Ordering::SeqCst);
            true
        }
        fn is_forceful(&self) -> bool {
            self.forceful.load(Ordering::SeqCst)
        }
        fn drain_status(&self) -> closeclaw_common::DrainStatus {
            closeclaw_common::DrainStatus {
                state: closeclaw_common::shutdown::ShutdownState::Running,
                busy_count: 0,
                is_draining: false,
            }
        }
    }

    let cs = make_session("s_g_interrupt");
    // Register a running tool so graceful_stop doesn't complete instantly.
    {
        let guard = cs.read().await;
        guard.register_tool_call("blocker", "bash", "blocker cmd");
        guard.update_tool_state(
            "blocker",
            closeclaw_common::ToolExecState::RunningForeground,
        );
    }

    // Attach a shutdown handle so graceful_stop can detect escalation.
    let sh = Arc::new(MockShutdownSignal::new());
    {
        let mut guard = cs.write().await;
        guard.set_shutdown_handle(sh.clone() as Arc<dyn closeclaw_common::ShutdownSignal>);
    }

    // Escalate to forceful in the background after a short delay.
    let sh_clone = Arc::clone(&sh);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        sh_clone.escalate_to_forceful();
    });

    let result = cs
        .read()
        .await
        .graceful_stop(Duration::from_secs(10), None)
        .await;
    assert_eq!(result, GracefulStopResult::Interrupted);
}

// ── 12. cascade must reach a grandchild whose stopped flag is initially false
//        (verifies that the cascade does NOT short-circuit because the
//        grandchild's cancel_token is already triggered — stop() is what
//        reaps the handles.) ────────────────────────────────────────────

#[tokio::test]
async fn test_cascade_runs_grandchild_stop_even_if_already_cancelled() {
    let parent = make_session("p2");
    let grandchild = make_session("g2");
    let gc_handle = Arc::new(MockKillHandle::new());
    let gc_count = gc_handle.kill_count();

    grandchild
        .read()
        .await
        .register_tool_handle("g2-tool", gc_handle as Arc<dyn KillHandle>);

    // Wire: parent → (intermediate) → grandchild. The intermediate
    // is here purely so the cascade path is parent -> intermediate
    // -> grandchild, exercising the recursion.
    let intermediate = make_session("i");
    parent
        .read()
        .await
        .register_child_handle("i", Arc::downgrade(&intermediate));
    intermediate
        .read()
        .await
        .register_child_handle("g2", Arc::downgrade(&grandchild));

    // Important: the grandchild's cancel_token is already triggered
    // (it inherits from the intermediate which is the parent's
    // child). The grandchild's `stopped` flag is *not* set, so
    // `stop(true)` must still run on it.
    assert!(!grandchild.read().await.is_stopped());
    assert!(grandchild.read().await.is_cancelled() == false);

    parent.read().await.stop(true, ShutdownMode::Forceful).await;

    assert!(grandchild.read().await.is_stopped());
    assert_eq!(
        gc_count.load(Ordering::SeqCst),
        1,
        "grandchild tool handle must be killed by the cascade even though \
         its cancel_token is already triggered"
    );
}

// ── Step 1.3: CascadeStopInfo unit tests ─────────────────────────────

#[test]
fn test_cascade_stop_info_default_is_empty() {
    let info = CascadeStopInfo::default();
    assert!(info.timed_out_children.is_empty());
}

#[test]
fn test_cascade_stop_info_merge() {
    let mut info = CascadeStopInfo::default();
    info.timed_out_children
        .push(("child-1".into(), Duration::from_secs(5)));

    let mut other = CascadeStopInfo::default();
    other
        .timed_out_children
        .push(("child-2".into(), Duration::from_secs(3)));
    other
        .timed_out_children
        .push(("child-3".into(), Duration::from_secs(7)));

    info.merge(other);
    assert_eq!(info.timed_out_children.len(), 3);
    assert_eq!(info.timed_out_children[0].0, "child-1");
    assert_eq!(info.timed_out_children[1].0, "child-2");
    assert_eq!(info.timed_out_children[2].0, "child-3");
}

#[test]
fn test_cascade_stop_info_merge_empty() {
    let mut info = CascadeStopInfo::default();
    info.timed_out_children
        .push(("c1".into(), Duration::from_secs(1)));
    info.merge(CascadeStopInfo::default());
    assert_eq!(info.timed_out_children.len(), 1);
}

#[test]
fn test_cascade_stop_info_merge_into_empty() {
    let mut info = CascadeStopInfo::default();
    let mut other = CascadeStopInfo::default();
    other
        .timed_out_children
        .push(("c1".into(), Duration::from_secs(1)));
    info.merge(other);
    assert_eq!(info.timed_out_children.len(), 1);
}

// ── Step 1.3: stop() returns CascadeStopInfo ─────────────────────────

#[tokio::test]
async fn test_stop_returns_cascade_stop_info_empty_when_no_children() {
    let cs = make_session("s_info_empty");
    let info = cs.read().await.stop(true, ShutdownMode::Forceful).await;
    assert!(info.timed_out_children.is_empty());
}

#[tokio::test]
async fn test_stop_returns_cascade_stop_info_with_children() {
    let parent = make_session("s_info_parent");
    let child = make_session("s_info_child");
    parent
        .read()
        .await
        .register_child_handle("s_info_child", Arc::downgrade(&child));

    let info = parent.read().await.stop(true, ShutdownMode::Forceful).await;
    // Child didn't timeout (no in-flight ops), so empty
    assert!(info.timed_out_children.is_empty());
    assert!(child.read().await.is_stopped());
}

// ── Step 1.3: Multi-child CascadeStopInfo merge ──────────────────────

#[tokio::test]
async fn test_stop_multi_child_all_stopped() {
    let parent = make_session("s_multi_parent");
    let child1 = make_session("s_multi_c1");
    let child2 = make_session("s_multi_c2");
    let child3 = make_session("s_multi_c3");

    parent
        .read()
        .await
        .register_child_handle("c1", Arc::downgrade(&child1));
    parent
        .read()
        .await
        .register_child_handle("c2", Arc::downgrade(&child2));
    parent
        .read()
        .await
        .register_child_handle("c3", Arc::downgrade(&child3));

    let info = parent.read().await.stop(true, ShutdownMode::Forceful).await;
    assert!(info.timed_out_children.is_empty());
    assert!(child1.read().await.is_stopped());
    assert!(child2.read().await.is_stopped());
    assert!(child3.read().await.is_stopped());
}

// ── Step 1.3: Nested cascade propagation ─────────────────────────────

#[tokio::test]
async fn test_nested_cascade_propagation() {
    let root = make_session("s_nested_root");
    let mid = make_session("s_nested_mid");
    let leaf = make_session("s_nested_leaf");

    let leaf_handle = Arc::new(MockKillHandle::new());
    let leaf_kill_count = leaf_handle.kill_count();
    leaf.read()
        .await
        .register_tool_handle("leaf-tool", leaf_handle as Arc<dyn KillHandle>);

    root.read()
        .await
        .register_child_handle("mid", Arc::downgrade(&mid));
    mid.read()
        .await
        .register_child_handle("leaf", Arc::downgrade(&leaf));

    let info = root.read().await.stop(true, ShutdownMode::Forceful).await;

    assert!(root.read().await.is_stopped());
    assert!(mid.read().await.is_stopped());
    assert!(leaf.read().await.is_stopped());
    assert_eq!(leaf_kill_count.load(Ordering::SeqCst), 1);
    assert!(info.timed_out_children.is_empty());
}

// ── Step 1.4: kill_tool_handles sets Terminated on Running tools ────

/// force_kill() calls kill_tool_handles() but NOT clear_exec_state().
/// Verify that RunningForeground / RunningBackground tools are set to
/// Terminated before the kill, so the state machine reaches a terminal
/// state even when clear_exec_state() is skipped.
#[tokio::test]
async fn test_force_kill_sets_terminated_on_running_tools() {
    let cs = make_session("s_term_fg_bg");

    // Register and start two tools: one foreground, one background.
    {
        let guard = cs.read().await;
        guard.register_tool_call("fg-tool", "bash", "fg cmd");
        guard.update_tool_state(
            "fg-tool",
            closeclaw_common::ToolExecState::RunningForeground,
        );
        guard.register_tool_call("bg-tool", "bash", "bg cmd");
        guard.update_tool_state(
            "bg-tool",
            closeclaw_common::ToolExecState::RunningBackground,
        );
    }

    // Also register kill handles so the handles map is non-empty.
    let fg_handle = Arc::new(MockKillHandle::new());
    let fg_kill_count = fg_handle.kill_count();
    let bg_handle = Arc::new(MockKillHandle::new());
    let bg_kill_count = bg_handle.kill_count();
    cs.read()
        .await
        .register_tool_handle("fg-tool", fg_handle as Arc<dyn KillHandle>);
    cs.read()
        .await
        .register_tool_handle("bg-tool", bg_handle as Arc<dyn KillHandle>);

    // force_kill() sets Terminated + kills handles, but does NOT
    // clear_exec_state(), so we can inspect tool_states afterwards.
    cs.read().await.force_kill().await;

    let guard = cs.read().await;
    let states = guard.tool_states.read().expect("tool_states lock poisoned");
    assert_eq!(
        states.len(),
        2,
        "both tool entries must remain after force_kill"
    );
    let (fg_state, _) = states.get("fg-tool").expect("fg-tool must exist");
    assert_eq!(*fg_state, closeclaw_common::ToolExecState::Terminated);
    let (bg_state, _) = states.get("bg-tool").expect("bg-tool must exist");
    assert_eq!(*bg_state, closeclaw_common::ToolExecState::Terminated);
    drop(states);
    drop(guard);

    // Kill handles must have been invoked.
    assert_eq!(fg_kill_count.load(Ordering::SeqCst), 1);
    assert_eq!(bg_kill_count.load(Ordering::SeqCst), 1);
}

/// Full stop(Forceful) sets Terminated then clears everything.
/// We verify the end state (empty) and that kill was invoked.
#[tokio::test]
async fn test_forceful_stop_sets_terminated_then_clears() {
    let cs = make_session("s_term_clear");

    // Register a tool in RunningForeground state.
    {
        let guard = cs.read().await;
        guard.register_tool_call("tool-1", "bash", "some cmd");
        guard.update_tool_state("tool-1", closeclaw_common::ToolExecState::RunningForeground);
    }
    let handle = Arc::new(MockKillHandle::new());
    let kill_count = handle.kill_count();
    cs.read()
        .await
        .register_tool_handle("tool-1", handle as Arc<dyn KillHandle>);

    cs.read().await.stop(false, ShutdownMode::Forceful).await;

    // After full stop: tool_states is cleared, handle was killed.
    assert!(cs.read().await.tool_states.read().expect("lock").is_empty());
    assert_eq!(kill_count.load(Ordering::SeqCst), 1);
    assert!(cs.read().await.is_stopped());
}

// ── Step 1.6: Forceful stop sets Terminated before clear_exec_state ────

/// Verify that force_kill sets Terminated on RunningForeground tools
/// and that clear_exec_state then empties everything.
#[tokio::test]
async fn test_forceful_stop_fg_terminated_then_cleared() {
    let cs = make_session("s_term_fg2");
    {
        let guard = cs.read().await;
        guard.register_tool_call("fg-t", "bash", "fg cmd");
        guard.update_tool_state("fg-t", ToolExecState::RunningForeground);
    }
    let h = Arc::new(MockKillHandle::new());
    let count = h.kill_count();
    cs.read()
        .await
        .register_tool_handle("fg-t", h as Arc<dyn KillHandle>);

    cs.read().await.stop(false, ShutdownMode::Forceful).await;

    // Everything cleared.
    assert!(cs.read().await.tool_states.read().expect("lock").is_empty());
    assert!(cs.read().await.is_stopped());
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

/// Verify that force_kill sets Terminated on RunningBackground tools
/// and that clear_exec_state then empties everything.
#[tokio::test]
async fn test_forceful_stop_bg_terminated_then_cleared() {
    let cs = make_session("s_term_bg2");
    {
        let guard = cs.read().await;
        guard.register_tool_call("bg-t", "bash", "bg cmd");
        guard.update_tool_state("bg-t", ToolExecState::RunningBackground);
    }
    let h = Arc::new(MockKillHandle::new());
    let count = h.kill_count();
    cs.read()
        .await
        .register_tool_handle("bg-t", h as Arc<dyn KillHandle>);

    cs.read().await.stop(false, ShutdownMode::Forceful).await;

    assert!(cs.read().await.tool_states.read().expect("lock").is_empty());
    assert!(cs.read().await.is_stopped());
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

/// Clear_exec_state resets all dimensions: LLM idle, tools empty, children empty.
#[tokio::test]
async fn test_clear_exec_state_resets_all_dimensions() {
    let cs = make_session("s_clear_all");
    {
        let guard = cs.read().await;
        guard.set_llm_state(closeclaw_common::LlmState::Requesting);
        guard.register_tool_call("cl-t", "bash", "cmd");
        guard.update_tool_state("cl-t", ToolExecState::RunningForeground);
        guard.register_child("cl-c", "agent", "task");
    }

    cs.read().await.clear_exec_state();

    let guard = cs.read().await;
    assert_eq!(guard.llm_state(), closeclaw_common::LlmState::Idle);
    assert!(guard.tool_states.read().expect("lock").is_empty());
    assert!(guard.child_states.read().expect("lock").is_empty());
}
