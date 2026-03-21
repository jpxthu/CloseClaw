//! Skills - Reusable tool capabilities for agents
//!
//! Skills are pluggable modules that agents can use to perform actions.

pub mod registry;
pub mod builtin;
pub mod coding_agent;
pub mod skill_creator;

pub use registry::{Skill, SkillRegistry, SkillError, SkillManifest, SkillInput, SkillOutput};
pub use builtin::builtin_skills;
pub use coding_agent::CodingAgentSkill;
pub use skill_creator::SkillCreatorSkill;
