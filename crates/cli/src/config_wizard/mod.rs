//! Config Wizard - interactive CLI configuration flow

pub mod ensure_agent;
pub mod fetch;
pub mod parse;
pub mod types;
pub mod wizard;
pub mod write_config;

pub use fetch::*;
pub use parse::parse_model_selection;
pub use types::*;
pub use wizard::run_wizard;
pub use write_config::{write_wizard_config, write_wizard_config_to};

pub(crate) use ensure_agent::ensure_master_agent;

// Test-only re-exports: child test modules resolve these helpers via
// `use super::{...}`; non-test builds have no in-crate users.
#[cfg(test)]
pub(crate) use parse::{compute_default_selection, format_discovery_source_suffix};

// Test-glob dependencies: child test modules resolve these external types via
// `use super::*;` — keep the bindings alive after the functions moved out.
#[cfg(test)]
use closeclaw_config::{
    agents::{AgentConfig, AgentsConfig},
    providers::models::{ModelDefinition, ModelsConfigData, ProviderConfig},
};
#[cfg(test)]
use closeclaw_llm::model_info::ModelInfo;

#[cfg(test)]
mod compute_default_selection_tests;
#[cfg(test)]
mod default_agent_id_same_source_tests;
#[cfg(test)]
mod discovery_source_suffix_tests;
#[cfg(test)]
mod parse_model_selection_tests;
#[cfg(test)]
mod provider_config_protocol_tests;
#[cfg(test)]
mod provider_info_tests;
#[cfg(test)]
mod reexport_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod wizard_context_tests;
