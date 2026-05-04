use super::super::types::{SkillContext, SkillEffort, SkillManifest, SkillSource};
use super::DiskSkill;
use super::DiskSkillRegistry;
use std::path::{Path, PathBuf};

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
            user_invocable: false,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
    }
}

#[test]
fn test_new_and_len() {
    let r = DiskSkillRegistry::new(vec![skill("a", SkillSource::Bundled)]);
    assert_eq!(r.len(), 1);
    assert!(!r.is_empty());
}

#[test]
fn test_empty_registry() {
    let r = DiskSkillRegistry::new(vec![]);
    assert!(r.is_empty());
    assert!(r.get("any").is_none());
    assert!(!r.contains("any"));
    assert!(r.list().is_empty());
    assert!(r.filter_by_source(SkillSource::Bundled).is_empty());
}

#[test]
fn test_get() {
    let r = DiskSkillRegistry::new(vec![
        skill("foo", SkillSource::Agent),
        skill("bar", SkillSource::Project),
    ]);
    assert_eq!(r.get("foo").unwrap().manifest.name, "foo");
    assert!(r.get("baz").is_none());
}

#[test]
fn test_contains() {
    let r = DiskSkillRegistry::new(vec![skill("x", SkillSource::Global)]);
    assert!(r.contains("x"));
    assert!(!r.contains("y"));
}

#[test]
fn test_list() {
    let r = DiskSkillRegistry::new(vec![
        skill("z", SkillSource::Bundled),
        skill("a", SkillSource::Global),
    ]);
    assert_eq!(r.list(), vec!["z", "a"]);
}

#[test]
fn test_filter_by_source() {
    let r = DiskSkillRegistry::new(vec![
        skill("b1", SkillSource::Bundled),
        skill("g1", SkillSource::Global),
        skill("b2", SkillSource::Bundled),
    ]);
    assert_eq!(r.filter_by_source(SkillSource::Bundled).len(), 2);
    assert_eq!(r.filter_by_source(SkillSource::Global).len(), 1);
    assert_eq!(r.filter_by_source(SkillSource::Agent).len(), 0);
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
            user_invocable: false,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
    }
}

fn skill_with_when_to_use(name: &str, source: SkillSource, when_to_use: &str) -> DiskSkill {
    DiskSkill {
        source,
        manifest: SkillManifest {
            name: name.into(),
            description: format!("desc of {}", name),
            allowed_tools: vec![],
            when_to_use: when_to_use.into(),
            context: SkillContext::default(),
            agent: String::new(),
            agent_id: String::new(),
            effort: SkillEffort::default(),
            paths: vec![],
            user_invocable: false,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
    }
}

#[test]
fn test_generate_listing_empty() {
    let r = DiskSkillRegistry::new(vec![]);
    assert_eq!(r.generate_listing(None), "");
    assert_eq!(r.generate_listing(Some("agent1")), "");
}

#[test]
fn test_generate_listing_single() {
    let r = DiskSkillRegistry::new(vec![skill("foo", SkillSource::Bundled)]);
    let listing = r.generate_listing(None);
    assert!(listing.contains("**foo**"));
    assert!(listing.contains("desc of foo"));
}

#[test]
fn test_generate_listing_sorted_by_priority_and_name() {
    let r = DiskSkillRegistry::new(vec![
        skill("z_bundled", SkillSource::Bundled),
        skill("a_bundled", SkillSource::Bundled),
        skill("z_global", SkillSource::Global),
        skill("a_global", SkillSource::Global),
        skill("z_agent", SkillSource::Agent),
        skill("a_agent", SkillSource::Agent),
    ]);
    let listing = r.generate_listing(None);
    let lines: Vec<&str> = listing.lines().collect();
    assert_eq!(lines.len(), 6);
    // Bundled before Global before Agent
    assert!(listing.find("**a_bundled**").unwrap() < listing.find("**a_global**").unwrap());
    assert!(listing.find("**a_global**").unwrap() < listing.find("**a_agent**").unwrap());
    // Within Bundled, alphabetical order
    assert!(listing.find("**a_bundled**").unwrap() < listing.find("**z_bundled**").unwrap());
}

#[test]
fn test_generate_listing_agent_id_filter() {
    let r = DiskSkillRegistry::new(vec![
        skill_with_agent_id("skill_a", SkillSource::Agent, "agent1"),
        skill_with_agent_id("skill_b", SkillSource::Agent, "agent2"),
        skill_with_agent_id("skill_c", SkillSource::Agent, ""), // no restriction
    ]);
    // None = no filter, all 3 returned
    assert_eq!(r.generate_listing(None).lines().count(), 3);
    // agent1 filter = skill_a (matches) + skill_c (no restriction)
    let listing = r.generate_listing(Some("agent1"));
    assert!(listing.contains("**skill_a**"));
    assert!(listing.contains("**skill_c**"));
    assert!(!listing.contains("**skill_b**"));
}

#[test]
fn test_generate_listing_when_to_use() {
    let r = DiskSkillRegistry::new(vec![
        skill_with_when_to_use("foo", SkillSource::Bundled, "Use when you need foo"),
        skill("bar", SkillSource::Bundled), // no when_to_use
    ]);
    let listing = r.generate_listing(None);
    assert!(listing.contains(" — Use when you need foo"));
    // bar should NOT have the dash separator
    let bar_line = listing.lines().find(|l| l.contains("**bar**")).unwrap();
    assert!(!bar_line.contains(" — "));
}

// --- helpers for conditional/ paths tests ---

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
            user_invocable: false,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
    }
}

