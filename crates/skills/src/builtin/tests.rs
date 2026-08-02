//! Tests for built-in skills
use crate::builtin::{builtin_skills, BuiltinSkills, FileOpsSkill, GitOpsSkill, SearchSkill};
use crate::registry::Skill;

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
    // SkillCreator does not override execute() yet (Step 1.4).
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
