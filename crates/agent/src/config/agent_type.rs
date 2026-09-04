//! Agent type definitions for Plan Mode spawn injection framework.
//!
//! Defines the three agent roles used in Plan Mode:
//! - `Explore` - Read-only codebase exploration (Research phase)
//! - `Plan` - Architecture-level design generation (Design phase)
//! - `Executor` - Full toolset implementation execution (Auto Mode)
//!
//! Design: `docs/design/mode/plan-mode.md` §Agent 类型

use std::fmt;
use std::str::FromStr;

/// Agent type enum for Plan Mode spawn injection.
///
/// Each variant maps to a specific role in the Plan Mode workflow,
/// determining the system prompt template and tool permissions applied
/// when spawning a child agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentType {
    /// Read-only exploration agent. Used in Research phase to explore
    /// the codebase and understand existing implementations.
    Explore,
    /// Architecture-level planning agent. Used in Design phase to
    /// generate implementation plans from an architect's perspective.
    Plan,
    /// Full execution agent. Used in Auto Mode to implement the plan
    /// with the complete toolset (dangerous operations subject to review).
    Executor,
}

impl AgentType {
    /// Returns the prompt prefix string for this agent type.
    pub fn prompt_prefix(&self) -> &'static str {
        match self {
            AgentType::Explore => "explore",
            AgentType::Plan => "plan",
            AgentType::Executor => "executor",
        }
    }
}

impl fmt::Display for AgentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentType::Explore => write!(f, "explore"),
            AgentType::Plan => write!(f, "plan"),
            AgentType::Executor => write!(f, "executor"),
        }
    }
}

impl FromStr for AgentType {
    type Err = AgentTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "explore" => Ok(AgentType::Explore),
            "plan" => Ok(AgentType::Plan),
            "executor" => Ok(AgentType::Executor),
            _ => Err(AgentTypeError::UnknownType(s.to_string())),
        }
    }
}

/// Error type for invalid agent type strings.
#[derive(Debug, Clone, thiserror::Error)]
pub enum AgentTypeError {
    /// The provided string does not match any known agent type.
    #[error("unknown agent type: `{0}`, expected one of: explore, plan, executor")]
    UnknownType(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_roundtrip() {
        let types = [AgentType::Explore, AgentType::Plan, AgentType::Executor];
        for agent_type in types {
            let s = agent_type.to_string();
            let parsed: AgentType = s.parse().unwrap();
            assert_eq!(agent_type, parsed);
        }
    }

    #[test]
    fn from_str_case_insensitive() {
        assert_eq!("EXPLORE".parse::<AgentType>().unwrap(), AgentType::Explore);
        assert_eq!("Plan".parse::<AgentType>().unwrap(), AgentType::Plan);
        assert_eq!(
            "EXECUTOR".parse::<AgentType>().unwrap(),
            AgentType::Executor
        );
    }

    #[test]
    fn from_str_invalid() {
        let err = "invalid".parse::<AgentType>().unwrap_err();
        match err {
            AgentTypeError::UnknownType(s) => assert_eq!(s, "invalid"),
        }
    }
}
