//! Tests for the create_workflow bundled skill.
//!
//! Covers: manifest, body, execute() normal/error/arg paths,
//! and validate() error paths exercising the 12-rule validator.

use super::create_workflow::WorkflowCreatorSkill;
use crate::registry::{Skill, SkillError};
use std::io::Write;

// ==========================================================================
// Helpers
// ==========================================================================

const SCHEMA_EMPTY: &str = "step_data_schema: {}";

/// Build a minimal but valid SKILL.md content string.
/// Minimal valid single-step workflow (default complete transition only).
fn valid_skill_md() -> String {
    let mut s = String::from("---\nid: test\nname: Test\ndescription: A test workflow\n");
    s.push_str("version: \"0.1\"\n");
    s.push_str("allow_blocked: false\n");
    s.push_str("verify_retry_limit: 3\n");
    s.push_str(SCHEMA_EMPTY);
    s.push_str(
        "\nsteps:\n  - id: 0\n    name: Step One\n    goal: Do something\n    verify:\n      - \"check A\"\n    jump: []\n    transitions:\n      - action: complete\n\n---\n",
    );
    s
}

/// Two-step workflow with a jump question and goto transition.
fn valid_skill_md_two_steps() -> String {
    let mut s = String::from("---\nid: test2\nname: Test Two\ndescription: A two-step workflow\n");
    s.push_str("version: \"0.1\"\n");
    s.push_str("allow_blocked: false\n");
    s.push_str("verify_retry_limit: 3\n");
    s.push_str(SCHEMA_EMPTY);
    s.push_str(
        "\nsteps:\n  - id: 0\n    name: Step One\n    goal: Do something\n    verify:\n      - \"check A\"\n    jump:\n      - id: done\n        prompt: Is it done?\n        type: boolean\n    transitions:\n      - when:\n          done: true\n        action: goto\n        target_step: 1\n      - action: complete\n  - id: 1\n    name: Step Two\n    goal: Do more\n    verify:\n      - \"check B\"\n    jump: []\n    transitions:\n      - action: complete\n\n---\n",
    );
    s
}

/// Write content to a temp file and return its handle.
fn write_temp_skill_md(content: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::Builder::new()
        .suffix(".md")
        .tempfile()
        .expect("failed to create temp file");
    f.write_all(content.as_bytes())
        .expect("failed to write temp file");
    f
}

/// Build a bad SKILL.md YAML with the given steps block substituted in.
fn bad_skill_yaml(steps_block: &str) -> String {
    format!(
        r#"---
id: test
name: Test
description: Bad workflow
version: "0.1"
allow_blocked: false
verify_retry_limit: 3
{schema}
steps:
{steps}
---
"#,
        schema = SCHEMA_EMPTY,
        steps = steps_block
    )
}

// ==========================================================================
// Manifest tests
// ==========================================================================

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
fn test_manifest_effort_valid() {
    let skill = WorkflowCreatorSkill::new();
    let effort_str = skill.manifest().effort.to_string();
    assert!(!effort_str.is_empty(), "effort display should not be empty");
    assert!(
        matches!(
            skill.manifest().effort,
            crate::disk::types::SkillEffort::Small
        ),
        "effort should be Small"
    );
}

#[test]
fn test_manifest_description_not_empty() {
    let skill = WorkflowCreatorSkill::new();
    assert!(!skill.manifest().description.is_empty());
}

// ==========================================================================
// Body tests
// ==========================================================================

#[test]
fn test_body_not_empty() {
    let skill = WorkflowCreatorSkill::new();
    assert!(!skill.body().is_empty());
}

#[test]
fn test_body_contains_frontmatter_guidance() {
    let skill = WorkflowCreatorSkill::new();
    let body = skill.body();
    assert!(
        body.contains("frontmatter"),
        "body should mention frontmatter"
    );
}

#[test]
fn test_body_contains_steps_guidance() {
    let skill = WorkflowCreatorSkill::new();
    let body = skill.body();
    assert!(body.contains("steps"), "body should mention steps");
}

#[test]
fn test_body_contains_transitions_rules() {
    let skill = WorkflowCreatorSkill::new();
    let body = skill.body();
    assert!(
        body.contains("transition"),
        "body should mention transitions"
    );
}