// --- conditional_skills tests ---

#[test]
fn test_conditional_skills_returns_skills_with_paths() {
    let r = DiskSkillRegistry::new(vec![
        skill_with_paths("rs-tools", SkillSource::Bundled, vec!["**/*.rs".into()]),
        skill_with_paths("toml-tools", SkillSource::Bundled, vec!["**/*.toml".into()]),
        skill("no-cond", SkillSource::Bundled),
    ]);
    let cond = r.conditional_skills();
    assert_eq!(cond.len(), 2);
    let names: Vec<&str> = cond.iter().map(|s| s.manifest.name.as_str()).collect();
    assert!(names.contains(&"rs-tools"));
    assert!(names.contains(&"toml-tools"));
    assert!(!names.contains(&"no-cond"));
}

#[test]
fn test_conditional_skills_returns_empty_when_no_paths_defined() {
    let r = DiskSkillRegistry::new(vec![
        skill("a", SkillSource::Bundled),
        skill("b", SkillSource::Global),
    ]);
    assert!(r.conditional_skills().is_empty());
}

#[test]
fn test_conditional_skills_returns_empty_for_empty_registry() {
    let r = DiskSkillRegistry::new(vec![]);
    assert!(r.conditional_skills().is_empty());
}

// --- matches_paths tests ---

#[test]
fn test_matches_paths_positive_match() {
    let r = DiskSkillRegistry::new(vec![skill_with_paths(
        "rs-skill",
        SkillSource::Bundled,
        vec!["**/*.rs".into()],
    )]);
    assert!(r.matches_paths("rs-skill", &[Path::new("src/main.rs")]));
}

#[test]
fn test_matches_paths_negative_match() {
    let r = DiskSkillRegistry::new(vec![skill_with_paths(
        "rs-skill",
        SkillSource::Bundled,
        vec!["**/*.rs".into()],
    )]);
    assert!(!r.matches_paths("rs-skill", &[Path::new("src/main.ts")]));
}

#[test]
fn test_matches_paths_multi_pattern_any_wins() {
    let r = DiskSkillRegistry::new(vec![skill_with_paths(
        "multi",
        SkillSource::Bundled,
        vec!["**/*.rs".into(), "**/*.toml".into()],
    )]);
    assert!(r.matches_paths("multi", &[Path::new("src/main.rs")]));
    assert!(r.matches_paths("multi", &[Path::new("Cargo.toml")]));
    assert!(!r.matches_paths("multi", &[Path::new("src/main.ts")]));
}

