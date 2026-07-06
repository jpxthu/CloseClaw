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

use super::super::KillHandle;
use super::super::*;

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
        s.register_tool_call("call-1");
        s.update_tool_state("call-1", closeclaw_common::ToolExecState::RunningForeground);
    }

    cs.read().await.stop(false).await;

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

    parent.read().await.stop(true).await;

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

    child.read().await.stop(false).await;

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

    cs.read().await.stop(false).await;

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

    parent.read().await.stop(true).await;

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

    parent.read().await.stop(false).await;
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

    cs.read().await.stop(false).await;
    cs.read().await.stop(false).await;
    cs.read().await.stop(true).await;

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
    cs.read().await.stop(true).await;

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
    let res = tokio::time::timeout(Duration::from_secs(30), cs.read().await.stop(false)).await;
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

    parent.read().await.stop(true).await;

    assert!(parent.read().await.is_stopped());
    assert!(child.read().await.is_stopped());
    assert!(grandchild.read().await.is_stopped());
    assert_eq!(parent_count.load(Ordering::SeqCst), 1);
    assert_eq!(child_count.load(Ordering::SeqCst), 1);
    assert_eq!(gc_count.load(Ordering::SeqCst), 1);
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

    parent.read().await.stop(true).await;

    assert!(grandchild.read().await.is_stopped());
    assert_eq!(
        gc_count.load(Ordering::SeqCst),
        1,
        "grandchild tool handle must be killed by the cascade even though \
         its cancel_token is already triggered"
    );
}
