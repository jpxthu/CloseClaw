//! Agent module - pure configuration layer for agent definitions.

pub mod config;
pub mod lookup;
pub mod registry;

pub use closeclaw_common::{AgentSkillsQuery, AgentToolsConfig, AgentToolsConfigQuery};
pub use config::agent_type::{AgentType, AgentTypeError};
pub use lookup::{AgentConfigInfo, AgentConfigLookup, AgentLookup, AgentRegistryQuery};
