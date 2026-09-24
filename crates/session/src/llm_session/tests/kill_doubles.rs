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
