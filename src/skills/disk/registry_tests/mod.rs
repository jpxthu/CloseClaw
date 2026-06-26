use super::super::types::{SkillContext, SkillEffort, SkillManifest, SkillSource};
use super::DiskSkill;
use super::DiskSkillRegistry;
use std::path::{Path, PathBuf};

mod user_invocable;

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
            user_invocable: true,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
        body: String::new(),
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
            user_invocable: true,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
        body: String::new(),
    }
}

#[test]
fn test_generate_listing_empty() {
    let r = DiskSkillRegistry::new(vec![]);
    assert_eq!(r.generate_listing(None, None), "");
    assert_eq!(r.generate_listing(Some("agent1"), None), "");
}
#[test]
fn test_generate_listing_single() {
    let r = DiskSkillRegistry::new(vec![skill("foo", SkillSource::Bundled)]);
    let listing = r.generate_listing(None, None);
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
    let listing = r.generate_listing(None, None);
    let lines: Vec<&str> = listing.lines().collect();
    assert_eq!(lines.len(), 6);
    // Agent before Global before Bundled (Project > Agent > Global > ExtraDirs > Bundled)
    assert!(listing.find("**a_agent**").unwrap() < listing.find("**a_global**").unwrap());
    assert!(listing.find("**a_global**").unwrap() < listing.find("**a_bundled**").unwrap());
    // Within Agent, alphabetical order
    assert!(listing.find("**a_agent**").unwrap() < listing.find("**z_agent**").unwrap());
}
#[test]
fn test_generate_listing_agent_id_filter() {
    let r = DiskSkillRegistry::new(vec![
        skill_with_agent_id("skill_a", SkillSource::Agent, "agent1"),
        skill_with_agent_id("skill_b", SkillSource::Agent, "agent2"),
        skill_with_agent_id("skill_c", SkillSource::Agent, ""),
    ]);
    // None = no filter, all 3 returned
    assert_eq!(r.generate_listing(None, None).lines().count(), 3);
    let listing = r.generate_listing(Some("agent1"), None);
    assert!(listing.contains("**skill_a**"));
    assert!(listing.contains("**skill_c**"));
    assert!(!listing.contains("**skill_b**"));
}
#[test]
fn test_generate_listing_when_to_use() {
    let r = DiskSkillRegistry::new(vec![
        skill_with_when_to_use("foo", SkillSource::Bundled, "Use when you need foo"),
        skill("bar", SkillSource::Bundled),
    ]);
    let listing = r.generate_listing(None, None);
    assert!(listing.contains(" — Use when you need foo"));
    assert!(!listing
        .lines()
        .any(|l| l.contains("**bar**") && l.contains(" — ")));
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
    assert!(r.matches_paths(
        "multi-skill",
        &[Path::new("src/main.rs"), Path::new("docs/README.md")],
    ));
    assert!(!r.matches_paths(
        "multi-skill",
        &[Path::new("src/main.rs"), Path::new("src/lib.rs")],
    ));
}

