//! Post-refactoring behavior equivalence tests.
//!
//! These tests verify that the unified `exclude_conditional` parameter
//! (introduced in Step 1.4 to eliminate `_all` duplicate methods)
//! produces identical results to the original separate code paths.

use super::{skill, skill_with_paths, MockAgentSkillsQuery};
use crate::disk::types::SkillSource;
use crate::disk::DiskSkillRegistry;
use std::sync::Arc;

#[test]
fn test_generate_listing_for_agent_matches_generate_listing_with_query() {
    // After refactoring, generate_listing_for_agent() and
    // generate_listing(Some(agent_id), None) should produce identical
    // output when both resolve the whitelist from the same agent query.
    let query = Arc::new(
        MockAgentSkillsQuery::new()
            .with_config("agent-eq", vec!["a".into(), "b".into(), "c".into()]),
    );
    let mut r = DiskSkillRegistry::new(vec![
        skill("a", SkillSource::Bundled),
        skill("b", SkillSource::Global),
        skill("c", SkillSource::Agent),
        skill("d", SkillSource::Bundled),
    ]);
    r.set_agent_skills_query(query);

    let via_for_agent = r.generate_listing_for_agent("agent-eq");
    let via_generate = r.generate_listing(Some("agent-eq"), None);
    assert_eq!(
        via_for_agent, via_generate,
        "generate_listing_for_agent and generate_listing(Some(id), None) must be equivalent"
    );
}

#[test]
fn test_generate_listing_for_agent_matches_generate_listing_with_conditional() {
    // Verify equivalence even when conditional skills are present.
    let query = Arc::new(
        MockAgentSkillsQuery::new().with_config("agent-eq2", vec!["plain".into(), "cond".into()]),
    );
    let mut r = DiskSkillRegistry::new(vec![
        skill("plain", SkillSource::Bundled),
        skill_with_paths("cond", SkillSource::Global, vec!["**/*.rs".into()]),
        skill("extra", SkillSource::Bundled),
    ]);
    r.set_agent_skills_query(query);

    let via_for_agent = r.generate_listing_for_agent("agent-eq2");
    let via_generate = r.generate_listing(Some("agent-eq2"), None);
    assert_eq!(
        via_for_agent, via_generate,
        "generate_listing_for_agent and generate_listing(Some(id), None) must be equivalent"
    );
    assert!(via_for_agent.contains("**plain**"));
    assert!(via_for_agent.contains("**cond**"));
    assert!(!via_for_agent.contains("**extra**"));
}

#[test]
fn test_generate_listing_no_conditional_equals_excluding() {
    // When no conditional skills exist, generate_listing() and
    // generate_listing_excluding_conditional() must return identical results.
    let r = DiskSkillRegistry::new(vec![
        skill("a", SkillSource::Bundled),
        skill("b", SkillSource::Global),
        skill("c", SkillSource::Agent),
    ]);
    let all = r.generate_listing(None, None);
    let excl = r.generate_listing_excluding_conditional(None, None);
    assert_eq!(
        all, excl,
        "when no conditional skills exist, both methods must return the same listing"
    );
}

#[test]
fn test_generate_listing_no_conditional_equals_excluding_with_whitelist() {
    // Same as above but with an explicit whitelist.
    let r = DiskSkillRegistry::new(vec![
        skill("x", SkillSource::Bundled),
        skill("y", SkillSource::Global),
    ]);
    let all = r.generate_listing(None, Some(&["x".into()]));
    let excl = r.generate_listing_excluding_conditional(None, Some(&["x".into()]));
    assert_eq!(
        all, excl,
        "when no conditional skills exist, whitelist filtering must be consistent"
    );
}
