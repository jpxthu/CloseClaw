//! Agent configuration types — config.json and permissions.json structures
//! for per-agent config files.
//!
//! Migrated from `closeclaw-common::agent_config`.
//! Design: `docs/agent/MULTI_AGENT_ARCHITECTURE.md`

mod core;
mod permissions;

pub use core::*;
pub use permissions::*;

#[cfg(test)]
mod agent_config_tests;

#[cfg(test)]
mod tests;
