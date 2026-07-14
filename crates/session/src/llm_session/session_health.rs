//! Health checker integration for `ConversationSession`.
//!
//! Provides dependency injection and accessor methods for the
//! per-session [`RunHealthChecker`]. Extracted from `mod.rs` to keep
//! that file under the CONTRIBUTING.md 1000-line cap.

use std::sync::Arc;

use super::ConversationSession;
use crate::run_health::RunHealthChecker;

/// Health checker injection and accessors.
impl ConversationSession {
    /// Inject a [`RunHealthChecker`] into this session.
    ///
    /// Called by Gateway after session creation so the session can
    /// run health checks at turn boundaries.
    pub fn set_health_checker(&mut self, checker: RunHealthChecker) {
        self.health_checker = Some(Arc::new(tokio::sync::Mutex::new(checker)));
    }

    /// Returns a handle to the health checker, if any.
    pub fn health_checker(&self) -> Option<&Arc<tokio::sync::Mutex<RunHealthChecker>>> {
        self.health_checker.as_ref()
    }
}
