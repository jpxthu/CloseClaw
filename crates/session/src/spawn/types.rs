//! Shared types for spawn operations.

use closeclaw_config::agents::ResolvedAgentConfig;

/// Result of a successful spawn validation, containing the resolved
/// target config and the effective max spawn depth for the child.
#[derive(Debug, Clone)]
pub struct SpawnValidationResult {
    /// Resolved configuration of the target agent.
    pub config: ResolvedAgentConfig,
    /// Effective max spawn depth the child may use.
    /// Computed as `min(child.max_spawn_depth, parent.max_spawn_depth - 1)`.
    pub effective_max_spawn_depth: u32,
    /// Sub-agent maximum execution duration (seconds), resolved via
    /// priority chain: spawn args → target agent config → global default.
    /// Never `None` after resolution — always falls back to global default.
    pub spawn_timeout: Option<u64>,
    /// Sub-agent timeout warning duration (seconds), resolved via
    /// priority chain: spawn args → target agent config → global default.
    /// `None` means legacy single warning 60s before hard timeout.
    pub timeout_warning_secs: Option<u64>,
    /// Interval ratio for cyclic warning notifications (relative to timeout_warning).
    /// Must be >=0.1 and <=2.0, default 0.5. `None` means use default.
    pub timeout_notify_interval_ratio: Option<f64>,
}

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
