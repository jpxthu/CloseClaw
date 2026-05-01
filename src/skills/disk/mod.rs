//! Disk Skill System - disk-based skill discovery and loading
//!
//! Provides a file-system based skill discovery mechanism that scans
//! hierarchical directories to find, parse, and load skill definitions.

pub mod frontmatter;
pub mod loader;
pub mod types;

pub use frontmatter::parse_skill_md;
pub use loader::scan_all_skills;
pub use types::{
    DiskSkill, ParseError, ParsedSkill, ScanConfig, SkillContext, SkillEffort, SkillManifest,
    SkillSource,
};
