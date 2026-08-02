//! Tests for built-in skills
use crate::builtin::{
    builtin_skills, BuiltinSkills, FileOpsSkill, GitOpsSkill, SearchSkill, SkillDiscoverySkill,
};
use crate::registry::{Skill, SkillError};
use crate::{CodingAgentSkill, SkillCreatorSkill};

#[tokio::test]
async fn test_file_ops_body_not_empty() {
    let skill = FileOpsSkill::new();
    let body = skill.body();
    assert!(!body.is_empty());
    assert!(body.contains("File Operations Skill"));
}

#[tokio::test]
async fn test_git_ops_body_not_empty() {
    let skill = GitOpsSkill::new();
    let body = skill.body();
    assert!(!body.is_empty());
    assert!(body.contains("Git Operations Skill"));
}

#[tokio::test]
async fn test_search_body_not_empty() {
    let skill = SearchSkill::new();
    let body = skill.body();
    assert!(!body.is_empty());
    assert!(body.contains("Search Skill"));
}

#[tokio::test]
async fn test_file_ops_manifest() {
    let skill = FileOpsSkill::new();
    let m = skill.manifest();
    assert_eq!(m.name, "file_ops");
    assert_eq!(m.version, "1.0.0");
}

#[tokio::test]
async fn test_git_ops_manifest() {
    let skill = GitOpsSkill::new();
    let m = skill.manifest();
    assert_eq!(m.name, "git_ops");
    assert_eq!(m.version, "1.0.0");
}

#[tokio::test]
async fn test_search_manifest() {
    let skill = SearchSkill::new();
    let m = skill.manifest();
    assert_eq!(m.name, "search");
    assert_eq!(m.version, "1.0.0");
}

#[test]
fn test_builtin_skills_count() {
    let skills = BuiltinSkills::all();
    assert_eq!(skills.len(), 6);
}

#[test]
fn test_builtin_skills_names() {
    let skills = BuiltinSkills::all();
    let names: Vec<String> = skills.iter().map(|s| s.manifest().name.clone()).collect();
    assert!(names.contains(&"file_ops".to_string()));
    assert!(names.contains(&"git_ops".to_string()));
    assert!(names.contains(&"search".to_string()));
    assert!(names.contains(&"skill_discovery".to_string()));
    assert!(names.contains(&"coding_agent".to_string()));
    assert!(names.contains(&"skill_creator".to_string()));
}

#[test]
fn test_builtin_skills_all_have_body() {
    let skills = BuiltinSkills::all();
    for skill in &skills {
        let body = skill.body();
        assert!(
            !body.is_empty(),
            "skill '{}' body should not be empty",
            skill.manifest().name
        );
    }
}

#[test]
fn test_builtin_skills_all_have_listing_meta() {
    let skills = BuiltinSkills::all();
    for skill in &skills {
        let meta = skill.listing_meta();
        assert!(
            !meta.when_to_use.is_empty(),
            "skill '{}' listing_meta.when_to_use should not be empty",
            skill.manifest().name
        );
        assert!(
            !meta.effort.to_string().is_empty(),
            "skill '{}' listing_meta.effort should not be empty",
            skill.manifest().name
        );
    }
}

#[test]
fn test_skill_creator_and_coding_agent_are_user_invocable() {
    let skills = BuiltinSkills::all();
    for skill in &skills {
        let name = skill.manifest().name;
        let meta = skill.listing_meta();
        if name == "skill_creator" || name == "coding_agent" {
            assert!(
                meta.user_invocable,
                "skill '{}' should be user_invocable",
                name
            );
        }
    }
}

#[test]
fn test_file_ops_and_git_ops_are_not_user_invocable() {
    let skills = BuiltinSkills::all();
    for skill in &skills {
        let name = skill.manifest().name;
        let meta = skill.listing_meta();
        if name == "file_ops" || name == "git_ops" {
            assert!(
                !meta.user_invocable,
                "skill '{}' should not be user_invocable",
                name
            );
        }
    }
}

