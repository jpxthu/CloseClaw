//! Tests for AgentSkillsQuery integration with DiskSkillRegistry.

use super::super::super::types::SkillSource;
use super::super::super::DiskSkillRegistry;
use super::{skill, MockAgentSkillsQuery};
use closeclaw_common::AgentSkillsQuery;
use std::sync::Arc;

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