#[test]
fn test_generate_listing_conditional_annotation_shown_for_paths() {
    let r = DiskSkillRegistry::new(vec![skill_with_paths(
        "rs-skill",
        SkillSource::Bundled,
        vec!["**/*.rs".into()],
    )]);
    let listing = r.generate_listing(None, None);
    assert!(listing.contains("⚡ auto-activates on: **/*.rs"));
}
#[test]
fn test_generate_listing_conditional_annotation_multi_patterns() {
    let r = DiskSkillRegistry::new(vec![skill_with_paths(
        "multi",
        SkillSource::Bundled,
        vec!["**/*.rs".into(), "**/*.toml".into()],
    )]);
    let listing = r.generate_listing(None, None);
    assert!(listing.contains("⚡ auto-activates on: **/*.rs, **/*.toml"));
}
#[test]
fn test_generate_listing_conditional_annotation_not_shown_without_paths() {
    let r = DiskSkillRegistry::new(vec![skill("plain", SkillSource::Bundled)]);
    let listing = r.generate_listing(None, None);
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
    let listing = r.generate_listing(None, None);
    let lines: Vec<&str> = listing.lines().collect();
    assert_eq!(lines.len(), 4, "all 4 skills should appear");
    let cond = lines.iter().find(|l| l.contains("**cond**")).unwrap();
    assert!(cond.contains("⚡ auto-activates on: **/*.rs"));
    let cond2 = lines.iter().find(|l| l.contains("**cond2**")).unwrap();
    assert!(cond2.contains("⚡ auto-activates on: **/*.md"));
    assert!(!lines
        .iter()
        .any(|l| l.contains("**plain1**") && l.contains("⚡")));
    assert!(!lines
        .iter()
        .any(|l| l.contains("**plain2**") && l.contains("⚡")));
}
#[test]
fn test_generate_listing_conditional_with_agent_id_filter() {
    let mut s1 = skill_with_paths("agent1-cond", SkillSource::Agent, vec!["**/*.rs".into()]);
    s1.manifest.agent_id = "agent1".into();
    let s2 = skill_with_paths("any-cond", SkillSource::Agent, vec!["**/*.md".into()]);
    let mut s3 = skill_with_paths("agent2-cond", SkillSource::Agent, vec!["**/*.toml".into()]);
    s3.manifest.agent_id = "agent2".into();
    let r = DiskSkillRegistry::new(vec![s1, s2, s3, skill("plain", SkillSource::Agent)]);
    let listing = r.generate_listing(Some("agent1"), None);
    assert!(listing.contains("**agent1-cond**"));
    assert!(listing.contains("**any-cond**"));
    assert!(listing.contains("**plain**"));
    assert!(!listing.contains("**agent2-cond**"));
    let a1 = listing
        .lines()
        .find(|l| l.contains("**agent1-cond**"))
        .unwrap();
    assert!(a1.contains("⚡ auto-activates on: **/*.rs"));
    let a2 = listing
        .lines()
        .find(|l| l.contains("**any-cond**"))
        .unwrap();
    assert!(a2.contains("⚡ auto-activates on: **/*.md"));
    let pl = listing.lines().find(|l| l.contains("**plain**")).unwrap();
    assert!(!pl.contains("⚡"));
}

#[test]
fn test_find_matching_skills_basic_and_edge_cases() {
    let rs = vec![skill_with_paths(
        "s",
        SkillSource::Bundled,
        vec!["**/*.rs".into()],
    )];
    let r = DiskSkillRegistry::new(rs);
    assert!(r.find_matching_skills(&[]).is_empty());
    assert!(r.find_matching_skills(&[Path::new("a.ts")]).is_empty());
    assert!(DiskSkillRegistry::new(vec![])
        .find_matching_skills(&[Path::new("a.rs")])
        .is_empty());
    let no_cond = DiskSkillRegistry::new(vec![skill("a", SkillSource::Bundled)]);
    assert!(no_cond
        .find_matching_skills(&[Path::new("a.rs")])
        .is_empty());
    let r2 = DiskSkillRegistry::new(vec![
        skill_with_paths("rs", SkillSource::Bundled, vec!["**/*.rs".into()]),
        skill_with_paths("md", SkillSource::Bundled, vec!["**/*.md".into()]),
    ]);
    let m = r2.find_matching_skills(&[Path::new("a.rs")]);
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].manifest.name, "rs");
    assert!(r2.find_matching_skills(&[Path::new("a.ts")]).is_empty());
}
#[test]
fn test_find_matching_skills_priority_dedup_mixed() {
    let r = DiskSkillRegistry::new(vec![
        skill_with_paths("low", SkillSource::Project, vec!["**/*.rs".into()]),
        skill_with_paths("high", SkillSource::Bundled, vec!["**/*.rs".into()]),
        skill_with_paths("mid", SkillSource::Global, vec!["**/*.rs".into()]),
    ]);
    let m = r.find_matching_skills(&[Path::new("a.rs")]);
    let names: Vec<&str> = m.iter().map(|s| s.manifest.name.as_str()).collect();
    assert_eq!(names, ["low", "mid", "high"]);
    // dedup: same name, higher priority wins (Project > Bundled)
    let r2 = DiskSkillRegistry::new(vec![
        skill_with_paths("x", SkillSource::Project, vec!["**/*.rs".into()]),
        skill_with_paths("x", SkillSource::Bundled, vec!["**/*.rs".into()]),
    ]);
    let m2 = r2.find_matching_skills(&[Path::new("a.rs")]);
    assert_eq!(m2.len(), 1);
    assert_eq!(m2[0].source, SkillSource::Project);
    let r3 = DiskSkillRegistry::new(vec![
        skill_with_paths("rs", SkillSource::Bundled, vec!["**/*.rs".into()]),
        skill_with_paths("md", SkillSource::Bundled, vec!["**/*.md".into()]),
        skill_with_paths("bad", SkillSource::Global, vec!["[invalid".into()]),
        skill("plain", SkillSource::Bundled),
    ]);
    let m3 = r3.find_matching_skills(&[Path::new("a.rs"), Path::new("a.md")]);
    let names3: Vec<&str> = m3.iter().map(|s| s.manifest.name.as_str()).collect();
    assert_eq!(names3, ["rs", "md"]);
}

