//! CLI admin module — handler functions for admin subcommands.

mod agent;
mod common;
mod config;
mod rule;
mod skill;
mod stop;

pub use agent::{handle_agent, handle_agent_with};
pub use common::{
    config_dir, config_dir_for, mask_key, pid_file_path, ConfigListFile, ConfigListOutput,
    ConfigValidateOutput, RuleCheckOutput, RuleListEntry, RuleListOutput, StopOutput,
};
pub use config::{handle_config, handle_config_with, read_config_files};
pub use rule::{handle_rule, handle_rule_with};
pub use skill::{handle_skill, handle_skill_with};
pub use stop::handle_stop;
