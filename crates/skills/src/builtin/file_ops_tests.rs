//! Tests for file_ops skill — permission engine integration and error paths
use crate::builtin::FileOpsSkill;
use crate::registry::Skill;
use closeclaw_permission::engine::engine_types::{
    Action, Defaults, Effect, MatchType, Rule, RuleSet, Subject,
};
use closeclaw_permission::skill_wrapper::SkillPermissionEngineWrapper;
use closeclaw_permission::PermissionEngine;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

fn make_allowed_engine_rwlock() -> Arc<tokio::sync::RwLock<PermissionEngine>> {
    let rules = vec![
        Rule {
            name: "allow-file-read".to_string(),
            subject: Subject::AgentOnly {
                agent: "test-agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![Action::File {
                operation: "read".to_string(),
                paths: vec!["**".to_string()],
            }],
            template: None,
            priority: 0,
        },
        Rule {
            name: "allow-file-write".to_string(),
            subject: Subject::AgentOnly {
                agent: "test-agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![Action::File {
                operation: "write".to_string(),
                paths: vec!["**".to_string()],
            }],
            template: None,
            priority: 0,
        },
        Rule {
            name: "user-allow-file-read".to_string(),
            subject: Subject::UserAndAgent {
                user_id: "*".to_string(),
                agent: "test-agent".to_string(),
                user_match: MatchType::Glob,
                agent_match: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![Action::File {
                operation: "read".to_string(),
                paths: vec!["**".to_string()],
            }],
            template: None,
            priority: 0,
        },
        Rule {
            name: "user-allow-file-write".to_string(),
            subject: Subject::UserAndAgent {
                user_id: "*".to_string(),
                agent: "test-agent".to_string(),
                user_match: MatchType::Glob,
                agent_match: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![Action::File {
                operation: "write".to_string(),
                paths: vec!["**".to_string()],
            }],
            template: None,
            priority: 0,
        },
    ];
    let ruleset = RuleSet {
        rules,
        defaults: Defaults::default(),
        template_includes: vec![],
        agent_creators: HashMap::new(),
        ..Default::default()
    };
    Arc::new(tokio::sync::RwLock::new(
        PermissionEngine::new_with_default_data_root(ruleset),
    ))
}

fn make_denied_engine_rwlock() -> Arc<tokio::sync::RwLock<PermissionEngine>> {
    let ruleset = RuleSet {
        rules: vec![],
        defaults: Defaults::default(),
        template_includes: vec![],
        agent_creators: HashMap::new(),
        ..Default::default()
    };
    Arc::new(tokio::sync::RwLock::new(
        PermissionEngine::new_with_default_data_root(ruleset),
    ))
}

fn wrap_engine(
    engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
) -> Arc<SkillPermissionEngineWrapper> {
    Arc::new(SkillPermissionEngineWrapper::new(engine))
}

#[tokio::test]
async fn test_file_ops_with_engine_constructs_skill() {
    let engine = make_allowed_engine_rwlock();
    let wrapper = wrap_engine(engine);
    let skill = FileOpsSkill::with_engine(wrapper);
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
    let engine = make_allowed_engine_rwlock();
    let wrapper = wrap_engine(engine);
    let skill = FileOpsSkill::with_engine(wrapper);
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
    let engine = make_allowed_engine_rwlock();
    let wrapper = wrap_engine(engine);
    let skill = FileOpsSkill::with_engine(wrapper);
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
    let engine = make_allowed_engine_rwlock();
    let wrapper = wrap_engine(engine);
    let skill = FileOpsSkill::with_engine(wrapper);
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
async fn test_file_ops_permission_denied() {
    let engine = make_denied_engine_rwlock();
    let wrapper = wrap_engine(engine);
    let skill = FileOpsSkill::with_engine(wrapper);
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
            assert!(!reason.is_empty());
        }
        other => panic!("expected PermissionDenied, got {:?}", other),
    }
}

#[tokio::test]
async fn test_file_ops_permission_missing_agent_id() {
    let engine = make_allowed_engine_rwlock();
    let wrapper = wrap_engine(engine);
    let skill = FileOpsSkill::with_engine(wrapper);
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
