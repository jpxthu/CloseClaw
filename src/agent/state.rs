//! Agent State Machine - lifecycle states, transitions, checkpoints, and pause points.
//!
//! See: `docs/agent/AGENT_LIFECYCLE_STATE_MACHINE.md`

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Agent lifecycle state with semantic details.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum AgentState {
    /// Initial state, agent created but not started.
    Idle,
    /// Actively processing a task.
    Running,
    /// Waiting for external response (user input, child agent result).
    Waiting,
    /// Paused state with a reason.
    Suspended(SuspendedReason),
    /// Completed normally or intentionally stopped.
    Stopped,
    /// Crashed with error details.
    Error(ErrorInfo),
}

impl fmt::Display for AgentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentState::Idle => write!(f, "idle"),
            AgentState::Running => write!(f, "running"),
            AgentState::Waiting => write!(f, "waiting"),
            AgentState::Suspended(r) => write!(f, "suspended({:?})", r),
            AgentState::Stopped => write!(f, "stopped"),
            AgentState::Error(e) => write!(f, "error({})", e.message),
        }
    }
}

/// Reason an agent is suspended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuspendedReason {
    /// Forced suspension by scheduler due to resource exhaustion or time slice expiry.
    /// Must resume from checkpoint.
    Forced,
    /// Voluntary suspension requested by the agent itself (waiting for user input or event).
    /// Can seamlessly continue from pause point.
    SelfRequested,
}

/// Error information attached to the Error state.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ErrorInfo {
    /// Human-readable error description.
    pub message: String,
    /// Whether the agent can recover from this error via resume.
    pub recoverable: bool,
}

impl ErrorInfo {
    /// Create a new error info.
    pub fn new(message: impl Into<String>, recoverable: bool) -> Self {
        Self {
            message: message.into(),
            recoverable,
        }
    }
}

/// What triggered a state transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionTrigger {
    /// User-initiated action.
    UserRequest,
    /// System shutdown signal.
    SystemShutdown,
    /// An error caused the transition.
    Error,
    /// Parent agent triggered cascade.
    ParentCascade,
    /// Scheduler-initiated action.
    Scheduler,
}

/// A state transition event emitted when an agent changes state.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentStateTransition {
    /// State before transition.
    pub from_state: AgentState,
    /// State after transition.
    pub to_state: AgentState,
    /// What triggered this transition.
    pub trigger: TransitionTrigger,
    /// When this transition occurred.
    pub timestamp: DateTime<Utc>,
}

impl AgentStateTransition {
    /// Create a new transition event.
    pub fn new(from_state: AgentState, to_state: AgentState, trigger: TransitionTrigger) -> Self {
        Self {
            from_state,
            to_state,
            trigger,
            timestamp: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Checkpoint and Pause Point
// ---------------------------------------------------------------------------

/// Location in source code for checkpoint/pause point.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SourceLocation {
    /// Function name.
    pub function: String,
    /// Source file path.
    pub file: String,
    /// Line number.
    pub line: u32,
}

impl SourceLocation {
    /// Create a new source location.
    pub fn new(function: impl Into<String>, file: impl Into<String>, line: u32) -> Self {
        Self {
            function: function.into(),
            file: file.into(),
            line,
        }
    }
}

/// Checkpoint for forced suspension — captures everything needed to resume from where the agent left off.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Checkpoint {
    /// Unique checkpoint ID.
    pub id: String,
    /// Agent ID this checkpoint belongs to.
    pub agent_id: String,
    /// Current execution location.
    pub location: SourceLocation,
    /// Serialized snapshot of key local variables (JSON string).
    pub variables_json: String,
    /// Parent agent ID at time of checkpoint.
    pub parent_id: Option<String>,
    /// When this checkpoint was created.
    pub created_at: DateTime<Utc>,
}

/// Pause point for self-requested suspension — captures call stack and execution position.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PausePoint {
    /// Unique pause point ID.
    pub id: String,
    /// Agent ID this pause point belongs to.
    pub agent_id: String,
    /// Current execution location.
    pub location: SourceLocation,
    /// Serialized call stack snapshot (JSON string).
    pub call_stack_json: String,
    /// Parent agent ID at time of pause.
    pub parent_id: Option<String>,
    /// When this pause point was created.
    pub created_at: DateTime<Utc>,
}

