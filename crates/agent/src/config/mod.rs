//! Agent Configuration - config.json and permissions.json structures for per-agent config files.
//!
//! Design: `docs/agent/MULTI_AGENT_ARCHITECTURE.md`

pub use closeclaw_common::agent_config::{
    ActionPermission, ActiveSearcherOverride, AgentConfig, AgentPermissions, MemoryConfig,
    PermissionLimits, SubagentsConfig,
};
pub use closeclaw_common::BootstrapMode;
pub use closeclaw_common::{
    check_communication_allowed, CommunicationCheckResult, CommunicationConfig,
};

#[cfg(test)]
mod config_tests;

#[cfg(test)]
mod config_intersect_tests;
