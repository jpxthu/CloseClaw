//! Tests for git_ops skill execute()
use crate::builtin::GitOpsSkill;
use crate::registry::{Skill, SkillError};
use std::path::Path;

/// Create a temporary git repo for testing.
async fn setup_temp_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::process::Command::new("git")
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args([
            "-C",
            dir.path().to_str().unwrap(),
            "config",
            "user.email",
            "test@test.com",
        ])
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args([
            "-C",
            dir.path().to_str().unwrap(),
            "config",
            "user.name",
            "Test",
        ])
        .output()
        .unwrap();
    dir
}

/// Create a commit in the temp repo.
fn create_commit(dir: &Path, filename: &str, content: &str) {
    std::fs::write(dir.join(filename), content).unwrap();
    std::process::Command::new("git")
        .args(["-C", dir.to_str().unwrap(), "add", "."])
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args([
            "-C",
            dir.to_str().unwrap(),
            "commit",
            "-m",
            "test commit",
            "--allow-empty",
        ])
        .output()
        .unwrap();
}

#[tokio::test]
async fn test_git_ops_body_not_empty() {
    let skill = GitOpsSkill::new();
    let body = skill.body();
    assert!(!body.is_empty());
    assert!(body.contains("Git Operations Skill"));
}

#[tokio::test]
async fn test_git_ops_manifest() {
    let skill = GitOpsSkill::new();
    let m = skill.manifest();
    assert_eq!(m.name, "git_ops");
    assert_eq!(m.version, "1.0.0");
    assert!(!m.description.is_empty());
}

// --- execute() tests ---

#[tokio::test]
async fn test_execute_none_returns_capabilities() {
    let skill = GitOpsSkill::new();
    let result = skill.execute(None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "git_ops");
    let actions = v["supported_actions"].as_array().unwrap();
    assert!(actions.contains(&serde_json::json!("status")));
    assert!(actions.contains(&serde_json::json!("log")));
    assert!(actions.contains(&serde_json::json!("diff")));
}

#[tokio::test]
async fn test_execute_empty_args_returns_capabilities() {
    let skill = GitOpsSkill::new();
    let result = skill.execute(Some(serde_json::json!({}))).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "git_ops");
}

#[tokio::test]
async fn test_execute_unknown_action() {
    let skill = GitOpsSkill::new();
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
async fn test_execute_status_clean_repo() {
    let dir = setup_temp_repo().await;
    create_commit(dir.path(), "file.txt", "hello");
    let skill = GitOpsSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "status",
            "path": dir.path()
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "status");
    assert_eq!(v["has_changes"], false);
}

#[tokio::test]
async fn test_execute_status_dirty_repo() {
    let dir = setup_temp_repo().await;
    create_commit(dir.path(), "file.txt", "hello");
    std::fs::write(dir.path().join("new_file.txt"), "world").unwrap();
    let skill = GitOpsSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "status",
            "path": dir.path()
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "status");
    assert_eq!(v["has_changes"], true);
}

#[tokio::test]
async fn test_execute_log_empty_repo() {
    let dir = setup_temp_repo().await;
    let skill = GitOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "log",
            "path": dir.path()
        })))
        .await
        .unwrap_err();
    assert!(matches!(err, SkillError::ExecutionFailed(_)));
}

#[tokio::test]
async fn test_execute_log_with_commits() {
    let dir = setup_temp_repo().await;
    create_commit(dir.path(), "a.txt", "a");
    create_commit(dir.path(), "b.txt", "b");
    let skill = GitOpsSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "log",
            "path": dir.path(),
            "max_count": 5
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "log");
    assert!(v["count"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn test_execute_diff_clean_repo() {
    let dir = setup_temp_repo().await;
    create_commit(dir.path(), "file.txt", "hello");
    let skill = GitOpsSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "diff",
            "path": dir.path()
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "diff");
    assert_eq!(v["has_diff"], false);
    assert_eq!(v["staged"], false);
}

#[tokio::test]
async fn test_execute_diff_with_changes() {
    let dir = setup_temp_repo().await;
    create_commit(dir.path(), "file.txt", "hello");
    std::fs::write(dir.path().join("file.txt"), "modified").unwrap();
    let skill = GitOpsSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "diff",
            "path": dir.path()
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "diff");
    assert_eq!(v["has_diff"], true);
}

#[tokio::test]
async fn test_execute_diff_staged() {
    let dir = setup_temp_repo().await;
    create_commit(dir.path(), "file.txt", "hello");
    std::fs::write(dir.path().join("file.txt"), "modified").unwrap();
    std::process::Command::new("git")
        .args(["-C", dir.path().to_str().unwrap(), "add", "."])
        .output()
        .unwrap();
    let skill = GitOpsSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "diff",
            "path": dir.path(),
            "staged": true
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "diff");
    assert_eq!(v["has_diff"], true);
    assert_eq!(v["staged"], true);
}

#[tokio::test]
async fn test_execute_invalid_git_repo() {
    let dir = tempfile::tempdir().unwrap();
    let skill = GitOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({
            "action": "status",
            "path": dir.path()
        })))
        .await
        .unwrap_err();
    assert!(matches!(err, SkillError::ExecutionFailed(_)));
}

#[tokio::test]
async fn test_execute_does_not_delegate_to_body() {
    let skill = GitOpsSkill::new();
    let result = skill.execute(None).await.unwrap();
    assert_ne!(result, skill.body());
    let _: serde_json::Value = serde_json::from_str(&result).unwrap();
}