/// Compute the base directory for agent persistence.
pub fn agent_base_dir(agent_id: &str) -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".closeclaw")
        .join("agents")
        .join(agent_id)
}

/// Save a checkpoint to disk.
pub fn save_checkpoint(checkpoint: &Checkpoint) -> std::io::Result<std::path::PathBuf> {
    let base = agent_base_dir(&checkpoint.agent_id);
    let dir = base.join("checkpoints");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", checkpoint.id));
    let content = serde_json::to_string_pretty(checkpoint)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&path, content)?;
    Ok(path)
}

/// Load a checkpoint from disk.
pub fn load_checkpoint(agent_id: &str, checkpoint_id: &str) -> std::io::Result<Checkpoint> {
    let path = agent_base_dir(agent_id)
        .join("checkpoints")
        .join(format!("{}.json", checkpoint_id));
    let content = std::fs::read_to_string(&path)?;
    serde_json::from_str(&content)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// List all checkpoints for an agent.
pub fn list_checkpoints(agent_id: &str) -> std::io::Result<Vec<Checkpoint>> {
    let dir = agent_base_dir(agent_id).join("checkpoints");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut checkpoints = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            let content = std::fs::read_to_string(&path)?;
            if let Ok(cp) = serde_json::from_str::<Checkpoint>(&content) {
                checkpoints.push(cp);
            }
        }
    }
    Ok(checkpoints)
}

/// Save a pause point to disk.
pub fn save_pause_point(pause_point: &PausePoint) -> std::io::Result<std::path::PathBuf> {
    let base = agent_base_dir(&pause_point.agent_id);
    let dir = base.join("pause_points");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", pause_point.id));
    let content = serde_json::to_string_pretty(pause_point)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&path, content)?;
    Ok(path)
}

/// Load a pause point from disk.
pub fn load_pause_point(agent_id: &str, pause_id: &str) -> std::io::Result<PausePoint> {
    let path = agent_base_dir(agent_id)
        .join("pause_points")
        .join(format!("{}.json", pause_id));
    let content = std::fs::read_to_string(&path)?;
    serde_json::from_str(&content)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// List all pause points for an agent.
pub fn list_pause_points(agent_id: &str) -> std::io::Result<Vec<PausePoint>> {
    let dir = agent_base_dir(agent_id).join("pause_points");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut pause_points = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            let content = std::fs::read_to_string(&path)?;
            if let Ok(pp) = serde_json::from_str::<PausePoint>(&content) {
                pause_points.push(pp);
            }
        }
    }
    Ok(pause_points)
}

/// Result of a destroy operation when confirmation is required.
#[derive(Debug, Clone)]
pub struct DestroyConfirmation {
    /// The agent that would be destroyed.
    pub agent_id: String,
    /// Human-readable confirmation message.
    pub message: String,
    /// Token to be used to confirm the destroy operation.
    pub confirm_token: String,
}

// ---------------------------------------------------------------------------
// State transition validation
// ---------------------------------------------------------------------------

