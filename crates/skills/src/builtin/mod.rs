//! Built-in skills - file_ops, git_ops, search, etc.

pub mod create_workflow;
#[cfg(test)]
mod create_workflow_tests;
pub mod discovery;
#[cfg(test)]
mod discovery_tests;
pub mod file_ops;
#[cfg(test)]
mod file_ops_tests;
pub mod git_ops;
#[cfg(test)]
mod git_ops_tests;
pub mod search;
#[cfg(test)]
mod search_tests;
#[cfg(test)]
mod tests;

pub use create_workflow::WorkflowCreatorSkill;
pub use create_workflow::WorkflowDefinitionValidator;
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
    ///
    /// `workflow_validator` resolves workflow definition content for
    /// the `create_workflow` skill; it is injected by the composition
    /// root (no silent fallback).
    pub fn all(workflow_validator: WorkflowDefinitionValidator) -> Vec<Arc<dyn Skill>> {
        vec![
            Arc::new(FileOpsSkill::new()) as Arc<dyn Skill>,
            Arc::new(GitOpsSkill::new()),
            Arc::new(SearchSkill::new()),
            Arc::new(SkillDiscoverySkill::new()),
            Arc::new(crate::CodingAgentSkill::new()),
            Arc::new(crate::SkillCreatorSkill::new()),
            Arc::new(WorkflowCreatorSkill::new(workflow_validator)),
        ]
    }
}

/// Get all built-in skills.
pub fn builtin_skills(workflow_validator: WorkflowDefinitionValidator) -> Vec<Arc<dyn Skill>> {
    BuiltinSkills::all(workflow_validator)
}

#[cfg(test)]
mod extra_tests {
    use super::*;

    fn trivial_validator() -> WorkflowDefinitionValidator {
        Arc::new(|_| Ok(()))
    }

    #[test]
    fn test_builtin_skills_all_returns_seven_skills() {
        let skills = BuiltinSkills::all(trivial_validator());
        assert_eq!(skills.len(), 7);
    }

    #[test]
    fn test_builtin_skills_all_have_manifests() {
        let skills = BuiltinSkills::all(trivial_validator());
        for skill in &skills {
            let m = skill.manifest();
            assert!(
                !m.name.is_empty(),
                "skill manifest name should not be empty"
            );
            assert!(
                !m.description.is_empty(),
                "skill manifest description should not be empty"
            );
        }
    }

    #[test]
    fn test_builtin_skills_names() {
        let skills = BuiltinSkills::all(trivial_validator());
        let names: Vec<String> = skills.iter().map(|s| s.manifest().name.clone()).collect();
        assert!(names.iter().any(|n| n == "file_ops"));
        assert!(names.iter().any(|n| n == "git_ops"));
        assert!(names.iter().any(|n| n == "search"));
        assert!(names.iter().any(|n| n == "skill_discovery"));
    }

    #[test]
    fn test_builtin_skills_function() {
        let skills = builtin_skills(trivial_validator());
        assert_eq!(skills.len(), 7);
    }

    #[test]
    fn test_builtin_skills_all_have_body() {
        let skills = BuiltinSkills::all(trivial_validator());
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