#[tokio::test]
async fn test_file_ops_execute_none_returns_capabilities() {
    let skill = FileOpsSkill::new();
    let result = skill.execute(None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "file_ops");
    assert!(v["supported_actions"].is_array());
}

#[tokio::test]
async fn test_git_ops_execute_none_returns_capabilities() {
    let skill = GitOpsSkill::new();
    let result = skill.execute(None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "git_ops");
    assert!(v["supported_actions"].is_array());
}

#[tokio::test]
async fn test_search_execute_none_returns_capabilities() {
    let skill = SearchSkill::new();
    let result = skill.execute(None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "search");
    assert!(v["supported_tools"].is_array());
}

#[tokio::test]
async fn test_all_bundled_skills_override_execute() {
    // All Bundled skills now override execute() — none should
    // return the body text.
    let skills = BuiltinSkills::all();
    for skill in &skills {
        let name = skill.manifest().name;
        let body = skill.body().to_string();
        let result = skill.execute(None).await.unwrap();
        assert_ne!(
            result, body,
            "skill '{name}' overrides execute(), should not return body"
        );
        // Result should be valid JSON
        let _: serde_json::Value = serde_json::from_str(&result).unwrap();
    }
}

#[tokio::test]
async fn test_skill_registry_with_builtins() {
    use crate::registry::BuiltinSkillRegistry;
    let registry = BuiltinSkillRegistry::new();
    for skill in builtin_skills() {
        registry.register(skill).await;
    }
    let skills: Vec<String> = registry.list().await;
    assert!(skills.contains(&"file_ops".to_string()));
    assert!(skills.contains(&"git_ops".to_string()));
    assert!(skills.contains(&"search".to_string()));
}

// ==========================================================================
// Step 1.5 — Comprehensive integration tests for all Bundled skills
// ==========================================================================

// --- Boundary: no args (None) returns valid JSON capabilities ---

#[tokio::test]
async fn test_file_ops_execute_none_returns_valid_json() {
    let skill = FileOpsSkill::new();
    let result = skill.execute(None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "file_ops");
    assert!(v["supported_actions"].as_array().unwrap().len() > 0);
}

#[tokio::test]
async fn test_git_ops_execute_none_returns_valid_json() {
    let skill = GitOpsSkill::new();
    let result = skill.execute(None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "git_ops");
    assert!(v["supported_actions"].as_array().unwrap().len() > 0);
}

#[tokio::test]
async fn test_search_execute_none_returns_valid_json() {
    let skill = SearchSkill::new();
    let result = skill.execute(None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "search");
    assert!(v["supported_tools"].as_array().unwrap().len() > 0);
}

#[tokio::test]
async fn test_discovery_execute_none_returns_valid_json() {
    let skill = SkillDiscoverySkill::new();
    let result = skill.execute(None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "skill_discovery");
    assert!(v["supported_actions"].as_array().unwrap().len() > 0);
}

#[tokio::test]
async fn test_coding_agent_execute_none_returns_valid_json() {
    let skill = CodingAgentSkill::new();
    let result = skill.execute(None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "coding_agent");
    assert!(v["supported_actions"].as_array().unwrap().len() > 0);
}

#[tokio::test]
async fn test_skill_creator_execute_none_returns_valid_json() {
    let skill = SkillCreatorSkill::new();
    let result = skill.execute(None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "skill_creator");
    assert!(v["supported_actions"].as_array().unwrap().len() > 0);
}

// --- Boundary: empty args ({}) returns valid JSON capabilities ---

#[tokio::test]
async fn test_file_ops_execute_empty_args() {
    let skill = FileOpsSkill::new();
    let result = skill.execute(Some(serde_json::json!({}))).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "file_ops");
}

#[tokio::test]
async fn test_git_ops_execute_empty_args() {
    let skill = GitOpsSkill::new();
    let result = skill.execute(Some(serde_json::json!({}))).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "git_ops");
}

#[tokio::test]
async fn test_search_execute_empty_args() {
    let skill = SearchSkill::new();
    let result = skill.execute(Some(serde_json::json!({}))).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "search");
}

