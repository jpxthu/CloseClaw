//! Default validator registry per config section.

use crate::manager::ConfigSection;
use crate::SectionValidator;

use super::accounts::validate_accounts;
use super::channels::validate_channels;
use super::memory::validate_memory;
use super::models::validate_models;
use super::sections::{
    validate_agents, validate_credentials, validate_gateway, validate_media, validate_plugins,
    validate_skills, validate_system,
};
use super::session::validate_session;
use super::tools::validate_tools;

/// Build the default `SectionValidator` for a given config section.
pub fn for_section(section: ConfigSection) -> Box<SectionValidator> {
    match section {
        ConfigSection::Models => Box::new(validate_models),
        ConfigSection::Channels => Box::new(validate_channels),
        ConfigSection::Gateway => Box::new(validate_gateway),
        ConfigSection::Plugins => Box::new(validate_plugins),
        ConfigSection::System => Box::new(validate_system),
        ConfigSection::Session => Box::new(validate_session),
        ConfigSection::Credentials => Box::new(validate_credentials),
        ConfigSection::Accounts => Box::new(|v| validate_accounts(v, None)),
        ConfigSection::Agents => Box::new(validate_agents),
        ConfigSection::Memory => Box::new(validate_memory),
        ConfigSection::Skills => Box::new(validate_skills),
        ConfigSection::Media => Box::new(validate_media),
        ConfigSection::Tools => Box::new(validate_tools),
    }
}

impl ConfigSection {
    /// Return the default structural validator for this section.
    pub fn default_validator(&self) -> Box<SectionValidator> {
        for_section(*self)
    }
}
