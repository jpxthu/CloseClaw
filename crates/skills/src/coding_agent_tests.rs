//! Tests for coding_agent skill execute()
use crate::registry::Skill;
use crate::CodingAgentSkill;

#[tokio::test]
async fn test_coding_agent_body_not_empty() {
    let skill = CodingAgentSkill::new();
    let body = skill.body();
    assert!(!body.is_empty());
    assert!(body.contains("Coding Agent Skill"));
}

#[tokio::test]
async fn test_coding_agent_manifest() {
    let skill = CodingAgentSkill::new();
    let m = skill.manifest();
    assert_eq!(m.name, "coding_agent");
    assert_eq!(m.version, "1.0.0");
    assert!(m.description.contains("AI coding agents"));
    assert!(m.dependencies.is_empty());
}

#[tokio::test]
async fn test_coding_agent_listing_meta() {
    let skill = CodingAgentSkill::new();
    let meta = skill.listing_meta();
    assert!(meta.user_invocable);
    assert!(!meta.when_to_use.is_empty());
}

// --- execute() tests ---

#[tokio::test]
async fn test_execute_none_returns_capabilities() {
    let skill = CodingAgentSkill::new();
    let result = skill.execute(None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "coding_agent");
    let actions = v["supported_actions"].as_array().unwrap();
    assert!(actions.contains(&serde_json::json!("delegate")));
}

#[tokio::test]
async fn test_execute_empty_args_returns_capabilities() {
    let skill = CodingAgentSkill::new();
    let result = skill.execute(Some(serde_json::json!({}))).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "coding_agent");
}

#[tokio::test]
async fn test_execute_no_task_returns_capabilities() {
    let skill = CodingAgentSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"action": "delegate"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "coding_agent");
    assert!(v["supported_actions"].is_array());
}

#[tokio::test]
async fn test_execute_with_task() {
    let skill = CodingAgentSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"task": "refactor auth module"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "coding_agent");
    assert_eq!(v["action"], "delegate");
    assert_eq!(v["task"], "refactor auth module");
    assert!(v["guidance"].is_string());
    let agents = v["agents"].as_array().unwrap();
    assert!(agents.contains(&serde_json::json!("opencode")));
    assert!(agents.contains(&serde_json::json!("claude-code")));
}

#[tokio::test]
async fn test_execute_with_empty_task() {
    let skill = CodingAgentSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"task": ""})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    // Empty task string is still Some(""), should return delegation params
    assert_eq!(v["action"], "delegate");
    assert_eq!(v["task"], "");
}

#[tokio::test]
async fn test_execute_does_not_delegate_to_body() {
    let skill = CodingAgentSkill::new();
    let result = skill.execute(None).await.unwrap();
    assert_ne!(result, skill.body());
    // Should be valid JSON
    let _: serde_json::Value = serde_json::from_str(&result).unwrap();
}

#[tokio::test]
async fn test_execute_with_extra_args_no_task() {
    let skill = CodingAgentSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"foo": "bar"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    // No task field → capabilities description
    assert_eq!(v["skill"], "coding_agent");
    assert!(v["supported_actions"].is_array());
}
