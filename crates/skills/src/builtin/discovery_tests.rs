//! Tests for skill_discovery skill execute()
use crate::builtin::SkillDiscoverySkill;
use crate::registry::{Skill, SkillError};

#[tokio::test]
async fn test_discovery_body_not_empty() {
    let skill = SkillDiscoverySkill::new();
    let body = skill.body();
    assert!(!body.is_empty());
    assert!(body.contains("Skill Discovery Skill"));
}

#[tokio::test]
async fn test_discovery_manifest() {
    let skill = SkillDiscoverySkill::new();
    let m = skill.manifest();
    assert_eq!(m.name, "skill_discovery");
    assert_eq!(m.version, "1.0.0");
    assert!(m.dependencies.contains(&"clawhub".to_string()));
}

#[tokio::test]
async fn test_discovery_listing_meta() {
    let skill = SkillDiscoverySkill::new();
    let meta = skill.listing_meta();
    assert!(meta.user_invocable);
    assert!(!meta.when_to_use.is_empty());
}

// --- execute() tests ---

#[tokio::test]
async fn test_execute_none_returns_capabilities() {
    let skill = SkillDiscoverySkill::new();
    let result = skill.execute(None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "skill_discovery");
    let actions = v["supported_actions"].as_array().unwrap();
    assert!(actions.contains(&serde_json::json!("find")));
    assert!(actions.contains(&serde_json::json!("install")));
    assert!(actions.contains(&serde_json::json!("list")));
    assert!(actions.contains(&serde_json::json!("update")));
}

#[tokio::test]
async fn test_execute_empty_args_returns_capabilities() {
    let skill = SkillDiscoverySkill::new();
    let result = skill.execute(Some(serde_json::json!({}))).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "skill_discovery");
}

#[tokio::test]
async fn test_execute_no_action_returns_capabilities() {
    let skill = SkillDiscoverySkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"query": "test"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "skill_discovery");
    assert!(v["supported_actions"].is_array());
}

#[tokio::test]
async fn test_execute_find_missing_query() {
    let skill = SkillDiscoverySkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "find"})))
        .await
        .unwrap_err();
    assert!(matches!(err, SkillError::InvalidArgs(_)));
}

#[tokio::test]
async fn test_execute_install_missing_name() {
    let skill = SkillDiscoverySkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "install"})))
        .await
        .unwrap_err();
    assert!(matches!(err, SkillError::InvalidArgs(_)));
}

#[tokio::test]
async fn test_execute_unknown_action() {
    let skill = SkillDiscoverySkill::new();
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "unknown_action",
            "query": "test"
        })))
        .await
        .unwrap_err();
    assert!(matches!(err, SkillError::InvalidArgs(_)));
}

#[tokio::test]
async fn test_execute_find_returns_structured_result() {
    let skill = SkillDiscoverySkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "find",
            "query": "nonexistent_skill_abc123"
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "find");
    assert_eq!(v["query"], "nonexistent_skill_abc123");
    assert!(v["results"].is_string());
}

#[tokio::test]
async fn test_execute_list_returns_structured_result() {
    let skill = SkillDiscoverySkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"action": "list"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "list");
    assert!(v["installed"].is_string());
}

#[tokio::test]
async fn test_execute_install_nonexistent_skill() {
    let skill = SkillDiscoverySkill::new();
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "install",
            "name": "nonexistent_skill_abc123"
        })))
        .await
        .unwrap_err();
    // Skill not found returns ExecutionFailed from clawhub
    assert!(matches!(err, SkillError::ExecutionFailed(_)));
}

#[tokio::test]
async fn test_execute_update_nonexistent_skill() {
    let skill = SkillDiscoverySkill::new();
    // update with a specific non-existent skill name should fail
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "update",
            "name": "nonexistent_skill_abc123"
        })))
        .await
        .unwrap_err();
    assert!(matches!(err, SkillError::ExecutionFailed(_)));
}

#[tokio::test]
async fn test_execute_does_not_delegate_to_body() {
    let skill = SkillDiscoverySkill::new();
    let result = skill.execute(None).await.unwrap();
    assert_ne!(result, skill.body());
    // Should be valid JSON
    let _: serde_json::Value = serde_json::from_str(&result).unwrap();
}
