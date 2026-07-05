//! Agent Configuration - config.json and permissions.json structures for per-agent config files.
//!
//! Design: `docs/agent/MULTI_AGENT_ARCHITECTURE.md`

pub use crate::communication::{
    check_communication_allowed, CommunicationCheckResult, CommunicationConfig,
};
pub use closeclaw_common::BootstrapMode;
pub use closeclaw_config::agents::{
    ActionPermission, AgentConfig, AgentPermissions, MemoryConfig, PermissionLimits,
    SubagentsConfig,
};

#[cfg(test)]
mod config_tests;

#[cfg(test)]
mod config_intersect_tests;
