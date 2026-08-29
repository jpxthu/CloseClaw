//! Skills - Reusable tool capabilities for agents
//!
//! Skills are pluggable modules that agents can use to perform actions.

pub mod builtin;
pub mod coding_agent;
#[cfg(test)]
mod coding_agent_tests;
pub mod disk;
pub mod registry;
pub mod skill_creator;
#[cfg(test)]
mod skill_creator_tests;
pub mod skills_fragment_provider;
pub mod tool_registrar;
#[cfg(test)]
mod tool_registrar_tests;

pub use builtin::builtin_skills;
pub use coding_agent::CodingAgentSkill;
pub use disk::{init_disk_skills, resolve_skill, DiskSkillRegistry, ResolvedSkill, ScanConfig};
pub use registry::{BuiltinSkillRegistry, Skill, SkillError, SkillListingMeta, SkillManifest};
pub use skill_creator::SkillCreatorSkill;
pub use skills_fragment_provider::SkillsFragmentProvider;
pub use tool_registrar::SkillsToolsRegistrar;