#[tokio::test]
async fn test_discovery_execute_empty_args() {
    let skill = SkillDiscoverySkill::new();
    let result = skill.execute(Some(serde_json::json!({}))).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "skill_discovery");
}

#[tokio::test]
async fn test_coding_agent_execute_empty_args() {
    let skill = CodingAgentSkill::new();
    let result = skill.execute(Some(serde_json::json!({}))).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "coding_agent");
}

#[tokio::test]
async fn test_skill_creator_execute_empty_args() {
    let skill = SkillCreatorSkill::new();
    let result = skill.execute(Some(serde_json::json!({}))).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "skill_creator");
}

// --- Error paths: unknown action returns InvalidArgs ---

#[tokio::test]
async fn test_file_ops_unknown_action() {
    let skill = FileOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "bogus"})))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => assert!(msg.contains("unknown action")),
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_git_ops_unknown_action() {
    let skill = GitOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "bogus"})))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => assert!(msg.contains("unknown action")),
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_discovery_unknown_action() {
    let skill = SkillDiscoverySkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "bogus"})))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => assert!(msg.contains("unknown action")),
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_skill_creator_unknown_action() {
    let skill = SkillCreatorSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "bogus"})))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => assert!(msg.contains("unknown action")),
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

// --- Error paths: missing required params ---

#[tokio::test]
async fn test_file_ops_read_missing_path() {
    let skill = FileOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "read"})))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => assert!(msg.contains("path")),
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_file_ops_list_missing_path() {
    let skill = FileOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "list"})))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => assert!(msg.contains("path")),
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_file_ops_stat_missing_path() {
    let skill = FileOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "stat"})))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => assert!(msg.contains("path")),
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_discovery_find_missing_query() {
    let skill = SkillDiscoverySkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "find"})))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => assert!(msg.contains("query")),
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_discovery_install_missing_name() {
    let skill = SkillDiscoverySkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "install"})))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => assert!(msg.contains("name")),
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_skill_creator_create_missing_name() {
    let skill = SkillCreatorSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "create"})))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => assert!(msg.contains("name")),
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_skill_creator_validate_missing_path() {
    let skill = SkillCreatorSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "validate"})))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => assert!(msg.contains("path")),
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_skill_creator_edit_missing_field() {
    let skill = SkillCreatorSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "edit", "path": "x"})))
        .await
        .unwrap_err();
    match err {
        SkillError::InvalidArgs(msg) => assert!(msg.contains("field")),
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

// --- Boundary: args with action but no missing required params ---

#[tokio::test]
async fn test_file_ops_no_action_returns_capabilities() {
    let skill = FileOpsSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"extra": "data"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "file_ops");
    assert!(v["supported_actions"].is_array());
}

#[tokio::test]
async fn test_git_ops_no_action_returns_capabilities() {
    let skill = GitOpsSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"extra": "data"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "git_ops");
    assert!(v["supported_actions"].is_array());
}

#[tokio::test]
async fn test_discovery_no_action_returns_capabilities() {
    let skill = SkillDiscoverySkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"extra": "data"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "skill_discovery");
    assert!(v["supported_actions"].is_array());
}

#[tokio::test]
async fn test_skill_creator_no_action_returns_capabilities() {
    let skill = SkillCreatorSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"extra": "data"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "skill_creator");
    assert!(v["supported_actions"].is_array());
}

// --- State transition: all bundled skills override execute() ---

#[tokio::test]
async fn test_all_bundled_skills_execute_returns_valid_json_not_body() {
    let skills = BuiltinSkills::all();
    for skill in &skills {
        let name = skill.manifest().name;
        let body = skill.body().to_string();
        let result = skill.execute(None).await.unwrap();
        assert_ne!(
            result, body,
            "skill '{name}' execute() should not return body text"
        );
        // Must be valid JSON
        let v: serde_json::Value = serde_json::from_str(&result)
            .unwrap_or_else(|e| panic!("skill '{name}' result is not valid JSON: {e}"));
        // Must have a skill field identifying itself
        assert_eq!(
            v["skill"].as_str().unwrap(),
            name.as_str(),
            "skill '{name}' result 'skill' field should match its name"
        );
    }
}

