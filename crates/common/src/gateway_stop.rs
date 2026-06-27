//! Gateway stop-related types.
//!
//! Types for session shutdown progress and result tracking, extracted
//! from `src/gateway/session_manager/stop.rs`.

/// Progress event emitted each time a single session stop completes.
#[derive(Debug, Clone)]
pub struct StopProgress {
    /// ID of the session whose stop just completed.
    pub session_id: String,
    /// Whether the session was stopped successfully.
    pub success: bool,
    /// Number of sessions remaining to be stopped after this one.
    pub remaining: usize,
}

/// Aggregated result of stopping all sessions.
#[derive(Debug, Default)]
pub struct StopResult {
    /// Sessions stopped successfully.
    pub succeeded: usize,
    /// Sessions where stop or persist failed.
    pub failed: usize,
    /// Sessions skipped (not found or no ConversationSession).
    pub skipped: usize,
}

impl StopResult {
    /// Total number of sessions processed.
    pub fn total(&self) -> usize {
        self.succeeded + self.failed + self.skipped
    }
}
