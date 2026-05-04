//! Tests for builder skill listing injection (src/system_prompt/builder.rs)

use closeclaw::skills::disk::types::{SkillContext, SkillEffort, SkillManifest, SkillSource};
use closeclaw::skills::disk::DiskSkill;
use closeclaw::skills::DiskSkillRegistry;
use closeclaw::system_prompt::builder::build_from_workspace;
use closeclaw::system_prompt::sections::invalidate_all_sections;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

fn make_test_skill(name: &str, description: &str, when_to_use: &str, agent_id: &str) -> DiskSkill {
    DiskSkill {
        source: SkillSource::Bundled,
        manifest: SkillManifest {
            name: name.into(),
            description: description.into(),
            allowed_tools: vec![],
            when_to_use: when_to_use.into(),
            context: SkillContext::default(),
            agent: String::new(),
            agent_id: agent_id.into(),
            effort: SkillEffort::default(),
            paths: vec![],
            user_invocable: false,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
    }
}

#[test]
fn test_build_from_workspace_skill_listing_injected() {
    invalidate_all_sections();

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("IDENTITY.md"), "test identity").unwrap();
    std::fs::write(dir.path().join("SOUL.md"), "test soul").unwrap();
    std::fs::write(dir.path().join("MEMORY.md"), "test memory").unwrap();

    let skill = make_test_skill("testskill", "A test skill", "Use when testing", "eda");
    let registry = DiskSkillRegistry::new(vec![skill]);
    let registry_arc = Arc::new(RwLock::new(Some(registry)));

    let result = build_from_workspace(dir.path(), vec![], Some(registry_arc), Some("eda"), None);

    assert!(
        result.contains("## Available Skills"),
        "skill listing should appear in prompt: {}",
        result
    );
    assert!(
        result.contains("**testskill**"),
        "skill name should appear: {}",
        result
    );
    assert!(
        result.contains("Use when testing"),
        "when_to_use should appear: {}",
        result
    );
}

#[test]
fn test_build_from_workspace_no_skill_info_no_section() {
    invalidate_all_sections();

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("IDENTITY.md"), "test identity").unwrap();
    std::fs::write(dir.path().join("SOUL.md"), "test soul").unwrap();
    std::fs::write(dir.path().join("MEMORY.md"), "test memory").unwrap();

    let result = build_from_workspace(dir.path(), vec![], None, None, None);
    assert!(!result.contains("## Available Skills"));
}

#[test]
fn test_build_from_workspace_empty_listing_no_section() {
    invalidate_all_sections();

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("IDENTITY.md"), "test identity").unwrap();
    std::fs::write(dir.path().join("SOUL.md"), "test soul").unwrap();
    std::fs::write(dir.path().join("MEMORY.md"), "test memory").unwrap();

    let registry = DiskSkillRegistry::new(vec![]);
    let registry_arc = Arc::new(RwLock::new(Some(registry)));
    let result = build_from_workspace(dir.path(), vec![], Some(registry_arc), Some("eda"), None);
    assert!(!result.contains("## Available Skills"));
}

#[test]
fn test_build_from_workspace_skill_section_not_duplicated() {
    invalidate_all_sections();

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("IDENTITY.md"), "test identity").unwrap();
    std::fs::write(dir.path().join("SOUL.md"), "test soul").unwrap();
    std::fs::write(dir.path().join("MEMORY.md"), "test memory").unwrap();

    let skill = make_test_skill("unique_skill", "desc", "", "eda");
    let registry = DiskSkillRegistry::new(vec![skill]);
    let registry_arc = Arc::new(RwLock::new(Some(registry)));

    let result = build_from_workspace(dir.path(), vec![], Some(registry_arc), Some("eda"), None);

    let count = result.matches("## Available Skills").count();
    assert_eq!(
        count, 1,
        "skill_listing should appear exactly once in: {}",
        result
    );
}
