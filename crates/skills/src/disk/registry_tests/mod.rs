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

fn skill_with_when_to_use(name: &str, source: SkillSource, when_to_use: &str) -> DiskSkill {
    DiskSkill {
        source,
        manifest: SkillManifest {
            name: name.into(),
            description: format!("desc of {}", name),
            allowed_tools: vec![],
            when_to_use: when_to_use.into(),
            context: SkillContext::default(),
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
    assert!(listing.find("**a_agent**").unwrap() < listing.find("**a_global**").unwrap());
    assert!(listing.find("**a_global**").unwrap() < listing.find("**a_bundled**").unwrap());
    assert!(listing.find("**a_agent**").unwrap() < listing.find("**z_agent**").unwrap());
}
#[test]
fn test_generate_listing_agent_id_filter() {
    // Agent-scoped filtering is now handled by directory-based discovery
    // (agents/<id>/skills/), not by manifest fields. All skills appear
    // in the listing regardless of the agent_id parameter.
    let r = DiskSkillRegistry::new(vec![
        skill("skill_a", SkillSource::Agent),
        skill("skill_b", SkillSource::Agent),
        skill("skill_c", SkillSource::Agent),
    ]);
    assert_eq!(r.generate_listing(None, None).lines().count(), 3);
    let listing = r.generate_listing(Some("agent1"), None);
    assert!(listing.contains("**skill_a**"));
    assert!(listing.contains("**skill_c**"));
    assert!(listing.contains("**skill_b**"));
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
    // Agent-scoped filtering is now handled by directory-based discovery
    // (agents/<id>/skills/), not by manifest fields. All skills appear
    // in the listing regardless of the agent_id parameter.
    let s1 = skill_with_paths("agent1-cond", SkillSource::Agent, vec!["**/*.rs".into()]);
    let s2 = skill_with_paths("any-cond", SkillSource::Agent, vec!["**/*.md".into()]);
    let s3 = skill_with_paths("agent2-cond", SkillSource::Agent, vec!["**/*.toml".into()]);
    let r = DiskSkillRegistry::new(vec![s1, s2, s3, skill("plain", SkillSource::Agent)]);
    let listing = r.generate_listing(Some("agent1"), None);
    assert!(listing.contains("**agent1-cond**"));
    assert!(listing.contains("**any-cond**"));
    assert!(listing.contains("**agent2-cond**"));
    assert!(listing.contains("**plain**"));
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

// ---- AgentSkillsQuery mock tests ----
// Tests for DiskSkillRegistry's ability to query the AgentSkillsQuery
// trait directly for skills whitelist configuration.

use closeclaw_agent::AgentSkillsQuery;
use std::sync::Arc;

/// Mock agent skills query for testing.
struct MockAgentSkillsQuery {
    configs: std::collections::HashMap<String, Vec<String>>,
}

impl MockAgentSkillsQuery {
    fn new() -> Self {
        Self {
            configs: std::collections::HashMap::new(),
        }
    }

    fn with_config(mut self, agent_id: &str, skills: Vec<String>) -> Self {
        self.configs.insert(agent_id.to_string(), skills);
        self
    }
}

impl AgentSkillsQuery for MockAgentSkillsQuery {
    fn get_agent_skills(&self, agent_id: &str) -> Option<Vec<String>> {
        self.configs.get(agent_id).and_then(|skills| {
            // Match real AgentRegistry behavior: empty or ["*"] means wildcard
            if skills.is_empty() || (skills.len() == 1 && skills[0] == "*") {
                None
            } else {
                Some(skills.clone())
            }
        })
    }
}

#[test]
fn test_set_agent_skills_query_and_accessor() {
    let mut r = DiskSkillRegistry::new(vec![]);
    assert!(r.agent_skills_query().is_none());

    let query: Arc<dyn AgentSkillsQuery> = Arc::new(MockAgentSkillsQuery::new());
    r.set_agent_skills_query(Arc::clone(&query));
    assert!(r.agent_skills_query().is_some());
    let returned = r.agent_skills_query().unwrap();
    assert!(Arc::ptr_eq(returned, &query));
}

#[test]
fn test_generate_listing_for_agent_with_whitelist() {
    let query = Arc::new(
        MockAgentSkillsQuery::new().with_config("agent-1", vec!["foo".into(), "bar".into()]),
    );

    let mut r = DiskSkillRegistry::new(vec![
        skill("foo", SkillSource::Bundled),
        skill("bar", SkillSource::Bundled),
        skill("baz", SkillSource::Bundled),
    ]);
    r.set_agent_skills_query(query);

    let listing = r.generate_listing_for_agent("agent-1");
    assert!(listing.contains("**foo**"));
    assert!(listing.contains("**bar**"));
    assert!(!listing.contains("**baz**"));
}

#[test]
fn test_generate_listing_for_agent_agent_not_found() {
    let query = Arc::new(MockAgentSkillsQuery::new());

    let mut r = DiskSkillRegistry::new(vec![
        skill("foo", SkillSource::Bundled),
        skill("bar", SkillSource::Bundled),
    ]);
    r.set_agent_skills_query(query);

    let listing = r.generate_listing_for_agent("nonexistent");
    assert!(listing.contains("**foo**"));
    assert!(listing.contains("**bar**"));
}

#[test]
fn test_generate_listing_for_agent_wildcard_whitelist() {
    let query = Arc::new(MockAgentSkillsQuery::new().with_config("agent-wild", vec!["*".into()]));

    let mut r = DiskSkillRegistry::new(vec![
        skill("foo", SkillSource::Bundled),
        skill("bar", SkillSource::Global),
    ]);
    r.set_agent_skills_query(query);

    let listing = r.generate_listing_for_agent("agent-wild");
    assert!(listing.contains("**foo**"));
    assert!(listing.contains("**bar**"));
}

#[test]
fn test_generate_listing_for_agent_empty_skills() {
    let query = Arc::new(MockAgentSkillsQuery::new().with_config("agent-empty", vec![]));

    let mut r = DiskSkillRegistry::new(vec![
        skill("foo", SkillSource::Bundled),
        skill("bar", SkillSource::Global),
    ]);
    r.set_agent_skills_query(query);

    let listing = r.generate_listing_for_agent("agent-empty");
    assert!(listing.contains("**foo**"));
    assert!(listing.contains("**bar**"));
}

#[test]
fn test_generate_listing_for_agent_no_query_set() {
    let r = DiskSkillRegistry::new(vec![
        skill("foo", SkillSource::Bundled),
        skill("bar", SkillSource::Bundled),
    ]);

    let listing = r.generate_listing_for_agent("any-agent");
    assert!(listing.contains("**foo**"));
    assert!(listing.contains("**bar**"));
}

#[test]
fn test_generate_listing_fallback_to_agent_skills_query() {
    let query =
        Arc::new(MockAgentSkillsQuery::new().with_config("agent-fb", vec!["skill-a".into()]));

    let mut r = DiskSkillRegistry::new(vec![
        skill("skill-a", SkillSource::Bundled),
        skill("skill-b", SkillSource::Bundled),
    ]);
    r.set_agent_skills_query(query);

    let listing = r.generate_listing(Some("agent-fb"), None);
    assert!(listing.contains("**skill-a**"));
    assert!(!listing.contains("**skill-b**"));
}

#[test]
fn test_generate_listing_explicit_whitelist_overrides_query() {
    let query =
        Arc::new(MockAgentSkillsQuery::new().with_config("agent-ov", vec!["skill-a".into()]));

    let mut r = DiskSkillRegistry::new(vec![
        skill("skill-a", SkillSource::Bundled),
        skill("skill-b", SkillSource::Bundled),
    ]);
    r.set_agent_skills_query(query);

    let listing = r.generate_listing(Some("agent-ov"), Some(&["skill-b".into()]));
    assert!(!listing.contains("**skill-a**"));
    assert!(listing.contains("**skill-b**"));
}

#[test]
fn test_generate_listing_for_agent_user_invocable_filter() {
    let query = Arc::new(
        MockAgentSkillsQuery::new()
            .with_config("agent-inv", vec!["visible".into(), "hidden".into()]),
    );

    let visible = skill("visible", SkillSource::Bundled);
    let mut hidden = skill("hidden", SkillSource::Bundled);
    hidden.manifest.user_invocable = false;

    let mut r = DiskSkillRegistry::new(vec![visible, hidden]);
    r.set_agent_skills_query(query);

    let listing = r.generate_listing_for_agent("agent-inv");
    assert!(listing.contains("**visible**"));
    assert!(!listing.contains("**hidden**"));
}

// ---- Effort rendering tests ----

fn skill_with_effort(name: &str, source: SkillSource, effort: SkillEffort) -> DiskSkill {
    DiskSkill {
        source,
        manifest: SkillManifest {
            name: name.into(),
            description: format!("desc of {}", name),
            allowed_tools: vec![],
            when_to_use: String::new(),
            context: SkillContext::default(),
            effort,
            paths: vec![],
            user_invocable: true,
        },
        readme_path: PathBuf::from(format!("/skills/{}/SKILL.md", name)),
        skill_dir: PathBuf::from(format!("/skills/{}", name)),
        body: String::new(),
    }
}

#[test]
fn test_skill_effort_display_trivial() {
    assert_eq!(SkillEffort::Trivial.to_string(), "trivial");
}

#[test]
fn test_skill_effort_display_small() {
    assert_eq!(SkillEffort::Small.to_string(), "small");
}

#[test]
fn test_skill_effort_display_medium() {
    assert_eq!(SkillEffort::Medium.to_string(), "medium");
}

#[test]
fn test_skill_effort_display_large() {
    assert_eq!(SkillEffort::Large.to_string(), "large");
}

#[test]
fn test_skill_effort_display_unknown() {
    assert_eq!(SkillEffort::Unknown.to_string(), "unknown");
}

#[test]
fn test_render_listing_effort_medium() {
    let r = DiskSkillRegistry::new(vec![skill_with_effort(
        "my-skill",
        SkillSource::Bundled,
        SkillEffort::Medium,
    )]);
    let listing = r.generate_listing(None, None);
    assert!(listing.contains("[effort: medium]"));
}

#[test]
fn test_render_listing_effort_trivial() {
    let r = DiskSkillRegistry::new(vec![skill_with_effort(
        "tiny",
        SkillSource::Bundled,
        SkillEffort::Trivial,
    )]);
    let listing = r.generate_listing(None, None);
    assert!(listing.contains("[effort: trivial]"));
}

#[test]
fn test_render_listing_effort_small() {
    let r = DiskSkillRegistry::new(vec![skill_with_effort(
        "small-task",
        SkillSource::Bundled,
        SkillEffort::Small,
    )]);
    let listing = r.generate_listing(None, None);
    assert!(listing.contains("[effort: small]"));
}

#[test]
fn test_render_listing_effort_large() {
    let r = DiskSkillRegistry::new(vec![skill_with_effort(
        "big-task",
        SkillSource::Bundled,
        SkillEffort::Large,
    )]);
    let listing = r.generate_listing(None, None);
    assert!(listing.contains("[effort: large]"));
}

#[test]
fn test_render_listing_effort_unknown_not_rendered() {
    let r = DiskSkillRegistry::new(vec![skill_with_effort(
        "unknown-effort",
        SkillSource::Bundled,
        SkillEffort::Unknown,
    )]);
    let listing = r.generate_listing(None, None);
    assert!(!listing.contains("[effort:"));
    assert!(!listing.contains("[effort: unknown]"));
}

#[test]
fn test_render_listing_no_effort_field_not_rendered() {
    // Default effort is Unknown, should not render
    let r = DiskSkillRegistry::new(vec![skill("no-effort", SkillSource::Bundled)]);
    let listing = r.generate_listing(None, None);
    assert!(!listing.contains("[effort:"));
}

#[test]
fn test_render_listing_effort_with_when_to_use() {
    let mut s = skill_with_effort("combo", SkillSource::Bundled, SkillEffort::Medium);
    s.manifest.when_to_use = "Use when combo needed".into();
    let r = DiskSkillRegistry::new(vec![s]);
    let listing = r.generate_listing(None, None);
    assert!(listing.contains("— Use when combo needed [effort: medium]"));
}

#[test]
fn test_render_listing_effort_with_paths() {
    let mut s = skill_with_effort("cond-effort", SkillSource::Bundled, SkillEffort::Large);
    s.manifest.paths = vec!["**/*.rs".into()];
    let r = DiskSkillRegistry::new(vec![s]);
    let listing = r.generate_listing(None, None);
    assert!(listing.contains("⚡ auto-activates on: **/*.rs [effort: large]"));
}

#[test]
fn test_render_listing_effort_mixed_skills() {
    let r = DiskSkillRegistry::new(vec![
        skill_with_effort("a", SkillSource::Bundled, SkillEffort::Medium),
        skill_with_effort("b", SkillSource::Bundled, SkillEffort::Unknown),
        skill_with_effort("c", SkillSource::Bundled, SkillEffort::Small),
    ]);
    let listing = r.generate_listing(None, None);
    let lines: Vec<&str> = listing.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("[effort: medium]"));
    assert!(!lines[1].contains("[effort:"));
    assert!(lines[2].contains("[effort: small]"));
}
