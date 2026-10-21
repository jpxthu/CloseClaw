//! Tests for built-in skill assembly (`builtin_skills`).

use super::builder::{builtin_skills, trivial_validator};

#[test]
fn test_builtin_skills_returns_seven_skills() {
    let skills = builtin_skills(trivial_validator());
    assert_eq!(skills.len(), 7);
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
fn test_builtin_skills_names() {
    let skills = builtin_skills(trivial_validator());
    let names: Vec<String> = skills.iter().map(|s| s.manifest().name.clone()).collect();
    assert!(names.iter().any(|n| n == "file_ops"));
    assert!(names.iter().any(|n| n == "git_ops"));
    assert!(names.iter().any(|n| n == "search"));
    assert!(names.iter().any(|n| n == "skill_discovery"));
}

#[test]
fn test_builtin_skills_function() {
    let skills = builtin_skills(trivial_validator());
    assert_eq!(skills.len(), 7);
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
