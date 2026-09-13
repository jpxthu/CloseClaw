//! Tests for the create_workflow bundled skill.
//!
//! Full test suite is added in Step 1.2.

use super::create_workflow::WorkflowCreatorSkill;
use crate::registry::Skill;

#[test]
fn test_manifest_name() {
    let skill = WorkflowCreatorSkill::new();
    assert_eq!(skill.manifest().name, "create_workflow");
}

#[test]
fn test_manifest_when_to_use_not_empty() {
    let skill = WorkflowCreatorSkill::new();
    assert!(!skill.manifest().when_to_use.is_empty());
}

#[test]
fn test_manifest_user_invocable() {
    let skill = WorkflowCreatorSkill::new();
    assert!(skill.manifest().user_invocable);
}

#[test]
fn test_body_not_empty() {
    let skill = WorkflowCreatorSkill::new();
    assert!(!skill.body().is_empty());
}
