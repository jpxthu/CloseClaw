//! Skills - Reusable tool capabilities for agents
//!
//! Skills are pluggable modules that agents can use to perform actions.

pub mod builtin;
pub mod coding_agent;
pub mod disk;
pub mod registry;
pub mod skill_creator;

pub use builtin::builtin_skills;
pub use coding_agent::CodingAgentSkill;
pub use disk::{
    init_disk_skills, resolve_skill, start_skill_watcher, DiskSkillRegistry, ResolvedSkill,
    ScanConfig, SkillWatcherHandle,
};
pub use registry::{BuiltinSkillRegistry, Skill, SkillError, SkillListingMeta, SkillManifest};
pub use skill_creator::SkillCreatorSkill;
