//! Skills - Reusable tool capabilities for agents
//!
//! Skills are pluggable modules that agents can use to perform actions.

pub mod builtin;
pub mod coding_agent;
pub mod registry;
pub mod skill_creator;

pub use builtin::{builtin_skills, builtin_skills_with_engine};
pub use coding_agent::CodingAgentSkill;
pub use registry::{Skill, SkillError, SkillInput, SkillManifest, SkillOutput, SkillRegistry};
pub use skill_creator::SkillCreatorSkill;