#[test]
fn test_body_contains_writing_principles() {
    let skill = WorkflowCreatorSkill::new();
    let body = skill.body();
    assert!(
        body.contains("Writing Principles"),
        "body should contain Writing Principles section"
    );
}

#[test]
fn test_body_mentions_file_structure() {
    let skill = WorkflowCreatorSkill::new();
    let body = skill.body();
    assert!(
        body.contains("SKILL.md"),
        "body should mention the SKILL.md target file"
    );
}

#[test]
fn test_body_mentions_default_transition_requirement() {
    let skill = WorkflowCreatorSkill::new();
    let body = skill.body();
    assert!(
        body.contains("default"),
        "body should mention default transition requirement"
    );
}

// ==========================================================================
// execute() — normal path
// ==========================================================================

#[tokio::test]
async fn test_execute_none_returns_capabilities() {
    let skill = WorkflowCreatorSkill::new();
    let result = skill.execute(None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "create_workflow");
    let actions = v["supported_actions"].as_array().unwrap();
    assert!(actions.contains(&serde_json::json!("create")));
    assert!(actions.contains(&serde_json::json!("validate")));
}

#[tokio::test]
async fn test_execute_empty_args_returns_capabilities() {
    let skill = WorkflowCreatorSkill::new();
    let result = skill.execute(Some(serde_json::json!({}))).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "create_workflow");
    assert_eq!(v["supported_actions"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_execute_create_returns_guidance_with_target_path() {
    let skill = WorkflowCreatorSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "create",
            "name": "my_workflow",
            "description": "A test workflow"
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "create_workflow");
    assert_eq!(v["action"], "create");
    let target_file = v["target"]["file"].as_str().unwrap();
    assert!(
        target_file.contains("my_workflow"),
        "target file should include workflow name"
    );
    assert!(
        target_file.contains("SKILL.md"),
        "target file should end with SKILL.md"
    );
}

#[tokio::test]
async fn test_execute_create_template_has_key_fields() {
    let skill = WorkflowCreatorSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "create",
            "name": "demo",
            "description": "Demo workflow"
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    let tmpl = &v["frontmatter_template"];
    assert!(tmpl.get("id").is_some(), "template should have id field");
    assert!(
        tmpl.get("steps").is_some(),
        "template should have steps field"
    );
    assert!(
        tmpl.get("allow_blocked").is_some(),
        "template should have allow_blocked field"
    );
    assert!(
        tmpl.get("verify_retry_limit").is_some(),
        "template should have verify_retry_limit field"
    );
}

#[tokio::test]
async fn test_execute_create_with_default_description() {
    let skill = WorkflowCreatorSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "create",
            "name": "no_desc"
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "create");
    assert!(v["target"]["file"].as_str().unwrap().contains("no_desc"));
}

#[tokio::test]
async fn test_execute_validate_valid_definition_passes() {
    let skill = WorkflowCreatorSkill::new();
    let tmp = write_temp_skill_md(&valid_skill_md());
    let path = tmp.path().to_str().unwrap();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": path
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "create_workflow");
    assert_eq!(v["action"], "validate");
    assert_eq!(v["valid"], true);
    assert!(
        v["message"].as_str().unwrap().contains("valid"),
        "message should indicate validity"
    );
}

#[tokio::test]
async fn test_execute_validate_two_step_valid_passes() {
    let skill = WorkflowCreatorSkill::new();
    let tmp = write_temp_skill_md(&valid_skill_md_two_steps());
    let path = tmp.path().to_str().unwrap();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": path
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["valid"], true);
}

// ==========================================================================
// execute() — validate error paths (structural violations)
// ==========================================================================

#[tokio::test]
async fn test_validate_yaml_syntax_error() {
    let skill = WorkflowCreatorSkill::new();
    let bad_yaml = "---\nid: test\nname: Test\n  bad_indent: [\n---\n";
    let tmp = write_temp_skill_md(bad_yaml);
    let path = tmp.path().to_str().unwrap();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": path
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["valid"], false);
    assert!(
        v.get("error").is_some(),
        "should include error details on YAML syntax error"
    );
}

