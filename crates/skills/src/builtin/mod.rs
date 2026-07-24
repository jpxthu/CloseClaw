//! Built-in skills - file_ops, git_ops, search, etc.

pub mod discovery;
pub mod file_ops;
#[cfg(test)]
mod file_ops_tests;
pub mod git_ops;
pub mod permission;
pub mod search;
#[cfg(test)]
pub mod tests;

pub use discovery::SkillDiscoverySkill;
pub use file_ops::FileOpsSkill;
pub use git_ops::GitOpsSkill;
pub use permission::PermissionSkill;
pub use search::SearchSkill;

use crate::registry::Skill;
use closeclaw_common::permission_types::{
    SharedSkillApprovalSubmitter, SharedSkillPermissionChecker,
};
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
            Arc::new(crate::CodingAgentSkill::new(None)),
            Arc::new(crate::SkillCreatorSkill::new()),
        ]
    }

    /// Create all built-in skills with a shared permission checker injected.
    pub fn all_with_engine(engine: SharedSkillPermissionChecker) -> Vec<Arc<dyn Skill>> {
        vec![
            Arc::new(FileOpsSkill::with_engine(engine.clone())) as Arc<dyn Skill>,
            Arc::new(GitOpsSkill::new()),
            Arc::new(SearchSkill::new()),
            Arc::new(PermissionSkill::with_engine(engine.clone())),
            Arc::new(SkillDiscoverySkill::with_engine(engine)),
            Arc::new(crate::CodingAgentSkill::new(None)),
            Arc::new(crate::SkillCreatorSkill::new()),
        ]
    }

    /// Create all built-in skills with a shared permission checker and approval
    /// submitter injected.
    pub fn all_with_engine_and_approval_flow(
        engine: SharedSkillPermissionChecker,
        approval_flow: SharedSkillApprovalSubmitter,
    ) -> Vec<Arc<dyn Skill>> {
        let file_ops =
            FileOpsSkill::with_engine_and_approval_flow(engine.clone(), approval_flow.clone());
        let perm_skill = PermissionSkill::with_engine(engine.clone());
        vec![
            Arc::new(file_ops) as Arc<dyn Skill>,
            Arc::new(GitOpsSkill::new()),
            Arc::new(SearchSkill::new()),
            Arc::new(perm_skill),
            Arc::new(SkillDiscoverySkill::with_engine_and_approval_flow(
                engine,
                approval_flow,
            )),
            Arc::new(crate::CodingAgentSkill::new(None)),
            Arc::new(crate::SkillCreatorSkill::new()),
        ]
    }
}

/// Get all built-in skills (without permission engine).
pub fn builtin_skills() -> Vec<Arc<dyn Skill>> {
    BuiltinSkills::all()
}

/// Get all built-in skills with a shared permission checker injected.
pub fn builtin_skills_with_engine(engine: SharedSkillPermissionChecker) -> Vec<Arc<dyn Skill>> {
    BuiltinSkills::all_with_engine(engine)
}

/// Get all built-in skills with a shared permission checker and approval
/// submitter injected.
pub fn builtin_skills_with_engine_and_approval_flow(
    engine: SharedSkillPermissionChecker,
    approval_flow: SharedSkillApprovalSubmitter,
) -> Vec<Arc<dyn Skill>> {
    BuiltinSkills::all_with_engine_and_approval_flow(engine, approval_flow)
}

#[cfg(test)]
mod extra_tests {
    use super::*;

    #[test]
    fn test_builtin_skills_all_returns_seven_skills() {
        let skills = BuiltinSkills::all();
        assert_eq!(skills.len(), 7);
    }

    #[test]
    fn test_builtin_skills_all_have_manifests() {
        let skills = BuiltinSkills::all();
        for skill in &skills {
            let m = skill.manifest();
            assert!(
                !m.name.is_empty(),
                "skill manifest name should not be empty"
            );
            assert!(
                !m.version.is_empty(),
                "skill manifest version should not be empty"
            );
        }
    }

    #[test]
    fn test_builtin_skills_names() {
        let skills = BuiltinSkills::all();
        let names: Vec<String> = skills.iter().map(|s| s.manifest().name.clone()).collect();
        assert!(names.iter().any(|n| n == "file_ops"));
        assert!(names.iter().any(|n| n == "git_ops"));
        assert!(names.iter().any(|n| n == "search"));
        assert!(names.iter().any(|n| n == "permission_query"));
        assert!(names.iter().any(|n| n == "skill_discovery"));
    }

    #[test]
    fn test_builtin_skills_function() {
        let skills = builtin_skills();
        assert_eq!(skills.len(), 7);
    }
}
