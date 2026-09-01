//! Shared rebuild-stash state for config-triggered gateway restarts.
//!
//! During a config-triggered gateway rebuild, inbound queue-full messages
//! are stashed in the [`RebuildStash`] buffer instead of being rejected.
//! After the rebuild completes, the Daemon drains the buffer and replays
//! messages into the new Gateway's inbound queue.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::InboundRequest;

/// Shared state for config-triggered gateway rebuild stash mode.
///
/// Holds a boolean flag indicating whether the Gateway is currently
/// undergoing a rebuild, and a FIFO buffer of stashed inbound requests
/// that arrived while the queue was full during rebuild.
pub struct RebuildStash {
    rebuild_mode: AtomicBool,
    stash: Mutex<VecDeque<InboundRequest>>,
}

impl RebuildStash {
    pub(crate) fn new() -> Self {
        Self {
            rebuild_mode: AtomicBool::new(false),
            stash: Mutex::new(VecDeque::new()),
        }
    }

    /// Enter or exit rebuild mode.
    ///
    /// When `enabled` is `true`, inbound queue-full hits stash the
    /// message instead of rejecting it.  The Daemon calls this before
    /// and after the rebuild cycle.
    pub fn set_rebuild_mode(&self, enabled: bool) {
        self.rebuild_mode.store(enabled, Ordering::Release);
    }

    /// Returns `true` if the Gateway is currently in rebuild mode.
    pub(crate) fn is_rebuild_mode(&self) -> bool {
        self.rebuild_mode.load(Ordering::Acquire)
    }

    /// Append a stashed request to the back of the buffer.
    pub(crate) fn push(&self, request: InboundRequest) {
        match self.stash.lock() {
            Ok(mut guard) => guard.push_back(request),
            Err(poisoned) => {
                tracing::error!(
                    error = %poisoned,
                    "RebuildStash::push: mutex poisoned — message lost"
                );
            }
        }
    }

    /// Drain all stashed requests in FIFO order and return them.
    ///
    /// Called by the Daemon after the rebuild completes to replay
    /// messages into the new Gateway.
    pub(crate) fn take_stashed(&self) -> Vec<InboundRequest> {
        match self.stash.lock() {
            Ok(mut guard) => guard.drain(..).collect(),
            Err(poisoned) => {
                tracing::error!(
                    error = %poisoned,
                    "RebuildStash::take_stashed: mutex poisoned — returning empty"
                );
                Vec::new()
            }
        }
    }
}
