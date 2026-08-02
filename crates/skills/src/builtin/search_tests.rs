//! Tests for search skill execute()
use crate::builtin::SearchSkill;
use crate::registry::Skill;

#[tokio::test]
async fn test_search_body_not_empty() {
    let skill = SearchSkill::new();
    let body = skill.body();
    assert!(!body.is_empty());
    assert!(body.contains("Search Skill"));
}

#[tokio::test]
async fn test_search_manifest() {
    let skill = SearchSkill::new();
    let m = skill.manifest();
    assert_eq!(m.name, "search");
    assert_eq!(m.version, "1.0.0");
    assert!(!m.description.is_empty());
}

#[tokio::test]
async fn test_search_listing_meta() {
    let skill = SearchSkill::new();
    let meta = skill.listing_meta();
    assert!(meta.user_invocable);
    assert!(!meta.when_to_use.is_empty());
}

// --- execute() tests ---

#[tokio::test]
async fn test_execute_none_returns_capabilities() {
    let skill = SearchSkill::new();
    let result = skill.execute(None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "search");
    assert!(v["supported_tools"].is_array());
    let tools = v["supported_tools"].as_array().unwrap();
    assert!(tools.contains(&serde_json::json!("web_search")));
    assert!(tools.contains(&serde_json::json!("web_fetch")));
}

#[tokio::test]
async fn test_execute_empty_args_returns_capabilities() {
    let skill = SearchSkill::new();
    let result = skill.execute(Some(serde_json::json!({}))).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "search");
}

#[tokio::test]
async fn test_execute_with_query() {
    let skill = SearchSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"query": "rust async"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "search");
    assert_eq!(v["action"], "search");
    assert_eq!(v["query"], "rust async");
    assert!(v["guidance"].is_string());
    assert!(v["tools"].is_array());
}

#[tokio::test]
async fn test_execute_with_empty_query() {
    let skill = SearchSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"query": ""})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    // Empty query string is still Some(""), should return guidance
    assert_eq!(v["action"], "search");
    assert_eq!(v["query"], "");
}

#[tokio::test]
async fn test_execute_with_extra_args_no_query() {
    let skill = SearchSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"foo": "bar"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    // No query field → capabilities description
    assert_eq!(v["skill"], "search");
    assert!(v["supported_tools"].is_array());
}

#[tokio::test]
async fn test_execute_does_not_delegate_to_body() {
    let skill = SearchSkill::new();
    let result = skill.execute(None).await.unwrap();
    assert_ne!(result, skill.body());
    // Should be valid JSON
    let _: serde_json::Value = serde_json::from_str(&result).unwrap();
}
