//! Provider for the Skills section of the system prompt.
//!
//! Delegates to [`SkillListingProvider`] (defined in `closeclaw_common`)
//! to produce a formatted listing of available skills.

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::SkillListingProvider;

use crate::fragment::{FragmentContext, PromptFragment, PromptFragmentProvider, SectionType};

/// Provider that contributes the skill listing to the system prompt.
///
/// Holds an [`Arc<dyn SkillListingProvider>`] and delegates to
/// [`SkillListingProvider::generate_listing_excluding_conditional`]
/// for the actual text generation.
pub struct SkillsFragmentProvider {
    /// Backing skill listing provider.
    listing: Arc<dyn SkillListingProvider>,
}

impl SkillsFragmentProvider {
    /// Create a new skills fragment provider.
    pub fn new(listing: Arc<dyn SkillListingProvider>) -> Self {
        Self { listing }
    }
}

#[async_trait]
impl PromptFragmentProvider for SkillsFragmentProvider {
    fn name(&self) -> &str {
        "skills"
    }

    fn priority(&self) -> u32 {
        3
    }

    async fn generate(&self, ctx: &FragmentContext) -> Option<PromptFragment> {
        let content = self
            .listing
            .generate_listing_excluding_conditional(Some(&ctx.agent_id), None);

        if content.is_empty() {
            return None;
        }

        Some(PromptFragment {
            section_title: "## Skills".to_string(),
            section_type: SectionType::Skills,
            content,
        })
    }

    fn cache_key(&self, _ctx: &FragmentContext) -> Option<String> {
        Some("skill_listing".to_string())
    }
}

#[cfg(test)]
mod tests {
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
}
