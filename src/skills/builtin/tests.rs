//! Tests for built-in skills
use crate::permission::{Action, Effect, MatchType, Rule, Subject};
use crate::skills::builtin::{
    builtin_skills, BuiltinSkills, FileOpsSkill, GitOpsSkill, SearchSkill,
};
use crate::skills::Skill;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn test_file_ops_read_requires_agent_id_when_engine_set() {
    // Without engine, agent_id is not required (backward compatible)
    let skill = FileOpsSkill::new();
    let result = skill
        .execute("read", serde_json::json!({"path": "Cargo.toml"}))
        .await;
    assert!(result.is_ok());

    // With engine, agent_id IS required
    let ruleset = crate::permission::RuleSet {
        version: "1.0".to_string(),
        rules: vec![],
        defaults: crate::permission::Defaults::default(),
        template_includes: vec![],
        agent_creators: std::collections::HashMap::new(),
    };
    let engine = Arc::new(crate::permission::PermissionEngine::new(ruleset));
    let skill_with = FileOpsSkill::with_engine(engine);
    let result = skill_with
        .execute("read", serde_json::json!({"path": "Cargo.toml"}))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_file_ops_read_with_permission() {
    let rule = Rule {
        name: "allow-read".to_string(),
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
    };
    let ruleset = crate::permission::RuleSet {
        version: "1.0".to_string(),
        rules: vec![rule],
        defaults: crate::permission::Defaults::default(),
        template_includes: vec![],
        agent_creators: std::collections::HashMap::new(),
    };
    let engine = Arc::new(crate::permission::PermissionEngine::new(ruleset));
    let skill = FileOpsSkill::with_engine(engine);
    let result = skill
        .execute(
            "read",
            serde_json::json!({"path": "Cargo.toml", "agent_id": "test-agent"}),
        )
        .await;
    assert!(
        result.is_ok(),
        "read should succeed with permission: {:?}",
        result
    );
    let value = result.unwrap();
    assert!(value.get("content").is_some());
}

#[tokio::test]
async fn test_file_ops_read_denied_without_permission() {
    let rule = Rule {
        name: "deny-all".to_string(),
        subject: Subject::AgentOnly {
            agent: "test-agent".to_string(),
            match_type: MatchType::Exact,
        },
        effect: Effect::Deny,
        actions: vec![Action::All],
        template: None,
        priority: 0,
    };
    let ruleset = crate::permission::RuleSet {
        version: "1.0".to_string(),
        rules: vec![rule],
        defaults: crate::permission::Defaults::default(),
        template_includes: vec![],
        agent_creators: std::collections::HashMap::new(),
    };
    let engine = Arc::new(crate::permission::PermissionEngine::new(ruleset));
    let skill = FileOpsSkill::with_engine(engine);
    let result = skill
        .execute(
            "read",
            serde_json::json!({"path": "Cargo.toml", "agent_id": "test-agent"}),
        )
        .await;
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        crate::skills::SkillError::PermissionDenied(_)
    ));
}

#[tokio::test]
async fn test_file_ops_exists_requires_agent_id_when_engine_set() {
    let skill = FileOpsSkill::new();
    let result = skill
        .execute("exists", serde_json::json!({"path": "Cargo.toml"}))
        .await;
    assert!(result.is_ok());

    let ruleset = crate::permission::RuleSet {
        version: "1.0".to_string(),
        rules: vec![],
        defaults: crate::permission::Defaults::default(),
        template_includes: vec![],
        agent_creators: std::collections::HashMap::new(),
    };
    let engine = Arc::new(crate::permission::PermissionEngine::new(ruleset));
    let skill_with = FileOpsSkill::with_engine(engine);
    let result = skill_with
        .execute("exists", serde_json::json!({"path": "Cargo.toml"}))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_file_ops_exists_with_permission() {
    let rule = Rule {
        name: "allow-exists".to_string(),
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
    };
    let ruleset = crate::permission::RuleSet {
        version: "1.0".to_string(),
        rules: vec![rule],
        defaults: crate::permission::Defaults::default(),
        template_includes: vec![],
        agent_creators: std::collections::HashMap::new(),
    };
    let engine = Arc::new(crate::permission::PermissionEngine::new(ruleset));
    let skill = FileOpsSkill::with_engine(engine);
    let result = skill
        .execute(
            "exists",
            serde_json::json!({"path": "Cargo.toml", "agent_id": "test-agent"}),
        )
        .await;
    assert!(
        result.is_ok(),
        "exists should succeed with permission: {:?}",
        result
    );
}

#[tokio::test]
async fn test_git_ops_status() {
    let skill = GitOpsSkill::new();
    let result = skill.execute("status", serde_json::json!({})).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_search() {
    let skill = SearchSkill::new();
    let result = skill
        .execute("search", serde_json::json!({"query": "rust programming"}))
        .await;
    assert!(result.is_ok());
    let value = result.unwrap();
    assert!(value.get("is_stub").and_then(|v| v.as_bool()) == Some(true));
}

#[test]
fn test_builtin_skills() {
    let skills = BuiltinSkills::all();
    assert_eq!(skills.len(), 7);
    assert_eq!(skills[0].manifest().name, "file_ops");
    assert_eq!(skills[1].manifest().name, "git_ops");
    assert_eq!(skills[2].manifest().name, "search");
    assert_eq!(skills[3].manifest().name, "permission_query");
    assert_eq!(skills[4].manifest().name, "skill_discovery");
    assert_eq!(skills[5].manifest().name, "coding_agent");
    assert_eq!(skills[6].manifest().name, "skill_creator");
}

#[tokio::test]
async fn test_skill_registry_with_builtins() {
    use crate::skills::SkillRegistry;
    let registry = SkillRegistry::new();
    for skill in builtin_skills() {
        registry.register(skill).await;
    }
    let skills: Vec<String> = registry.list().await;
    assert!(skills.contains(&"file_ops".to_string()));
    assert!(skills.contains(&"git_ops".to_string()));
    assert!(skills.contains(&"search".to_string()));
}
