//! Tests for SkillsFragmentProvider.

use super::*;
use std::sync::Arc;

/// Minimal mock for `SkillListingProvider`.
struct MockListingProvider {
    output: String,
}

impl SkillListingProvider for MockListingProvider {
    fn generate_listing(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        self.output.clone()
    }

    fn generate_listing_excluding_conditional(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        self.output.clone()
    }

    fn find_conditional_matches(
        &self,
        _paths: &[std::path::PathBuf],
    ) -> Vec<closeclaw_common::ConditionalSkillMatch> {
        vec![]
    }
}

#[test]
fn test_name_and_priority() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: String::new(),
    }));
    assert_eq!(provider.name(), "skills");
    assert_eq!(provider.priority(), 3);
}

#[test]
fn test_cache_key() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: String::new(),
    }));
    let ctx = FragmentContext::test_default();
    assert_eq!(provider.cache_key(&ctx).unwrap(), "skill_listing");
}

#[tokio::test]
async fn test_generate_with_listing() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: "- **foo**: A skill\n- **bar**: Another skill".to_string(),
    }));
    let ctx = FragmentContext::test_default();
    let fragment = provider.generate(&ctx).await;
    let frag = fragment.expect("expected a fragment");
    assert_eq!(frag.section_title, "## Skills");
    assert_eq!(frag.section_type, SectionType::Skills);
    assert!(frag.content.contains("foo"));
    assert!(frag.content.contains("bar"));
}

#[tokio::test]
async fn test_generate_empty_returns_none() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: String::new(),
    }));
    let ctx = FragmentContext::test_default();
    assert!(provider.generate(&ctx).await.is_none());
}
