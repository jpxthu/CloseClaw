use super::super::super::types::{SkillContext, SkillEffort, SkillManifest, SkillSource};
use super::super::super::DiskSkill;
use super::super::super::DiskSkillRegistry;
use std::path::Path;
use std::path::PathBuf;

// --- user_invocable filtering tests ---

fn skill_with_invocable(name: &str, source: SkillSource, user_invocable: bool) -> DiskSkill {
    DiskSkill {
        source,
        manifest: SkillManifest {
            name: name.into(),
            description: format!("desc of {}", name),
            allowed_tools: vec![],
            when_to_use: String::new(),
            context: SkillContext::default(),
            agent: String::new(),
            agent_id: String::new(),
            effort: SkillEffort::default(),
            paths: vec![],
            user_invocable,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
        body: String::new(),
    }
}

fn skill(name: &str, source: SkillSource) -> DiskSkill {
    DiskSkill {
        source,
        manifest: SkillManifest {
            name: name.into(),
            description: format!("desc of {}", name),
            allowed_tools: vec![],
            when_to_use: String::new(),
            context: SkillContext::default(),
            agent: String::new(),
            agent_id: String::new(),
            effort: SkillEffort::default(),
            paths: vec![],
            user_invocable: true,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
        body: String::new(),
    }
}

fn skill_with_agent_id(name: &str, source: SkillSource, agent_id: &str) -> DiskSkill {
    DiskSkill {
        source,
        manifest: SkillManifest {
            name: name.into(),
            description: format!("desc of {}", name),
            allowed_tools: vec![],
            when_to_use: String::new(),
            context: SkillContext::default(),
            agent: String::new(),
            agent_id: agent_id.into(),
            effort: SkillEffort::default(),
            paths: vec![],
            user_invocable: true,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
        body: String::new(),
    }
}

fn skill_with_paths(name: &str, source: SkillSource, paths: Vec<String>) -> DiskSkill {
    DiskSkill {
        source,
        manifest: SkillManifest {
            name: name.into(),
            description: format!("desc of {}", name),
            allowed_tools: vec![],
            when_to_use: String::new(),
            context: SkillContext::default(),
            agent: String::new(),
            agent_id: String::new(),
            effort: SkillEffort::default(),
            paths,
            user_invocable: true,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
        body: String::new(),
    }
}

#[test]
fn test_generate_listing_excludes_user_invocable_false() {
    let r = DiskSkillRegistry::new(vec![skill_with_invocable(
        "hidden",
        SkillSource::Bundled,
        false,
    )]);
    let listing = r.generate_listing(None, None);
    assert!(
        listing.is_empty(),
        "user_invocable: false skill must not appear in listing"
    );
}
#[test]
fn test_generate_listing_includes_user_invocable_true() {
    let r = DiskSkillRegistry::new(vec![skill("visible", SkillSource::Bundled)]);
    let listing = r.generate_listing(None, None);
    assert!(listing.contains("**visible**"));
    assert!(listing.contains("desc of visible"));
}
#[test]
fn test_generate_listing_mixed_user_invocable() {
    let r = DiskSkillRegistry::new(vec![
        skill("visible1", SkillSource::Bundled),
        skill_with_invocable("hidden1", SkillSource::Bundled, false),
        skill("visible2", SkillSource::Global),
        skill_with_invocable("hidden2", SkillSource::Agent, false),
    ]);
    let listing = r.generate_listing(None, None);
    assert!(listing.contains("**visible1**"));
    assert!(listing.contains("**visible2**"));
    assert!(!listing.contains("**hidden1**"));
    assert!(!listing.contains("**hidden2**"));
    assert_eq!(
        listing.lines().count(),
        2,
        "only user_invocable: true skills should appear"
    );
}
#[test]
fn test_user_invocable_false_still_gettable() {
    let r = DiskSkillRegistry::new(vec![skill_with_invocable(
        "hidden",
        SkillSource::Bundled,
        false,
    )]);
    let found = r.get("hidden");
    assert!(
        found.is_some(),
        "get() must still find user_invocable: false skills"
    );
    assert_eq!(found.unwrap().manifest.name, "hidden");
    assert!(!found.unwrap().manifest.user_invocable);
}
#[test]
fn test_user_invocable_false_with_agent_id_cross_filter() {
    let mut hidden = skill_with_agent_id("hidden_agent", SkillSource::Agent, "agent1");
    hidden.manifest.user_invocable = false;
    let r = DiskSkillRegistry::new(vec![
        hidden,
        skill_with_agent_id("visible_agent", SkillSource::Agent, "agent1"),
        skill("always_visible", SkillSource::Bundled),
    ]);
    let listing = r.generate_listing(Some("agent1"), None);
    assert!(listing.contains("**visible_agent**"));
    assert!(listing.contains("**always_visible**"));
    assert!(!listing.contains("**hidden_agent**"));
    let listing_all = r.generate_listing(None, None);
    assert!(listing_all.contains("**visible_agent**"));
    assert!(listing_all.contains("**always_visible**"));
    assert!(!listing_all.contains("**hidden_agent**"));
}
#[test]
fn test_user_invocable_false_still_findable_by_paths() {
    let mut hidden = skill_with_paths("hidden_rs", SkillSource::Bundled, vec!["**/*.rs".into()]);
    hidden.manifest.user_invocable = false;
    let r = DiskSkillRegistry::new(vec![
        hidden,
        skill_with_paths("visible_rs", SkillSource::Global, vec!["**/*.rs".into()]),
    ]);
    let matched = r.find_matching_skills(&[Path::new("src/main.rs")]);
    assert_eq!(matched.len(), 2, "both skills should be matched by paths");
    let names: Vec<&str> = matched.iter().map(|s| s.manifest.name.as_str()).collect();
    assert!(names.contains(&"hidden_rs"));
    assert!(names.contains(&"visible_rs"));
}
