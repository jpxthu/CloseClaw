//! Built-in skill assembly - constructs the bundled skill list.

use super::create_workflow::{WorkflowCreatorSkill, WorkflowDefinitionValidator};
use super::discovery::SkillDiscoverySkill;
use super::file_ops::FileOpsSkill;
use super::git_ops::GitOpsSkill;
use super::search::SearchSkill;
use crate::coding_agent::CodingAgentSkill;
use crate::registry::Skill;
use crate::skill_creator::SkillCreatorSkill;
use std::sync::Arc;

/// Create all built-in skills.
///
/// `workflow_validator` resolves workflow definition content for
/// the `create_workflow` skill; it is injected by the composition
/// root (no silent fallback).
pub fn builtin_skills(workflow_validator: WorkflowDefinitionValidator) -> Vec<Arc<dyn Skill>> {
    let skills: Vec<Arc<dyn Skill>> = vec![
        Arc::new(FileOpsSkill::new()),
        Arc::new(GitOpsSkill::new()),
        Arc::new(SearchSkill::new()),
        Arc::new(SkillDiscoverySkill::new()),
        Arc::new(CodingAgentSkill::new()),
        Arc::new(SkillCreatorSkill::new()),
        Arc::new(WorkflowCreatorSkill::new(workflow_validator)),
    ];
    skills
}

/// Validator accepting any content, for tests that do not exercise
/// `create_workflow` validate semantics.
#[cfg(test)]
pub(crate) fn trivial_validator() -> WorkflowDefinitionValidator {
    Arc::new(|_| Ok(()))
}
