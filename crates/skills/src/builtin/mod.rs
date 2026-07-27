//! Built-in skills - file_ops, git_ops, search, etc.

pub mod discovery;
pub mod file_ops;
#[cfg(test)]
mod file_ops_tests;
pub mod git_ops;
pub mod search;
#[cfg(test)]
pub mod tests;

pub use discovery::SkillDiscoverySkill;
pub use file_ops::FileOpsSkill;
pub use git_ops::GitOpsSkill;
pub use search::SearchSkill;

use crate::registry::Skill;
use std::sync::Arc;

/// Built-in skills registry
pub struct BuiltinSkills;

impl BuiltinSkills {
    /// Create all built-in skills.
    pub fn all() -> Vec<Arc<dyn Skill>> {
        vec![
            Arc::new(FileOpsSkill::new()) as Arc<dyn Skill>,
            Arc::new(GitOpsSkill::new()),
            Arc::new(SearchSkill::new()),
            Arc::new(SkillDiscoverySkill::new()),
            Arc::new(crate::CodingAgentSkill::new()),
            Arc::new(crate::SkillCreatorSkill::new()),
        ]
    }
}

/// Get all built-in skills.
pub fn builtin_skills() -> Vec<Arc<dyn Skill>> {
    BuiltinSkills::all()
}

#[cfg(test)]
mod extra_tests {
    use super::*;

    #[test]
    fn test_builtin_skills_all_returns_six_skills() {
        let skills = BuiltinSkills::all();
        assert_eq!(skills.len(), 6);
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
        assert!(names.iter().any(|n| n == "skill_discovery"));
    }

    #[test]
    fn test_builtin_skills_function() {
        let skills = builtin_skills();
        assert_eq!(skills.len(), 6);
    }

    #[test]
    fn test_builtin_skills_all_have_body() {
        let skills = BuiltinSkills::all();
        for skill in &skills {
            let body = skill.body();
            assert!(
                !body.is_empty(),
                "skill '{}' body should not be empty",
                skill.manifest().name
            );
        }
    }
}