// ---- AgentRegistry query path tests ----
// Tests for DiskSkillRegistry's ability to query AgentRegistry directly
// for skills whitelist configuration, per design-doc query path.

use crate::agent::registry::AgentRegistry;
use crate::config::agents::{ConfigSource, ResolvedAgentConfig};
use crate::session::bootstrap::loader::BootstrapMode;
use std::sync::Arc;

fn make_agent_config(id: &str, skills: Vec<String>) -> ResolvedAgentConfig {
    ResolvedAgentConfig {
        id: id.to_string(),
        name: id.to_string(),
        parent_id: None,
        model: None,
        workspace: None,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills,
        tools: vec![],
        disallowed_tools: vec![],
        subagents: Default::default(),
        memory: None,
        source: ConfigSource::User,
    }
}

#[test]
fn test_set_agent_registry_and_accessor() {
    let mut r = DiskSkillRegistry::new(vec![]);
    assert!(r.agent_registry().is_none());

    let arc_reg = Arc::new(AgentRegistry::new());
    r.set_agent_registry(Arc::clone(&arc_reg));
    assert!(r.agent_registry().is_some());
    // The returned Arc should point to the same registry
    let returned = r.agent_registry().unwrap();
    assert!(Arc::ptr_eq(returned, &arc_reg));
}

#[test]
fn test_generate_listing_for_agent_with_whitelist() {
    // Agent has skills whitelist ["foo", "bar"]
    let agent_reg = Arc::new(AgentRegistry::new());
    agent_reg.populate(vec![make_agent_config(
        "agent-1",
        vec!["foo".into(), "bar".into()],
    )]);

    let mut r = DiskSkillRegistry::new(vec![
        skill("foo", SkillSource::Bundled),
        skill("bar", SkillSource::Bundled),
        skill("baz", SkillSource::Bundled),
    ]);
    r.set_agent_registry(agent_reg);

    let listing = r.generate_listing_for_agent("agent-1");
    assert!(listing.contains("**foo**"));
    assert!(listing.contains("**bar**"));
    assert!(!listing.contains("**baz**"));
}

#[test]
fn test_generate_listing_for_agent_agent_not_found() {
    // Agent not in registry — should show all skills (no whitelist)
    let agent_reg = Arc::new(AgentRegistry::new());
    agent_reg.populate(vec![]);

    let mut r = DiskSkillRegistry::new(vec![
        skill("foo", SkillSource::Bundled),
        skill("bar", SkillSource::Bundled),
    ]);
    r.set_agent_registry(agent_reg);

    let listing = r.generate_listing_for_agent("nonexistent");
    assert!(listing.contains("**foo**"));
    assert!(listing.contains("**bar**"));
}

