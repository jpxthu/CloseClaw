//! Built-in skills - file_ops, git_ops, search, etc.

pub mod discovery;
pub mod file_ops;
pub mod git_ops;
pub mod permission;
pub mod search;
pub mod tests;

pub use discovery::SkillDiscoverySkill;
pub use file_ops::FileOpsSkill;
pub use git_ops::GitOpsSkill;
pub use permission::PermissionSkill;
pub use search::SearchSkill;

use crate::skills::Skill;
use std::sync::Arc;

/// Built-in skills registry
pub struct BuiltinSkills;

impl BuiltinSkills {
    /// Create all built-in skills without a permission engine.
    /// Permission checks will return `{ "allowed": null }` until an engine is injected.
    pub fn all() -> Vec<Arc<dyn Skill>> {
        vec![
            Arc::new(FileOpsSkill::new()) as Arc<dyn Skill>,
            Arc::new(GitOpsSkill::new()),
            Arc::new(SearchSkill::new()),
            Arc::new(PermissionSkill::new()),
            Arc::new(SkillDiscoverySkill::new()),
            Arc::new(super::CodingAgentSkill::new(None)),
            Arc::new(super::SkillCreatorSkill::new()),
        ]
    }

    /// Create all built-in skills with a shared permission engine injected.
    pub fn all_with_engine(
        engine: Arc<crate::permission::PermissionEngine>,
    ) -> Vec<Arc<dyn Skill>> {
        vec![
            Arc::new(FileOpsSkill::with_engine(engine.clone())) as Arc<dyn Skill>,
            Arc::new(GitOpsSkill::new()),
            Arc::new(SearchSkill::new()),
            Arc::new(PermissionSkill::with_engine(engine.clone())),
            Arc::new(SkillDiscoverySkill::with_engine(engine)),
            Arc::new(super::CodingAgentSkill::new(None)),
            Arc::new(super::SkillCreatorSkill::new()),
        ]
    }
}

/// Get all built-in skills (without permission engine).
pub fn builtin_skills() -> Vec<Arc<dyn Skill>> {
    BuiltinSkills::all()
}

/// Get all built-in skills with a shared permission engine injected.
pub fn builtin_skills_with_engine(
    engine: Arc<crate::permission::PermissionEngine>,
) -> Vec<Arc<dyn Skill>> {
    BuiltinSkills::all_with_engine(engine)
}
