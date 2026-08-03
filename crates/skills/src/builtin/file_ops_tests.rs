//! Tests for file_ops skill execute()
use crate::builtin::FileOpsSkill;
use crate::registry::{Skill, SkillError};

#[test]
fn test_file_ops_default() {
    let skill = FileOpsSkill::default();
    let m = skill.manifest();
    assert_eq!(m.name, "file_ops");
}

#[tokio::test]
async fn test_file_ops_body_not_empty() {
    let skill = FileOpsSkill::new();
    let body = skill.body();
    assert!(!body.is_empty());
    assert!(body.contains("File Operations Skill"));
}

#[tokio::test]
async fn test_file_ops_manifest() {
    let skill = FileOpsSkill::new();
    let m = skill.manifest();
    assert_eq!(m.name, "file_ops");
    assert_eq!(m.version, "1.0.0");
    assert!(!m.description.is_empty());
}

// --- execute() tests ---

#[tokio::test]
async fn test_execute_none_returns_capabilities() {
    let skill = FileOpsSkill::new();
    let result = skill.execute(None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "file_ops");
    let actions = v["supported_actions"].as_array().unwrap();
    assert!(actions.contains(&serde_json::json!("read")));
    assert!(actions.contains(&serde_json::json!("list")));
    assert!(actions.contains(&serde_json::json!("stat")));
}

#[tokio::test]
async fn test_execute_empty_args_returns_capabilities() {
    let skill = FileOpsSkill::new();
    let result = skill.execute(Some(serde_json::json!({}))).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "file_ops");
}

#[tokio::test]
async fn test_execute_read_missing_path() {
    let skill = FileOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "read"})))
        .await
        .unwrap_err();
    assert!(matches!(err, SkillError::InvalidArgs(_)));
}

#[tokio::test]
async fn test_execute_list_missing_path() {
    let skill = FileOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "list"})))
        .await
        .unwrap_err();
    assert!(matches!(err, SkillError::InvalidArgs(_)));
}

#[tokio::test]
async fn test_execute_stat_missing_path() {
    let skill = FileOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "stat"})))
        .await
        .unwrap_err();
    assert!(matches!(err, SkillError::InvalidArgs(_)));
}

#[tokio::test]
async fn test_execute_unknown_action() {
    let skill = FileOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "unknown_action",
            "path": "/tmp"
        })))
        .await
        .unwrap_err();
    assert!(matches!(err, SkillError::InvalidArgs(_)));
}

#[tokio::test]
async fn test_execute_read_nonexistent_file() {
    let skill = FileOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "read",
            "path": "/tmp/__nonexistent_closeclaw_test__"
        })))
        .await
        .unwrap_err();
    assert!(matches!(err, SkillError::ExecutionFailed(_)));
}

#[tokio::test]
async fn test_execute_read_directory() {
    let skill = FileOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "read",
            "path": "/tmp"
        })))
        .await
        .unwrap_err();
    assert!(matches!(err, SkillError::InvalidArgs(_)));
}

#[tokio::test]
async fn test_execute_read_existing_file() {
    let skill = FileOpsSkill::new();
    // /etc/hostname or /etc/os-release should exist on Linux
    let path = if std::path::Path::new("/etc/os-release").exists() {
        "/etc/os-release"
    } else {
        "/etc/hostname"
    };
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "read",
            "path": path
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "read");
    assert_eq!(v["path"], path);
    assert!(v["content"].is_string());
    assert!(v["size_bytes"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn test_execute_list_nonexistent_dir() {
    let skill = FileOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "list",
            "path": "/tmp/__nonexistent_closeclaw_test__"
        })))
        .await
        .unwrap_err();
    assert!(matches!(err, SkillError::ExecutionFailed(_)));
}

#[tokio::test]
async fn test_execute_list_file_not_dir() {
    let skill = FileOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "list",
            "path": "/etc/os-release"
        })))
        .await
        .unwrap_err();
    assert!(matches!(err, SkillError::InvalidArgs(_)));
}

#[tokio::test]
async fn test_execute_list_existing_dir() {
    let skill = FileOpsSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "list",
            "path": "/tmp"
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "list");
    assert_eq!(v["path"], "/tmp");
    assert!(v["entries"].is_array());
}

#[tokio::test]
async fn test_execute_stat_nonexistent_path() {
    let skill = FileOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "stat",
            "path": "/tmp/__nonexistent_closeclaw_test__"
        })))
        .await
        .unwrap_err();
    assert!(matches!(err, SkillError::ExecutionFailed(_)));
}

#[tokio::test]
async fn test_execute_stat_existing_file() {
    let skill = FileOpsSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "stat",
            "path": "/etc/os-release"
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "stat");
    assert_eq!(v["is_file"], true);
    assert_eq!(v["is_dir"], false);
}

#[tokio::test]
async fn test_execute_stat_existing_dir() {
    let skill = FileOpsSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "stat",
            "path": "/tmp"
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "stat");
    assert_eq!(v["is_dir"], true);
    assert_eq!(v["is_file"], false);
}

#[tokio::test]
async fn test_execute_does_not_delegate_to_body() {
    let skill = FileOpsSkill::new();
    let result = skill.execute(None).await.unwrap();
    assert_ne!(result, skill.body());
    // Should be valid JSON
    let _: serde_json::Value = serde_json::from_str(&result).unwrap();
}
