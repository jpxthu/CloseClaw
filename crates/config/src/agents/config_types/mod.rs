//! Agent configuration types — config.json and permissions.json structures
//! for per-agent config files.
//!
//! Migrated from `closeclaw-common::agent_config`.
//! Design: `docs/agent/MULTI_AGENT_ARCHITECTURE.md`

#[cfg(test)]
use std::collections::HashMap;

mod core;
mod memory_types;
mod model_spec;
mod permissions;

pub use core::*;
pub use memory_types::*;
pub use model_spec::*;
pub use permissions::*;

#[cfg(test)]
mod tests;
