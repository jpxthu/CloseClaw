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