#[test]
fn test_generate_listing_for_agent_wildcard_whitelist() {
    // Agent skills = ["*"] means show all
    let agent_reg = Arc::new(AgentRegistry::new());
    agent_reg.populate(vec![make_agent_config("agent-wild", vec!["*".into()])]);

    let mut r = DiskSkillRegistry::new(vec![
        skill("foo", SkillSource::Bundled),
        skill("bar", SkillSource::Global),
    ]);
    r.set_agent_registry(agent_reg);

    let listing = r.generate_listing_for_agent("agent-wild");
    assert!(listing.contains("**foo**"));
    assert!(listing.contains("**bar**"));
}

#[test]
fn test_generate_listing_for_agent_empty_skills() {
    // Agent skills = [] means show all (no filtering)
    let agent_reg = Arc::new(AgentRegistry::new());
    agent_reg.populate(vec![make_agent_config("agent-empty", vec![])]);

    let mut r = DiskSkillRegistry::new(vec![
        skill("foo", SkillSource::Bundled),
        skill("bar", SkillSource::Global),
    ]);
    r.set_agent_registry(agent_reg);

    let listing = r.generate_listing_for_agent("agent-empty");
    assert!(listing.contains("**foo**"));
    assert!(listing.contains("**bar**"));
}

#[test]
fn test_generate_listing_for_agent_no_registry_set() {
    // No agent_registry set — should show all skills
    let r = DiskSkillRegistry::new(vec![
        skill("foo", SkillSource::Bundled),
        skill("bar", SkillSource::Bundled),
    ]);
    // Not setting agent_registry

    let listing = r.generate_listing_for_agent("any-agent");
    assert!(listing.contains("**foo**"));
    assert!(listing.contains("**bar**"));
}

#[test]
fn test_generate_listing_fallback_to_agent_registry() {
    // When no explicit whitelist is passed, generate_listing should
    // fall back to AgentRegistry lookup.
    let agent_reg = Arc::new(AgentRegistry::new());
    agent_reg.populate(vec![make_agent_config("agent-fb", vec!["skill-a".into()])]);

    let mut r = DiskSkillRegistry::new(vec![
        skill("skill-a", SkillSource::Bundled),
        skill("skill-b", SkillSource::Bundled),
    ]);
    r.set_agent_registry(agent_reg);

    // Pass None for skills_whitelist — should use AgentRegistry
    let listing = r.generate_listing(Some("agent-fb"), None);
    assert!(listing.contains("**skill-a**"));
    assert!(!listing.contains("**skill-b**"));
}

#[test]
fn test_generate_listing_explicit_whitelist_overrides_registry() {
    // When explicit whitelist is provided, AgentRegistry is not used
    let agent_reg = Arc::new(AgentRegistry::new());
    agent_reg.populate(vec![make_agent_config("agent-ov", vec!["skill-a".into()])]);

    let mut r = DiskSkillRegistry::new(vec![
        skill("skill-a", SkillSource::Bundled),
        skill("skill-b", SkillSource::Bundled),
    ]);
    r.set_agent_registry(agent_reg);

    // Explicit whitelist allows skill-b (not in agent config)
    let listing = r.generate_listing(Some("agent-ov"), Some(&["skill-b".into()]));
    assert!(!listing.contains("**skill-a**"));
    assert!(listing.contains("**skill-b**"));
}

#[test]
fn test_generate_listing_for_agent_user_invocable_filter() {
    // user_invocable: false skills should still be excluded
    let agent_reg = Arc::new(AgentRegistry::new());
    agent_reg.populate(vec![make_agent_config(
        "agent-inv",
        vec!["visible".into(), "hidden".into()],
    )]);

    let visible = skill("visible", SkillSource::Bundled);
    let mut hidden = skill("hidden", SkillSource::Bundled);
    hidden.manifest.user_invocable = false;

    let mut r = DiskSkillRegistry::new(vec![visible, hidden]);
    r.set_agent_registry(agent_reg);

    let listing = r.generate_listing_for_agent("agent-inv");
    assert!(listing.contains("**visible**"));
    assert!(!listing.contains("**hidden**"));
}
