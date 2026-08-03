//! Tests for skill_creator skill
use crate::registry::{Skill, SkillError};
use crate::SkillCreatorSkill;
use serde_json::json;

#[test]
fn test_manifest() {
    let skill = SkillCreatorSkill::new();
    let m = skill.manifest();
    assert_eq!(m.name, "skill_creator");
    assert_eq!(m.version, "1.0.0");
    assert!(!m.description.is_empty());
}

#[test]
fn test_body_not_empty() {
    let skill = SkillCreatorSkill::new();
    let body = skill.body();
    assert!(!body.is_empty());
    assert!(body.contains("Skill Creator"));
    assert!(body.contains("SKILL.md Template"));
}

#[test]
fn test_body_contains_frontmatter_guide() {
    let skill = SkillCreatorSkill::new();
    let body = skill.body();
    assert!(body.contains("description"));
    assert!(body.contains("Frontmatter Fields"));
}

#[test]
fn test_default() {
    let skill = SkillCreatorSkill::default();
    assert_eq!(skill.manifest().name, "skill_creator");
}

// --- execute() tests ---

#[tokio::test]
async fn test_execute_none_returns_capabilities() {
    let skill = SkillCreatorSkill::new();
    let result = skill.execute(None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "skill_creator");
    let actions = v["supported_actions"].as_array().unwrap();
    assert!(actions.contains(&json!("create")));
    assert!(actions.contains(&json!("validate")));
    assert!(actions.contains(&json!("edit")));
}

#[tokio::test]
async fn test_execute_empty_args_returns_capabilities() {
    let skill = SkillCreatorSkill::new();
    let result = skill.execute(Some(json!({}))).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "skill_creator");
}

#[tokio::test]
async fn test_execute_no_action_returns_capabilities() {
    let skill = SkillCreatorSkill::new();
    let result = skill.execute(Some(json!({"name": "test"}))).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "skill_creator");
    assert!(v["supported_actions"].is_array());
}

#[tokio::test]
async fn test_execute_create_with_name_and_description() {
    let skill = SkillCreatorSkill::new();
    let result = skill
        .execute(Some(json!({
            "action": "create",
            "name": "my_skill",
            "description": "Does cool things"
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "skill_creator");
    assert_eq!(v["action"], "create");
    assert_eq!(v["target"]["name"], "my_skill");
    assert_eq!(v["target"]["description"], "Does cool things");
    assert!(v["template"]["frontmatter"].is_object());
    assert!(v["instructions"].is_string());
}

#[tokio::test]
async fn test_execute_create_without_description() {
    let skill = SkillCreatorSkill::new();
    let result = skill
        .execute(Some(json!({"action": "create", "name": "foo"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "create");
    assert_eq!(v["target"]["description"], "New skill");
}

#[tokio::test]
async fn test_execute_create_missing_name() {
    let skill = SkillCreatorSkill::new();
    let err = skill
        .execute(Some(json!({"action": "create"})))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => {
            assert!(msg.contains("name"));
        }
        other => panic!("expected InvalidArgs, got {:?}", other),
    }
}

#[tokio::test]
async fn test_execute_validate() {
    let skill = SkillCreatorSkill::new();
    let result = skill
        .execute(Some(json!({
            "action": "validate",
            "path": "skills/test/SKILL.md"
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "skill_creator");
    assert_eq!(v["action"], "validate");
    assert_eq!(v["target"]["path"], "skills/test/SKILL.md");
    let checks = v["checks"].as_array().unwrap();
    assert!(!checks.is_empty());
}

#[tokio::test]
async fn test_execute_validate_missing_path() {
    let skill = SkillCreatorSkill::new();
    let err = skill
        .execute(Some(json!({"action": "validate"})))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => {
            assert!(msg.contains("path"));
        }
        other => panic!("expected InvalidArgs, got {:?}", other),
    }
}

#[tokio::test]
async fn test_execute_edit() {
    let skill = SkillCreatorSkill::new();
    let result = skill
        .execute(Some(json!({
            "action": "edit",
            "path": "skills/test/SKILL.md",
            "field": "description",
            "value": "Updated desc"
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "skill_creator");
    assert_eq!(v["action"], "edit");
    assert_eq!(v["target"]["path"], "skills/test/SKILL.md");
    assert_eq!(v["change"]["field"], "description");
    assert_eq!(v["change"]["value"], "Updated desc");
}

#[tokio::test]
async fn test_execute_edit_missing_field() {
    let skill = SkillCreatorSkill::new();
    let err = skill
        .execute(Some(json!({"action": "edit", "path": "x"})))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => {
            assert!(msg.contains("field"));
        }
        other => panic!("expected InvalidArgs, got {:?}", other),
    }
}

#[tokio::test]
async fn test_execute_edit_missing_path() {
    let skill = SkillCreatorSkill::new();
    let err = skill
        .execute(Some(json!({
            "action": "edit",
            "field": "description",
            "value": "x"
        })))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => {
            assert!(msg.contains("path"));
        }
        other => panic!("expected InvalidArgs, got {:?}", other),
    }
}

#[tokio::test]
async fn test_execute_unknown_action() {
    let skill = SkillCreatorSkill::new();
    let err = skill
        .execute(Some(json!({"action": "bogus"})))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => {
            assert!(msg.contains("unknown action"));
        }
        other => panic!("expected InvalidArgs, got {:?}", other),
    }
}

#[tokio::test]
async fn test_execute_does_not_delegate_to_body() {
    let skill = SkillCreatorSkill::new();
    let result = skill.execute(None).await.unwrap();
    assert_ne!(result, skill.body());
    // Should be valid JSON
    let _: serde_json::Value = serde_json::from_str(&result).unwrap();
}