/// Check if a state transition is valid.
pub fn is_valid_transition(from: &AgentState, to: &AgentState) -> bool {
    use AgentState::*;
    match (from, to) {
        // Idle can go to Running, Suspended, Stopped, or Error
        (Idle, Running) | (Idle, Suspended(_)) | (Idle, Stopped) | (Idle, Error(_)) => true,
        // Running can go to Waiting, Suspended, Stopped, or Error
        (Running, Waiting)
        | (Running, Suspended(_))
        | (Running, Stopped)
        | (Running, Error(_)) => true,
        // Waiting can go back to Running, or to Suspended, Stopped, Error
        (Waiting, Running)
        | (Waiting, Suspended(_))
        | (Waiting, Stopped)
        | (Waiting, Error(_)) => true,
        // Suspended can go to Running or Stopped
        (Suspended(_), Running) | (Suspended(_), Stopped) => true,
        // Error can go to Running if recoverable (resume after error)
        (Error(e), Running) if e.recoverable => true,
        // Stopped is terminal
        (Stopped, _) => false,
        // Error is terminal unless recoverable
        (Error(_), _) => false,
        // Same state is always valid (no-op)
        _ if from == to => true,
        // Default: invalid
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_state_display() {
        assert_eq!(AgentState::Idle.to_string(), "idle");
        assert_eq!(AgentState::Running.to_string(), "running");
        assert_eq!(AgentState::Waiting.to_string(), "waiting");
        assert_eq!(
            AgentState::Suspended(SuspendedReason::Forced).to_string(),
            "suspended(Forced)"
        );
        assert_eq!(
            AgentState::Suspended(SuspendedReason::SelfRequested).to_string(),
            "suspended(SelfRequested)"
        );
        assert_eq!(AgentState::Stopped.to_string(), "stopped");
        assert_eq!(
            AgentState::Error(ErrorInfo::new("oops", false)).to_string(),
            "error(oops)"
        );
    }

    #[test]
    fn test_error_info() {
        let e = ErrorInfo::new("test error", true);
        assert_eq!(e.message, "test error");
        assert!(e.recoverable);

        let e2 = ErrorInfo::new("fatal", false);
        assert!(!e2.recoverable);
    }

    #[test]
    fn test_transition_trigger() {
        let t = AgentStateTransition::new(
            AgentState::Idle,
            AgentState::Running,
            TransitionTrigger::UserRequest,
        );
        assert!(matches!(t.from_state, AgentState::Idle));
        assert!(matches!(t.to_state, AgentState::Running));
        assert!(matches!(t.trigger, TransitionTrigger::UserRequest));
    }

    #[test]
    fn test_valid_transitions() {
        use AgentState::*;
        // Idle
        assert!(is_valid_transition(&Idle, &Running));
        assert!(is_valid_transition(&Idle, &Error(ErrorInfo::new("fail", false))));
        // Running
        assert!(is_valid_transition(&Running, &Waiting));
        assert!(is_valid_transition(&Running, &Suspended(SuspendedReason::Forced)));
        assert!(is_valid_transition(&Running, &Stopped));
        assert!(is_valid_transition(&Running, &Error(ErrorInfo::new("crash", false))));
        // Waiting
        assert!(is_valid_transition(&Waiting, &Running));
        assert!(is_valid_transition(&Waiting, &Suspended(SuspendedReason::Forced)));
        // Suspended
        assert!(is_valid_transition(&Suspended(SuspendedReason::Forced), &Running));
        assert!(is_valid_transition(&Suspended(SuspendedReason::SelfRequested), &Running));
        assert!(is_valid_transition(&Suspended(SuspendedReason::SelfRequested), &Stopped));
        // Terminal states
        assert!(!is_valid_transition(&Stopped, &Running));
        // Error (non-recoverable)
        let fatal = ErrorInfo::new("fatal", false);
        assert!(!is_valid_transition(&Error(fatal.clone()), &Running));
        // Error (recoverable)
        let recoverable = ErrorInfo::new("recoverable", true);
        assert!(is_valid_transition(&Error(recoverable), &Running));
        // Same state
        assert!(is_valid_transition(&Running, &Running));
    }

    #[test]
    fn test_source_location() {
        let loc = SourceLocation::new("my_func", "/path/to/file.rs", 42);
        assert_eq!(loc.function, "my_func");
        assert_eq!(loc.file, "/path/to/file.rs");
        assert_eq!(loc.line, 42);
    }

    #[test]
    fn test_checkpoint_roundtrip() {
        let cp = Checkpoint {
            id: "cp-1".to_string(),
            agent_id: "agent-1".to_string(),
            location: SourceLocation::new("run", "main.rs", 10),
            variables_json: r#"{"counter": 42}"#.to_string(),
            parent_id: Some("parent-1".to_string()),
            created_at: Utc::now(),
        };

        let json = serde_json::to_string_pretty(&cp).unwrap();
        let loaded: Checkpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.id, cp.id);
        assert_eq!(loaded.agent_id, cp.agent_id);
        assert_eq!(loaded.variables_json, cp.variables_json);
    }

    #[test]
    fn test_pause_point_roundtrip() {
        let pp = PausePoint {
            id: "pp-1".to_string(),
            agent_id: "agent-1".to_string(),
            location: SourceLocation::new("run", "main.rs", 20),
            call_stack_json: r#"["fn a()", "fn b()"]"#.to_string(),
            parent_id: Some("parent-1".to_string()),
            created_at: Utc::now(),
        };

        let json = serde_json::to_string_pretty(&pp).unwrap();
        let loaded: PausePoint = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.id, pp.id);
        assert_eq!(loaded.call_stack_json, pp.call_stack_json);
    }
}