#[test]
fn test_matches_paths_empty_paths_arg_returns_false() {
    let r = DiskSkillRegistry::new(vec![skill_with_paths(
        "rs-skill",
        SkillSource::Bundled,
        vec!["**/*.rs".into()],
    )]);
    assert!(!r.matches_paths("rs-skill", &[]));
}

#[test]
fn test_matches_paths_nonexistent_skill_returns_false() {
    let r = DiskSkillRegistry::new(vec![skill_with_paths(
        "rs-skill",
        SkillSource::Bundled,
        vec!["**/*.rs".into()],
    )]);
    assert!(!r.matches_paths("no-such-skill", &[Path::new("src/main.rs")]));
}

#[test]
fn test_matches_paths_skill_without_paths_returns_false() {
    let r = DiskSkillRegistry::new(vec![skill("plain", SkillSource::Bundled)]);
    assert!(!r.matches_paths("plain", &[Path::new("src/main.rs")]));
}

#[test]
fn test_matches_paths_multiple_input_paths_any_match() {
    let r = DiskSkillRegistry::new(vec![skill_with_paths(
        "multi-skill",
        SkillSource::Bundled,
        vec!["**/*.md".into()],
    )]);
    // One of the input paths matches
    assert!(r.matches_paths(
        "multi-skill",
        &[Path::new("src/main.rs"), Path::new("docs/README.md")]
    ));
    // None match
    assert!(!r.matches_paths(
        "multi-skill",
        &[Path::new("src/main.rs"), Path::new("src/lib.rs")]
    ));
}

// --- generate_listing conditional annotation tests ---

#[test]
fn test_generate_listing_conditional_annotation_shown_for_paths() {
    let r = DiskSkillRegistry::new(vec![skill_with_paths(
        "rs-skill",
        SkillSource::Bundled,
        vec!["**/*.rs".into()],
    )]);
    let listing = r.generate_listing(None);
    assert!(listing.contains("⚡ auto-activates on: **/*.rs"));
}

#[test]
fn test_generate_listing_conditional_annotation_multi_patterns() {
    let r = DiskSkillRegistry::new(vec![skill_with_paths(
        "multi",
        SkillSource::Bundled,
        vec!["**/*.rs".into(), "**/*.toml".into()],
    )]);
    let listing = r.generate_listing(None);
    assert!(listing.contains("⚡ auto-activates on: **/*.rs, **/*.toml"));
}

#[test]
fn test_generate_listing_conditional_annotation_not_shown_without_paths() {
    let r = DiskSkillRegistry::new(vec![skill("plain", SkillSource::Bundled)]);
    let listing = r.generate_listing(None);
    assert!(!listing.contains("⚡"));
    assert!(!listing.contains("auto-activates"));
}

#[test]
fn test_generate_listing_conditional_annotation_mixed() {
    let r = DiskSkillRegistry::new(vec![
        skill_with_paths("cond", SkillSource::ExtraDirs, vec!["**/*.rs".into()]),
        skill("plain1", SkillSource::Bundled),
        skill_with_paths("cond2", SkillSource::Global, vec!["**/*.md".into()]),
        skill("plain2", SkillSource::Global),
    ]);
    let listing = r.generate_listing(None);
    let lines: Vec<&str> = listing.lines().collect();
    assert_eq!(lines.len(), 4, "all 4 skills should appear");
    // cond has annotation, cond2 has annotation
    let cond_line = lines.iter().find(|l| l.contains("**cond**")).unwrap();
    assert!(cond_line.contains("⚡ auto-activates on: **/*.rs"));
    let cond2_line = lines.iter().find(|l| l.contains("**cond2**")).unwrap();
    assert!(cond2_line.contains("⚡ auto-activates on: **/*.md"));
    // plain1 and plain2 no annotation
    let plain1_line = lines.iter().find(|l| l.contains("**plain1**")).unwrap();
    assert!(!plain1_line.contains("⚡"));
    let plain2_line = lines.iter().find(|l| l.contains("**plain2**")).unwrap();
    assert!(!plain2_line.contains("⚡"));
}

