//! Agent Configuration — re-exports of common config types used by the
//! agent crate (communication rules, bootstrap mode) plus `AgentType`.
//!
//! Raw per-agent config structures (`AgentConfig`, permissions family)
//! live in the config crate; resolved/shared config types live in
//! `closeclaw_common`.

pub use closeclaw_common::communication::{
    check_communication_allowed, CommunicationCheckResult, CommunicationConfig,
};
pub use closeclaw_common::BootstrapMode;

pub mod agent_type;
pub use agent_type::{AgentType, AgentTypeError};

#[cfg(test)]
mod config_tests;
