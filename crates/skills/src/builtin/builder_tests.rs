//! Tests for built-in skill assembly (`builtin_skills`).

use std::collections::HashSet;
use std::sync::Arc;

use super::builder::{builtin_skills, trivial_validator};
use super::create_workflow::WorkflowDefinitionValidator;

/// Write content to a temp SKILL.md file and return its handle.
fn write_temp_content(content: &str) -> tempfile::NamedTempFile {
    use std::io::Write;
    let mut f = tempfile::Builder::new()
        .suffix(".md")
        .tempfile()
        .expect("failed to create temp file");
    f.write_all(content.as_bytes())
        .expect("failed to write temp file");
    f
}

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

    let invalid = write_temp_content(&format!("---\n{MARKER}\n---\n"));
    let path = invalid.path().to_str().unwrap();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": path
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        v["valid"], false,
        "probe validator must reject marker content"
    );
    assert_eq!(
        v["error"].as_str().unwrap(),
        PROBE_ERROR,
        "rejection must come from the validator passed to builtin_skills"
    );

    let valid = write_temp_content("---\nid: clean\nname: Clean\n---\n");
    let path = valid.path().to_str().unwrap();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": path
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        v["valid"], true,
        "probe validator must accept clean content"
    );
}