#[tokio::test]
async fn test_validate_steps_not_sequential() {
    let skill = WorkflowCreatorSkill::new();
    let steps = r#"  - id: 0
    name: Step Zero
    goal: Do A
    verify:
      - "check"
    jump: []
    transitions:
      - action: complete
  - id: 2
    name: Step Two
    goal: Do B
    verify:
      - "check"
    jump: []
    transitions:
      - action: complete"#;
    let tmp = write_temp_skill_md(&bad_skill_yaml(steps));
    let path = tmp.path().to_str().unwrap();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": path
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["valid"], false);
    let error = v["error"].as_str().unwrap();
    assert!(
        error.contains("id") || error.contains("expected"),
        "error should mention step id issue: {error}"
    );
}

#[tokio::test]
async fn test_validate_transitions_no_default() {
    let skill = WorkflowCreatorSkill::new();
    let steps = r#"  - id: 0
    name: Step One
    goal: Do something
    verify:
      - "check"
    jump: []
    transitions:
      - when:
          done: true
        action: complete
      - when:
          done: false
        action: complete"#;
    let tmp = write_temp_skill_md(&bad_skill_yaml(steps));
    let path = tmp.path().to_str().unwrap();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": path
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["valid"], false);
    let error = v["error"].as_str().unwrap();
    assert!(
        error.contains("default") || error.contains("transition"),
        "error should mention default transition issue: {error}"
    );
}

#[tokio::test]
async fn test_validate_enum_options_over_limit() {
    let skill = WorkflowCreatorSkill::new();
    let options: Vec<String> = (0..27).map(|i| format!("opt_{i}")).collect();
    let options_yaml = options
        .iter()
        .map(|o| format!("        - \"{o}\""))
        .collect::<Vec<_>>()
        .join("\n");
    let steps = format!(
        r#"  - id: 0
    name: Step One
    goal: Do something
    verify:
      - "check"
    jump:
      - id: choice
        prompt: Pick one
        type: enum
        options:
{options_yaml}
    transitions:
      - when:
          choice: "opt_0"
        action: complete
      - action: complete"#
    );
    let tmp = write_temp_skill_md(&bad_skill_yaml(&steps));
    let path = tmp.path().to_str().unwrap();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": path
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["valid"], false);
    let error = v["error"].as_str().unwrap();
    assert!(
        error.contains("26") || error.contains("option"),
        "error should mention enum option limit: {error}"
    );
}

#[tokio::test]
async fn test_validate_file_not_found() {
    let skill = WorkflowCreatorSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": "/tmp/__nonexistent_closeclaw_workflow_test__.md"
        })))
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, SkillError::ExecutionFailed(_)),
        "file not found should return ExecutionFailed, got: {err:?}"
    );
}

#[tokio::test]
async fn test_validate_empty_steps_list() {
    let skill = WorkflowCreatorSkill::new();
    let bad_yaml = "---\nid: test\nname: Test\ndescription: Empty\n\
                     version: \"0.1\"\nallow_blocked: false\n\
                     verify_retry_limit: 3\nstep_data_schema: {}\n\
                     steps: []\n---\n";
    let tmp = write_temp_skill_md(bad_yaml);
    let path = tmp.path().to_str().unwrap();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": path
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["valid"], false);
    let error = v["error"].as_str().unwrap();
    assert!(
        error.contains("empty") || error.contains("steps"),
        "error should mention empty steps: {error}"
    );
}

#[tokio::test]
async fn test_validate_step_name_empty() {
    let skill = WorkflowCreatorSkill::new();
    let steps = r#"  - id: 0
    name: ""
    goal: Do something
    verify:
      - "check"
    jump: []
    transitions:
      - action: complete"#;
    let tmp = write_temp_skill_md(&bad_skill_yaml(steps));
    let path = tmp.path().to_str().unwrap();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": path
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["valid"], false);
    let error = v["error"].as_str().unwrap();
    assert!(
        error.contains("name"),
        "error should mention empty name: {error}"
    );
}

