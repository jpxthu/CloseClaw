//! Tests for skill_discovery skill execute()
//!
//! Convention (see `discovery_test_helpers` module docs): any test that
//! creates and actually spawns a mock clawhub script must be marked
//! `#[serial_test::serial]` (ETXTBSY root cause); mock scripts must live
//! in a `tempfile::TempDir`.
use super::discovery_test_helpers::{
    create_failing_mock_clawhub, create_mock_clawhub, MOCK_FAILURE_MESSAGE, MOCK_LIST_OUTPUT,
    MOCK_SEARCH_OUTPUT,
};
use crate::builtin::SkillDiscoverySkill;
use crate::registry::{Skill, SkillError};
use serial_test::serial;
use tempfile::TempDir;

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
    assert!(!m.description.is_empty());
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

#[serial]
#[tokio::test]
async fn test_execute_find_returns_structured_result() {
    let tmp = TempDir::new().unwrap();
    let mock = create_mock_clawhub(&tmp);
    let skill = SkillDiscoverySkill::with_cli_command(mock);
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "find",
            "query": "alpha"
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "find");
    assert_eq!(v["query"], "alpha");
    assert_eq!(v["results"], MOCK_SEARCH_OUTPUT);
}

#[serial]
#[tokio::test]
async fn test_execute_list_returns_structured_result() {
    let tmp = TempDir::new().unwrap();
    let mock = create_mock_clawhub(&tmp);
    let skill = SkillDiscoverySkill::with_cli_command(mock);
    let result = skill
        .execute(Some(serde_json::json!({"action": "list"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "list");
    assert_eq!(v["installed"], MOCK_LIST_OUTPUT);
}

#[serial]
#[tokio::test]
async fn test_execute_find_propagates_clawhub_failure() {
    let tmp = TempDir::new().unwrap();
    let mock = create_failing_mock_clawhub(&tmp);
    let skill = SkillDiscoverySkill::with_cli_command(mock);
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "find",
            "query": "alpha"
        })))
        .await
        .unwrap_err();
    let SkillError::ExecutionFailed(msg) = err else {
        panic!("clawhub failure should return ExecutionFailed, got: {err:?}");
    };
    assert!(
        msg.contains(MOCK_FAILURE_MESSAGE),
        "stderr not propagated, got: {msg}"
    );
}

#[serial]
#[tokio::test]
async fn test_execute_install_propagates_clawhub_failure() {
    let tmp = TempDir::new().unwrap();
    let mock = create_failing_mock_clawhub(&tmp);
    let skill = SkillDiscoverySkill::with_cli_command(mock);
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "install",
            "name": "skill_under_test_abc123"
        })))
        .await
        .unwrap_err();
    let SkillError::ExecutionFailed(msg) = err else {
        panic!("clawhub install failure should return ExecutionFailed, got: {err:?}");
    };
    assert!(
        msg.contains(MOCK_FAILURE_MESSAGE),
        "expected stderr containing {MOCK_FAILURE_MESSAGE:?}, got: {msg}"
    );
}

#[serial]
#[tokio::test]
async fn test_execute_update_propagates_clawhub_failure() {
    let tmp = TempDir::new().unwrap();
    let mock = create_failing_mock_clawhub(&tmp);
    let skill = SkillDiscoverySkill::with_cli_command(mock);
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "update",
            "name": "skill_under_test_abc123"
        })))
        .await
        .unwrap_err();
    let SkillError::ExecutionFailed(msg) = err else {
        panic!("clawhub update failure should return ExecutionFailed, got: {err:?}");
    };
    assert!(
        msg.contains(MOCK_FAILURE_MESSAGE),
        "expected stderr containing {MOCK_FAILURE_MESSAGE:?}, got: {msg}"
    );
}

#[tokio::test]
async fn test_execute_does_not_delegate_to_body() {
    let skill = SkillDiscoverySkill::new();
    let result = skill.execute(None).await.unwrap();
    assert_ne!(result, skill.body());
    // Should be valid JSON
    let _: serde_json::Value = serde_json::from_str(&result).unwrap();
}
