//! Provider for the Skills section of the system prompt.
//!
//! Delegates to [`SkillListingProvider`] (defined in `closeclaw_common`)
//! to produce a formatted listing of available skills.

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::fragment::{
    FragmentContext, PromptFragment, PromptFragmentProvider, SectionType,
};
use closeclaw_common::skill_listing_provider::SkillListingProvider;

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
        // Re-scan disk skill directories at every SP assembly boundary
        // so the listing reflects the latest on-disk skill files.
        // Spawn on a blocking thread to avoid blocking the async runtime
        // with synchronous disk I/O.
        let listing = Arc::clone(&self.listing);
        tokio::task::spawn_blocking(move || listing.rescan())
            .await
            .ok();

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
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Minimal mock for `SkillListingProvider`.
    struct MockListingProvider {
        output: String,
        rescan_called: Arc<AtomicBool>,
    }

    impl SkillListingProvider for MockListingProvider {
        fn rescan(&self) {
            self.rescan_called.store(true, Ordering::SeqCst);
        }

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
            rescan_called: Arc::new(AtomicBool::new(false)),
        }));
        assert_eq!(provider.name(), "skills");
        assert_eq!(provider.priority(), 3);
    }

    #[test]
    fn test_cache_key() {
        let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
            output: String::new(),
            rescan_called: Arc::new(AtomicBool::new(false)),
        }));
        let ctx = FragmentContext::test_default();
        assert_eq!(provider.cache_key(&ctx).unwrap(), "skill_listing");
    }

    #[tokio::test]
    async fn test_generate_with_listing() {
        let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
            output: "- **foo**: A skill\n- **bar**: Another skill".to_string(),
            rescan_called: Arc::new(AtomicBool::new(false)),
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
            rescan_called: Arc::new(AtomicBool::new(false)),
        }));
        let ctx = FragmentContext::test_default();
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[tokio::test]
    async fn test_generate_triggers_rescan() {
        let rescan_flag = Arc::new(AtomicBool::new(false));
        let mock = MockListingProvider {
            output: "- **test_skill**: A skill".to_string(),
            rescan_called: rescan_flag.clone(),
        };
        let provider = SkillsFragmentProvider::new(Arc::new(mock));
        let ctx = FragmentContext::test_default();

        let fragment = provider.generate(&ctx).await;
        assert!(fragment.is_some(), "expected a fragment");

        // Verify rescan was called during generate()
        assert!(
            rescan_flag.load(Ordering::SeqCst),
            "rescan() should have been called during generate()"
        );
    }
}
