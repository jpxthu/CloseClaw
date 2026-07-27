//! Tests for file_ops skill
use crate::builtin::FileOpsSkill;
use crate::registry::Skill;

#[tokio::test]
async fn test_file_ops_body_not_empty() {
    let skill = FileOpsSkill::new();
    let body = skill.body();
    assert!(!body.is_empty());
    assert!(body.contains("File Operations Skill"));
}

#[tokio::test]
async fn test_file_ops_manifest() {
    let skill = FileOpsSkill::new();
    let m = skill.manifest();
    assert_eq!(m.name, "file_ops");
    assert_eq!(m.version, "1.0.0");
    assert!(!m.description.is_empty());
}
