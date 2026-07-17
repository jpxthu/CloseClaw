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

/// Metadata for a child session tracked by the parent.
#[derive(Debug, Clone)]
pub struct ChildSessionInfo {
    pub session_id: String,
    pub parent_session_id: String,
    pub agent_id: String,
    pub depth: u32,
    pub mode: SpawnMode,
    pub status: ChildSessionStatus,
}

/// Spawn mode for child sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnMode {
    /// One-shot: child runs one LLM turn then completes.
    Run,
    /// Persistent: child stays alive for subsequent steering.
    Session,
}
