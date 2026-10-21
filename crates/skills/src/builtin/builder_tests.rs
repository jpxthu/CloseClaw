//! Tests for built-in skill assembly (`builtin_skills`).

use std::collections::HashSet;
use std::sync::Arc;

use super::builder::{builtin_skills, trivial_validator};
use super::create_workflow::WorkflowDefinitionValidator;
use super::create_workflow_tests::write_temp_skill_md;
use crate::registry::Skill;

#[test]
fn test_builtin_skills_returns_seven_expected_skills() {
    let skills = builtin_skills(trivial_validator());
    assert_eq!(skills.len(), 7);

    let mut names: Vec<String> = skills.iter().map(|s| s.manifest().name.clone()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "coding_agent",
            "create_workflow",
            "file_ops",
            "git_ops",
            "search",
            "skill_creator",
            "skill_discovery"
        ]
    );

    let unique: HashSet<&str> = names.iter().map(String::as_str).collect();
    assert_eq!(unique.len(), 7, "builtin skill names should be unique");
}

#[test]
fn test_builtin_skills_all_have_manifests() {
    let skills = builtin_skills(trivial_validator());
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
fn test_builtin_skills_all_have_body() {
    let skills = builtin_skills(trivial_validator());
    for skill in &skills {
        let body = skill.body();
        assert!(
            !body.is_empty(),
            "skill '{}' body should not be empty",
            skill.manifest().name
        );
    }
}

#[tokio::test]
async fn test_builtin_skills_wires_workflow_validator_into_create_workflow() {
    /// Drive `create_workflow` validate action through the assembled
    /// skill and parse the JSON result.
    async fn validate_via_builtin(skill: &dyn Skill, path: &str) -> serde_json::Value {
        let result = skill
            .execute(Some(serde_json::json!({
                "action": "validate",
                "path": path
            })))
            .await
            .unwrap();
        serde_json::from_str(&result).unwrap()
    }

    const MARKER: &str = "wiring probe invalid marker";
    const PROBE_ERROR: &str = "wiring probe: invalid workflow definition";
    let probe: WorkflowDefinitionValidator = Arc::new(|content: &str| {
        if content.contains(MARKER) {
            Err(PROBE_ERROR.to_string())
        } else {
            Ok(())
        }
    });

    let skills = builtin_skills(probe);
    let skill = skills
        .iter()
        .find(|s| s.manifest().name == "create_workflow")
        .expect("builtin_skills should assemble create_workflow");

    let invalid = write_temp_skill_md(&format!("---\n{MARKER}\n---\n"));
    let path = invalid.path().to_str().unwrap();
    let v = validate_via_builtin(skill.as_ref(), path).await;
    assert_eq!(
        v["valid"], false,
        "probe validator must reject marker content"
    );
    assert_eq!(
        v["error"].as_str().unwrap(),
        PROBE_ERROR,
        "rejection must come from the validator passed to builtin_skills"
    );

    let valid = write_temp_skill_md("---\nid: clean\nname: Clean\n---\n");
    let path = valid.path().to_str().unwrap();
    let v = validate_via_builtin(skill.as_ref(), path).await;
    assert_eq!(
        v["valid"], true,
        "probe validator must accept clean content"
    );
}
