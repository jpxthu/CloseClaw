//! Cross-skill integration tests for built-in skills.
//!
//! Per-skill tests live in `*_tests.rs` files. This file only contains
//! assertions that span multiple skills or test shared infrastructure.

use super::builder::{builtin_skills, trivial_validator};

// ==========================================================================
// Cross-skill manifest / body assertions
// ==========================================================================

#[test]
fn test_builtin_skills_all_have_manifest_fields() {
    let skills = builtin_skills(trivial_validator());
    for skill in &skills {
        let m = skill.manifest();
        assert!(
            !m.when_to_use.is_empty(),
            "skill '{}' manifest.when_to_use should not be empty",
            m.name
        );
        assert!(
            !m.effort.to_string().is_empty(),
            "skill '{}' manifest.effort should not be empty",
            m.name
        );
    }
}

#[test]
fn test_skill_creator_and_coding_agent_are_user_invocable() {
    let skills = builtin_skills(trivial_validator());
    for skill in &skills {
        let m = skill.manifest();
        if m.name == "skill_creator" || m.name == "coding_agent" {
            assert!(
                m.user_invocable,
                "skill '{}' should be user_invocable",
                m.name
            );
        }
    }
}

#[test]
fn test_file_ops_and_git_ops_are_not_user_invocable() {
    let skills = builtin_skills(trivial_validator());
    for skill in &skills {
        let m = skill.manifest();
        if m.name == "file_ops" || m.name == "git_ops" {
            assert!(
                !m.user_invocable,
                "skill '{}' should not be user_invocable",
                m.name
            );
        }
    }
}

// ==========================================================================
// State transition: all bundled skills override execute()
// ==========================================================================

#[tokio::test]
async fn test_all_bundled_skills_override_execute() {
    let skills = builtin_skills(trivial_validator());
    for skill in &skills {
        let name = skill.manifest().name;
        let body = skill.body().to_string();
        let result = skill.execute(None).await.unwrap();
        assert_ne!(
            result, body,
            "skill '{name}' overrides execute(), should not return body"
        );
        let _: serde_json::Value = serde_json::from_str(&result).unwrap();
    }
}

#[tokio::test]
async fn test_all_bundled_skills_execute_returns_valid_json_not_body() {
    let skills = builtin_skills(trivial_validator());
    for skill in &skills {
        let name = skill.manifest().name;
        let body = skill.body().to_string();
        let result = skill.execute(None).await.unwrap();
        assert_ne!(
            result, body,
            "skill '{name}' execute() should not return body text"
        );
        let v: serde_json::Value = serde_json::from_str(&result)
            .unwrap_or_else(|e| panic!("skill '{name}' result is not valid JSON: {e}"));
        assert_eq!(
            v["skill"].as_str().unwrap(),
            name.as_str(),
            "skill '{name}' result 'skill' field should match its name"
        );
    }
}

#[tokio::test]
async fn test_all_bundled_skills_empty_args_returns_valid_json() {
    let skills = builtin_skills(trivial_validator());
    for skill in &skills {
        let name = skill.manifest().name;
        let result = skill.execute(Some(serde_json::json!({}))).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&result)
            .unwrap_or_else(|e| panic!("skill '{name}' empty-args result is not valid JSON: {e}"));
        assert_eq!(v["skill"].as_str().unwrap(), name.as_str());
    }
}

// ==========================================================================
// Skill registry integration
// ==========================================================================

#[tokio::test]
async fn test_skill_registry_with_builtins() {
    use crate::registry::BuiltinSkillRegistry;
    let registry = BuiltinSkillRegistry::new();
    for skill in builtin_skills(trivial_validator()) {
        registry.register(skill).await;
    }
    let skills: Vec<String> = registry.list().await;
    assert!(skills.contains(&"file_ops".to_string()));
    assert!(skills.contains(&"git_ops".to_string()));
    assert!(skills.contains(&"search".to_string()));
}
