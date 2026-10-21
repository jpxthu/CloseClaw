//! Tests for built-in skill assembly (`builtin_skills`).

use super::builder::{builtin_skills, trivial_validator};

#[test]
fn test_builtin_skills_returns_seven_expected_skills() {
    let skills = builtin_skills(trivial_validator());
    assert_eq!(skills.len(), 7);

    let mut names: Vec<String> = skills.iter().map(|s| s.manifest().name.clone()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "coding_agent",
            "create_workflow",
            "file_ops",
            "git_ops",
            "search",
            "skill_creator",
            "skill_discovery"
        ]
    );

    let unique: std::collections::HashSet<&str> = names.iter().map(String::as_str).collect();
    assert_eq!(unique.len(), 7, "builtin skill names should be unique");
}

#[test]
fn test_builtin_skills_all_have_manifests() {
    let skills = builtin_skills(trivial_validator());
    for skill in &skills {
        let m = skill.manifest();
        assert!(
            !m.name.is_empty(),
            "skill manifest name should not be empty"
        );
        assert!(
            !m.description.is_empty(),
            "skill manifest description should not be empty"
        );
    }
}

#[test]
fn test_builtin_skills_all_have_body() {
    let skills = builtin_skills(trivial_validator());
    for skill in &skills {
        let body = skill.body();
        assert!(
            !body.is_empty(),
            "skill '{}' body should not be empty",
            skill.manifest().name
        );
    }
}
