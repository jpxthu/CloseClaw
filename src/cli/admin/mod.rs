//! CLI admin module — handler functions for admin subcommands.

mod agent;
mod common;
mod config;
mod rule;
mod run;
mod skill;
mod stop;

pub use agent::{handle_agent, handle_agent_with};
pub use common::{
    config_dir, config_dir_for, mask_key, ConfigListFile, ConfigListOutput, ConfigValidateOutput,
    RuleCheckOutput, RuleListEntry, RuleListOutput, RunOutput, StopOutput,
};
pub use config::{handle_config, handle_config_with, read_config_files};
pub use rule::{handle_rule, handle_rule_with};
pub use run::handle_run;
pub use skill::{handle_skill, handle_skill_with};
pub use stop::handle_stop;

#[cfg(test)]
mod run_tests;