#[tokio::test]
async fn test_validate_step_goal_empty() {
    let skill = WorkflowCreatorSkill::new();
    let steps = r#"  - id: 0
    name: Step One
    goal: ""
    verify:
      - "check"
    jump: []
    transitions:
      - action: complete"#;
    let tmp = write_temp_skill_md(&bad_skill_yaml(steps));
    let path = tmp.path().to_str().unwrap();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": path
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["valid"], false);
    let error = v["error"].as_str().unwrap();
    assert!(
        error.contains("goal"),
        "error should mention empty goal: {error}"
    );
}

#[tokio::test]
async fn test_validate_verify_empty() {
    let skill = WorkflowCreatorSkill::new();
    let steps = r#"  - id: 0
    name: Step One
    goal: Do something
    verify: []
    jump: []
    transitions:
      - action: complete"#;
    let tmp = write_temp_skill_md(&bad_skill_yaml(steps));
    let path = tmp.path().to_str().unwrap();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": path
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["valid"], false);
    let error = v["error"].as_str().unwrap();
    assert!(
        error.contains("verify"),
        "error should mention empty verify: {error}"
    );
}

#[tokio::test]
async fn test_validate_transitions_empty() {
    let skill = WorkflowCreatorSkill::new();
    let steps = r#"  - id: 0
    name: Step One
    goal: Do something
    verify:
      - "check"
    jump: []
    transitions: []"#;
    let tmp = write_temp_skill_md(&bad_skill_yaml(steps));
    let path = tmp.path().to_str().unwrap();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": path
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["valid"], false);
    let error = v["error"].as_str().unwrap();
    assert!(
        error.contains("transitions"),
        "error should mention empty transitions: {error}"
    );
}

#[tokio::test]
async fn test_validate_missing_frontmatter_delimiters() {
    let skill = WorkflowCreatorSkill::new();
    let bad_yaml = "id: test\nname: Test\n";
    let tmp = write_temp_skill_md(bad_yaml);
    let path = tmp.path().to_str().unwrap();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": path
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["valid"], false);
    let error = v["error"].as_str().unwrap();
    assert!(
        error.contains("---") || error.contains("frontmatter") || error.contains("delimiter"),
        "error should mention missing frontmatter delimiters: {error}"
    );
}

#[tokio::test]
async fn test_validate_duplicate_jump_ids() {
    let skill = WorkflowCreatorSkill::new();
    let steps = r#"  - id: 0
    name: Step One
    goal: Do something
    verify:
      - "check"
    jump:
      - id: q1
        prompt: Question one
        type: boolean
      - id: q1
        prompt: Question one again
        type: boolean
    transitions:
      - when:
          q1: true
        action: complete
      - action: complete"#;
    let tmp = write_temp_skill_md(&bad_skill_yaml(steps));
    let path = tmp.path().to_str().unwrap();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": path
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["valid"], false);
    let error = v["error"].as_str().unwrap();
    assert!(
        error.contains("jump") || error.contains("duplicate"),
        "error should mention duplicate jump ids: {error}"
    );
}

// ==========================================================================
// execute() — argument error paths
// ==========================================================================

#[tokio::test]
async fn test_execute_validate_missing_path() {
    let skill = WorkflowCreatorSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "validate"
        })))
        .await
        .unwrap_err();
    assert!(
        matches!(err, SkillError::InvalidArgs(_)),
        "validate without path should return InvalidArgs, got: {err:?}"
    );
}

#[tokio::test]
async fn test_execute_unknown_action() {
    let skill = WorkflowCreatorSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "nonexistent"
        })))
        .await
        .unwrap_err();
    assert!(
        matches!(err, SkillError::InvalidArgs(_)),
        "unknown action should return InvalidArgs, got: {err:?}"
    );
}

#[tokio::test]
async fn test_execute_create_missing_name() {
    let skill = WorkflowCreatorSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "create"
        })))
        .await
        .unwrap_err();
    assert!(
        matches!(err, SkillError::InvalidArgs(_)),
        "create without name should return InvalidArgs, got: {err:?}"
    );
}

// ==========================================================================
// execute() — does not delegate to body
// ==========================================================================

#[tokio::test]
async fn test_execute_does_not_return_body_text() {
    let skill = WorkflowCreatorSkill::new();
    let result = skill.execute(None).await.unwrap();
    assert_ne!(result, skill.body());
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"].as_str().unwrap(), "create_workflow");
}