#[tokio::test]
async fn test_all_bundled_skills_empty_args_returns_valid_json() {
    let skills = BuiltinSkills::all();
    for skill in &skills {
        let name = skill.manifest().name;
        let result = skill.execute(Some(serde_json::json!({}))).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&result)
            .unwrap_or_else(|e| panic!("skill '{name}' empty-args result is not valid JSON: {e}"));
        assert_eq!(v["skill"].as_str().unwrap(), name.as_str());
    }
}

// --- Normal path: each skill's execute() with valid args ---

#[tokio::test]
async fn test_file_ops_read_existing_file() {
    let skill = FileOpsSkill::new();
    let path = if std::path::Path::new("/etc/os-release").exists() {
        "/etc/os-release"
    } else {
        "/etc/hostname"
    };
    let result = skill
        .execute(Some(serde_json::json!({"action": "read", "path": path})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "read");
    assert_eq!(v["path"], path);
    assert!(v["content"].is_string());
    assert!(v["size_bytes"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn test_file_ops_list_existing_dir() {
    let skill = FileOpsSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"action": "list", "path": "/tmp"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "list");
    assert!(v["entries"].is_array());
}

#[tokio::test]
async fn test_file_ops_stat_existing_path() {
    let skill = FileOpsSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"action": "stat", "path": "/tmp"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "stat");
    assert_eq!(v["is_dir"], true);
}

#[tokio::test]
async fn test_search_with_query() {
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
}

#[tokio::test]
async fn test_coding_agent_with_task() {
    let skill = CodingAgentSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"task": "refactor auth"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "coding_agent");
    assert_eq!(v["action"], "delegate");
    assert_eq!(v["task"], "refactor auth");
    assert!(v["guidance"].is_string());
    assert!(v["agents"].as_array().unwrap().len() > 0);
}

#[tokio::test]
async fn test_skill_creator_create_with_params() {
    let skill = SkillCreatorSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "create",
            "name": "test_skill",
            "description": "A test skill"
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "create");
    assert_eq!(v["target"]["name"], "test_skill");
    assert!(v["template"].is_object());
}

#[tokio::test]
async fn test_skill_creator_validate_with_path() {
    let skill = SkillCreatorSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "validate",
            "path": "skills/test/SKILL.md"
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "validate");
    assert!(v["checks"].as_array().unwrap().len() > 0);
}

#[tokio::test]
async fn test_skill_creator_edit_with_params() {
    let skill = SkillCreatorSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({
            "action": "edit",
            "path": "skills/test/SKILL.md",
            "field": "description",
            "value": "updated"
        })))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["action"], "edit");
    assert_eq!(v["change"]["field"], "description");
    assert_eq!(v["change"]["value"], "updated");
}

// --- Error: nonexistent file/directory operations ---

#[tokio::test]
async fn test_file_ops_read_nonexistent() {
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
async fn test_file_ops_list_nonexistent() {
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
async fn test_file_ops_stat_nonexistent() {
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
async fn test_file_ops_read_directory_as_file() {
    let skill = FileOpsSkill::new();
    let err = skill
        .execute(Some(serde_json::json!({"action": "read", "path": "/tmp"})))
        .await
        .unwrap_err();
    assert!(matches!(err, SkillError::InvalidArgs(_)));
}

#[tokio::test]
async fn test_file_ops_list_file_as_dir() {
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

// --- Boundary: CodingAgentSkill with no task returns capabilities ---

#[tokio::test]
async fn test_coding_agent_no_task_returns_capabilities() {
    let skill = CodingAgentSkill::new();
    let result = skill
        .execute(Some(serde_json::json!({"action": "delegate"})))
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["skill"], "coding_agent");
    assert!(v["supported_actions"].is_array());
}
