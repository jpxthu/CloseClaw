//! Tests for file_ops skill — permission engine integration and error paths
use crate::builtin::FileOpsSkill;
use crate::registry::Skill;
use closeclaw_permission::actions::ActionBuilder;
use closeclaw_permission::rules::{RuleBuilder, RuleSetBuilder};
use closeclaw_permission::{Effect, MatchType};
use std::sync::Arc;
use tempfile::TempDir;

fn make_allowed_engine() -> Arc<closeclaw_permission::PermissionEngine> {
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .rule(
            RuleBuilder::new()
                .name("allow-file-read")
                .subject_agent("test-agent")
                .allow()
                .action(
                    ActionBuilder::file("read", vec!["**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("allow-file-write")
                .subject_agent("test-agent")
                .allow()
                .action(
                    ActionBuilder::file("write", vec!["**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("allow-file-exists")
                .subject_agent("test-agent")
                .allow()
                .action(
                    ActionBuilder::file("exists", vec!["**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("allow-file-delete")
                .subject_agent("test-agent")
                .allow()
                .action(
                    ActionBuilder::file("delete", vec!["**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("allow-file-list")
                .subject_agent("test-agent")
                .allow()
                .action(
                    ActionBuilder::file("list", vec!["**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("user-allow-file-read")
                .subject_user_and_agent("*", "test-agent", MatchType::Glob, MatchType::Exact)
                .allow()
                .action(
                    ActionBuilder::file("read", vec!["**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("user-allow-file-write")
                .subject_user_and_agent("*", "test-agent", MatchType::Glob, MatchType::Exact)
                .allow()
                .action(
                    ActionBuilder::file("write", vec!["**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("user-allow-file-exists")
                .subject_user_and_agent("*", "test-agent", MatchType::Glob, MatchType::Exact)
                .allow()
                .action(
                    ActionBuilder::file("exists", vec!["**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("user-allow-file-delete")
                .subject_user_and_agent("*", "test-agent", MatchType::Glob, MatchType::Exact)
                .allow()
                .action(
                    ActionBuilder::file("delete", vec!["**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .rule(
            RuleBuilder::new()
                .name("user-allow-file-list")
                .subject_user_and_agent("*", "test-agent", MatchType::Glob, MatchType::Exact)
                .allow()
                .action(
                    ActionBuilder::file("list", vec!["**".to_string()])
                        .build()
                        .unwrap(),
                )
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    Arc::new(closeclaw_permission::PermissionEngine::new_with_default_data_root(ruleset))
}

fn make_denied_engine() -> Arc<closeclaw_permission::PermissionEngine> {
    let ruleset = RuleSetBuilder::new()
        .default_file_read(Effect::Deny)
        .default_file_write(Effect::Deny)
        .build()
        .unwrap();
    Arc::new(closeclaw_permission::PermissionEngine::new_with_default_data_root(ruleset))
}

#[tokio::test]
async fn test_file_ops_with_engine_constructs_skill() {
    let engine = make_allowed_engine();
    let skill = FileOpsSkill::with_engine(engine);
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.txt");
    std::fs::write(&path, "hello").unwrap();
    let result = skill
        .execute(
            "read",
            serde_json::json!({
                "path": path.to_str().unwrap(),
                "agent_id": "test-agent"
            }),
        )
        .await;
    assert!(
        result.is_ok(),
        "expected Ok with allowed engine, got {:?}",
        result
    );
    assert_eq!(result.unwrap()["content"], "hello");
}

#[tokio::test]
async fn test_file_ops_permission_allowed_read() {
    let engine = make_allowed_engine();
    let skill = FileOpsSkill::with_engine(engine);
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("secret.txt");
    std::fs::write(&path, "secret data").unwrap();
    let result = skill
        .execute(
            "read",
            serde_json::json!({
                "path": path.to_str().unwrap(),
                "agent_id": "test-agent"
            }),
        )
        .await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap()["content"], "secret data");
}

#[tokio::test]
async fn test_file_ops_permission_allowed_write() {
    let engine = make_allowed_engine();
    let skill = FileOpsSkill::with_engine(engine);
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("out.txt");
    let result = skill
        .execute(
            "write",
            serde_json::json!({
                "path": path.to_str().unwrap(),
                "content": "allowed content",
                "agent_id": "test-agent"
            }),
        )
        .await;
    assert!(result.is_ok());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "allowed content");
}

#[tokio::test]
async fn test_file_ops_permission_allowed_exists() {
    let engine = make_allowed_engine();
    let skill = FileOpsSkill::with_engine(engine);
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("check.txt");
    std::fs::write(&path, "").unwrap();
    let result = skill
        .execute(
            "exists",
            serde_json::json!({
                "path": path.to_str().unwrap(),
                "agent_id": "test-agent"
            }),
        )
        .await;
    assert_eq!(result.unwrap()["exists"], true);
}

#[tokio::test]
async fn test_file_ops_permission_allowed_list() {
    let engine = make_allowed_engine();
    let skill = FileOpsSkill::with_engine(engine);
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("a.txt"), "").unwrap();
    let result = skill
        .execute(
            "list",
            serde_json::json!({
                "path": dir.path().to_str().unwrap(),
                "agent_id": "test-agent"
            }),
        )
        .await;
    let entries = result.as_ref().unwrap()["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
}

#[tokio::test]
async fn test_file_ops_permission_allowed_delete() {
    let engine = make_allowed_engine();
    let skill = FileOpsSkill::with_engine(engine);
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("todelete.txt");
    std::fs::write(&path, "to delete").unwrap();
    let result = skill
        .execute(
            "delete",
            serde_json::json!({
                "path": path.to_str().unwrap(),
                "agent_id": "test-agent"
            }),
        )
        .await;
    assert!(result.is_ok());
    assert!(!path.exists());
}

#[tokio::test]
async fn test_file_ops_permission_denied() {
    let engine = make_denied_engine();
    let skill = FileOpsSkill::with_engine(engine);
    let result = skill
        .execute(
            "read",
            serde_json::json!({
                "path": "/nonexistent/file.txt",
                "agent_id": "other-agent"
            }),
        )
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        crate::registry::SkillError::PermissionDenied(reason) => {
            assert!(reason.contains("no matching rule") || !reason.is_empty());
        }
        other => panic!("expected PermissionDenied, got {:?}", other),
    }
}

#[tokio::test]
async fn test_file_ops_permission_missing_agent_id() {
    let engine = make_allowed_engine();
    let skill = FileOpsSkill::with_engine(engine);
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("x.txt");
    let result = skill
        .execute("read", serde_json::json!({"path": path.to_str().unwrap()}))
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        crate::registry::SkillError::InvalidArgs(msg) => {
            assert!(msg.contains("agent_id"))
        }
        other => panic!("expected InvalidArgs, got {:?}", other),
    }
}

#[tokio::test]
async fn test_file_ops_read_nonexistent_file() {
    let skill = FileOpsSkill::new();
    let result = skill
        .execute("read", serde_json::json!({"path": "/nonexistent/file.txt"}))
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        crate::registry::SkillError::ExecutionFailed(_) => {}
        other => panic!("expected ExecutionFailed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_file_ops_delete_nonexistent_file() {
    let skill = FileOpsSkill::new();
    let result = skill
        .execute(
            "delete",
            serde_json::json!({"path": "/nonexistent/file.txt"}),
        )
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        crate::registry::SkillError::ExecutionFailed(_) => {}
        other => panic!("expected ExecutionFailed, got {:?}", other),
    }
}

#[tokio::test]
async fn test_file_ops_list_nonexistent_dir() {
    let skill = FileOpsSkill::new();
    let result = skill
        .execute("list", serde_json::json!({"path": "/nonexistent/dir"}))
        .await;
    let err = result.unwrap_err();
    assert!(matches!(
        err,
        crate::registry::SkillError::ExecutionFailed(_)
    ));
}

#[tokio::test]
async fn test_file_ops_list_default_path() {
    let skill = FileOpsSkill::new();
    let result = skill.execute("list", serde_json::json!({})).await;
    let value = result.unwrap();
    let entries = value["entries"].as_array().unwrap();
    assert!(!entries.is_empty());
}

#[tokio::test]
async fn test_file_ops_write_missing_path() {
    let skill = FileOpsSkill::new();
    let result = skill
        .execute("write", serde_json::json!({"content": "data"}))
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        crate::registry::SkillError::InvalidArgs(msg) => assert!(msg.contains("path")),
        other => panic!("expected InvalidArgs, got {:?}", other),
    }
}

#[tokio::test]
async fn test_file_ops_exists_missing_path() {
    let skill = FileOpsSkill::new();
    let result = skill.execute("exists", serde_json::json!({})).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        crate::registry::SkillError::InvalidArgs(msg) => assert!(msg.contains("path")),
        other => panic!("expected InvalidArgs, got {:?}", other),
    }
}