#[test]
fn test_generate_listing_conditional_with_agent_id_filter() {
    let s1 = DiskSkill {
        source: SkillSource::Agent,
        manifest: SkillManifest {
            name: "agent1-cond".into(),
            description: "desc".into(),
            allowed_tools: vec![],
            when_to_use: String::new(),
            context: SkillContext::default(),
            agent: String::new(),
            agent_id: "agent1".into(),
            effort: SkillEffort::default(),
            paths: vec!["**/*.rs".into()],
            user_invocable: false,
        },
        readme_path: PathBuf::from("/skills/agent1-cond/SKILL.md"),
        skill_dir: PathBuf::from("/skills/agent1-cond"),
    };
    let s2 = DiskSkill {
        source: SkillSource::Agent,
        manifest: SkillManifest {
            name: "any-cond".into(),
            description: "desc".into(),
            allowed_tools: vec![],
            when_to_use: String::new(),
            context: SkillContext::default(),
            agent: String::new(),
            agent_id: String::new(),
            effort: SkillEffort::default(),
            paths: vec!["**/*.md".into()],
            user_invocable: false,
        },
        readme_path: PathBuf::from("/skills/any-cond/SKILL.md"),
        skill_dir: PathBuf::from("/skills/any-cond"),
    };
    let s3 = DiskSkill {
        source: SkillSource::Agent,
        manifest: SkillManifest {
            name: "agent2-cond".into(),
            description: "desc".into(),
            allowed_tools: vec![],
            when_to_use: String::new(),
            context: SkillContext::default(),
            agent: String::new(),
            agent_id: "agent2".into(),
            effort: SkillEffort::default(),
            paths: vec!["**/*.toml".into()],
            user_invocable: false,
        },
        readme_path: PathBuf::from("/skills/agent2-cond/SKILL.md"),
        skill_dir: PathBuf::from("/skills/agent2-cond"),
    };
    let s4 = DiskSkill {
        source: SkillSource::Agent,
        manifest: SkillManifest {
            name: "plain".into(),
            description: "desc".into(),
            allowed_tools: vec![],
            when_to_use: String::new(),
            context: SkillContext::default(),
            agent: String::new(),
            agent_id: String::new(),
            effort: SkillEffort::default(),
            paths: vec![],
            user_invocable: false,
        },
        readme_path: PathBuf::from("/skills/plain/SKILL.md"),
        skill_dir: PathBuf::from("/skills/plain"),
    };

    let r = DiskSkillRegistry::new(vec![s1, s2, s3, s4]);

    // With agent1 filter: agent1-cond (matches agent1) + any-cond (no restriction) + plain (no restriction)
    let listing = r.generate_listing(Some("agent1"));
    assert!(listing.contains("**agent1-cond**"));
    assert!(listing.contains("**any-cond**"));
    assert!(listing.contains("**plain**"));
    assert!(!listing.contains("**agent2-cond**"));
    // agent1-cond has annotation
    let agent1_line = listing
        .lines()
        .find(|l| l.contains("**agent1-cond**"))
        .unwrap();
    assert!(agent1_line.contains("⚡ auto-activates on: **/*.rs"));
    // any-cond has annotation
    let any_line = listing
        .lines()
        .find(|l| l.contains("**any-cond**"))
        .unwrap();
    assert!(any_line.contains("⚡ auto-activates on: **/*.md"));
    // plain has no annotation
    let plain_line = listing.lines().find(|l| l.contains("**plain**")).unwrap();
    assert!(!plain_line.contains("⚡"));
}
