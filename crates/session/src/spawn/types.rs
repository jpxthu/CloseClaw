//! Shared types for spawn operations.

/// Status of a child session tracked by the parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildSessionStatus {
    /// Child session is currently active.
    Active,
    /// Child session has completed successfully.
    Completed,
    /// Child session has been terminated.
    Terminated,
}

impl ChildSessionStatus {
    /// Returns `true` if this status represents a terminal (non-active) state.
    ///
    /// Note: `ChildSessionStatus` in SpawnTree has only `Active`, `Completed`,
    /// and `Terminated`. The `Errored` variant exists only in
    /// `ConversationSession`'s `ChildSessionState` (design doc is silent on
    /// this distinction; retained as-is).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Terminated)
    }
}

/// Metadata for a child session tracked by the parent.
#[derive(Debug, Clone)]
pub struct ChildSessionInfo {
    pub session_id: String,
    pub parent_session_id: String,
    pub agent_id: String,
    pub depth: u32,
    pub mode: SpawnMode,
    pub status: ChildSessionStatus,
    /// Spawn timeout in seconds, if configured.
    pub timeout_secs: Option<u64>,
    /// Timeout warning duration (seconds) resolved for this child's agent.
    /// `None` means legacy single warning 60s before hard timeout.
    pub timeout_warning_secs: Option<u64>,
    /// Interval ratio for cyclic warning notifications.
    pub timeout_notify_interval_ratio: Option<f64>,
    /// Wall-clock instant when this child session was created.
    /// Used by yield timeout to compute elapsed time per child.
    pub created_at: std::time::Instant,
}

/// Spawn mode for child sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnMode {
    /// One-shot: child runs one LLM turn then completes.
    Run,
    /// Persistent: child stays alive for subsequent steering.
    Session,
}
