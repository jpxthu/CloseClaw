//! Shared test doubles/helpers for the `llm_session` stop & kill-path
//! tests (issue #3161 Step 1.4; extracted along the `log_capture.rs`
//! precedent so `tests/mod.rs` keeps only module declarations, helper
//! imports, and its inline tests).
//!
//! [`stop_tests.rs`](super::stop_tests) and
//! [`stop_kill_path_tests.rs`](super::stop_kill_path_tests) both used
//! an isomorphic counting `KillHandle` (duplicated there as
//! `FastKillHandle`) plus a copy-pasted `make_session`; both now come
//! from here. Only the files this issue touched were deduplicated —
//! other test modules keep their own helpers.
//!
//! Constraint: `MockKillHandle` records kill invocations, so it is a
//! plain counter double — no global state, safe under parallel tests.

use super::super::KillHandle;
use super::*;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

/// `KillHandle` that records every `kill()` call and returns `Ok`
/// immediately — the "fast / normal path" double. Lets tests assert
/// "the kill handle was invoked exactly once".
pub(super) struct MockKillHandle {
    kill_count: Arc<AtomicUsize>,
}

impl MockKillHandle {
    /// Creates a fresh double whose kill counter starts at zero; keep
    /// the [`Self::kill_count`] handle before the double moves into
    /// the session's tool-handle map.
    pub(super) fn new() -> Self {
        Self {
            kill_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Cloneable counter handle to keep after the double moves into
    /// the session's tool-handle map.
    pub(super) fn kill_count(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.kill_count)
    }
}

impl KillHandle for MockKillHandle {
    fn kill(&self) -> io::Result<()> {
        self.kill_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Minimal `ConversationSession` factory shared by the stop /
/// kill-path tests (was copy-pasted in both consumer files).
pub(super) fn make_session(id: &str) -> Arc<RwLock<ConversationSession>> {
    Arc::new(RwLock::new(ConversationSession::new(
        id.to_string(),
        "gpt-4o".to_string(),
        tmp_path(),
    )))
}

/// One-line registration helper: collapses the repeated
/// "`cs.read().await` + `register_tool_handle`" boilerplate (with its
/// coercion to `Arc<dyn KillHandle>`) at the test call sites into a
/// single call (issue #3186 Step 1.1).
///
/// The `&Arc<K>` parameter is generic over the concrete handle type,
/// so callers pass `&handle` with no `.clone()` or cast at the call
/// site: the single `Arc` clone happens here (issue #3191 Step 1.1).
/// Semantics are identical to the inlined form: take a read lock,
/// register the handle under `call_id`.
pub(super) async fn register_kill_handle<K>(
    cs: &Arc<RwLock<ConversationSession>>,
    call_id: &str,
    handle: &Arc<K>,
) where
    K: KillHandle + 'static,
{
    let handle = Arc::clone(handle);
    cs.read().await.register_tool_handle(call_id, handle);
}

/// Count of registered tool handles — single point for the
/// `tool_handles.read().expect("tool_handles lock poisoned")`
/// boilerplate that `stop_kill_path_tests.rs` previously copy-pasted
/// at every map assertion (issue #3172 Step 1.2). Takes the
/// already-acquired session read guard.
pub(super) fn tool_handles_len(s: &ConversationSession) -> usize {
    s.tool_handles
        .read()
        .expect("tool_handles lock poisoned")
        .len()
}

/// Assert the session's tool-handle map is empty, i.e. stop cleared
/// every registered handle (the zero-count form, on top of
/// [`tool_handles_len`]). `context` names the test/scenario so a
/// failure from inside this helper still points at the right case
/// (issue #3172 Step 1.2).
pub(super) fn assert_tool_handles_empty(s: &ConversationSession, context: &str) {
    assert_eq!(
        tool_handles_len(s),
        0,
        "tool_handles map must be cleared after stop ({context})"
    );
}

/// Count form of [`assert_tool_handles_empty`]: assert the map holds
/// exactly `expected` handles — e.g. the panic path where cleanup
/// must *not* have run and `len == 1` is the expected state
/// (issue #3172 Step 1.2).
pub(super) fn assert_tool_handles_count(s: &ConversationSession, expected: usize, context: &str) {
    assert_eq!(
        tool_handles_len(s),
        expected,
        "unexpected tool_handles count ({context})"
    );
}
