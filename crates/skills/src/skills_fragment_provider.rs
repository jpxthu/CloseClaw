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

        let content = self.listing.generate_listing_with_activated(
            Some(&ctx.agent_id),
            None,
            &ctx.activated_skills,
        );

        if content.is_empty() {
            return None;
        }

        Some(PromptFragment {
            section_title: "## Skills".to_string(),
            section_type: SectionType::Skills,
            content,
        })
    }

    fn cache_key(&self, ctx: &FragmentContext) -> Option<String> {
        // Include activated skills fingerprint so different activation
        // states produce distinct cache entries.
        let mut sorted_activated = ctx.activated_skills.clone();
        sorted_activated.sort();
        let fingerprint = self.listing.fingerprint();
        Some(format!(
            "skill_listing:{}:{}:{}",
            ctx.agent_id,
            sorted_activated.join(","),
            fingerprint
        ))
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
    fn test_cache_key_includes_agent_id() {
        let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
            output: String::new(),
            rescan_called: Arc::new(AtomicBool::new(false)),
        }));
        let mut ctx = FragmentContext::test_default();
        ctx.agent_id = "agent-xyz".to_string();
        // Empty activated skills → trailing colon + empty string + fingerprint
        let key = provider.cache_key(&ctx).unwrap();
        assert!(
            key.starts_with("skill_listing:agent-xyz:"),
            "key should start with agent prefix, got: {key}"
        );
    }

    #[test]
    fn test_cache_key_varies_with_agent_id() {
        let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
            output: String::new(),
            rescan_called: Arc::new(AtomicBool::new(false)),
        }));

        let mut ctx_a = FragmentContext::test_default();
        ctx_a.agent_id = "agent-a".to_string();
        let mut ctx_b = FragmentContext::test_default();
        ctx_b.agent_id = "agent-b".to_string();

        assert_ne!(provider.cache_key(&ctx_a), provider.cache_key(&ctx_b));
    }

    #[test]
    fn test_cache_key_varies_with_activated_skills() {
        let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
            output: String::new(),
            rescan_called: Arc::new(AtomicBool::new(false)),
        }));

        let mut ctx_empty = FragmentContext::test_default();
        ctx_empty.agent_id = "agent-1".to_string();

        let mut ctx_activated = FragmentContext::test_default();
        ctx_activated.agent_id = "agent-1".to_string();
        ctx_activated.activated_skills = vec!["skill-a".to_string(), "skill-b".to_string()];

        assert_ne!(
            provider.cache_key(&ctx_empty),
            provider.cache_key(&ctx_activated),
            "different activation sets must produce different cache keys"
        );
    }

    #[test]
    fn test_cache_key_sorts_activated_skills() {
        let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
            output: String::new(),
            rescan_called: Arc::new(AtomicBool::new(false)),
        }));

        let mut ctx_a = FragmentContext::test_default();
        ctx_a.agent_id = "agent-1".to_string();
        ctx_a.activated_skills = vec!["b".to_string(), "a".to_string()];

        let mut ctx_b = FragmentContext::test_default();
        ctx_b.agent_id = "agent-1".to_string();
        ctx_b.activated_skills = vec!["a".to_string(), "b".to_string()];

        assert_eq!(
            provider.cache_key(&ctx_a),
            provider.cache_key(&ctx_b),
            "same activation set in different order must produce same cache key"
        );
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

    /// A mock that returns different output based on the activated set,
    /// simulating real generate_listing_with_activated behavior.
    struct ActivatedMockProvider {
        base_listing: String,
        activated_listing: String,
    }

    impl SkillListingProvider for ActivatedMockProvider {
        fn rescan(&self) {}

        fn generate_listing(
            &self,
            _agent_id: Option<&str>,
            _agent_skills: Option<&[String]>,
        ) -> String {
            self.activated_listing.clone()
        }

        fn generate_listing_excluding_conditional(
            &self,
            _agent_id: Option<&str>,
            _agent_skills: Option<&[String]>,
        ) -> String {
            self.base_listing.clone()
        }

        fn generate_listing_with_activated(
            &self,
            _agent_id: Option<&str>,
            _agent_skills: Option<&[String]>,
            activated: &[String],
        ) -> String {
            if activated.is_empty() {
                self.base_listing.clone()
            } else {
                self.activated_listing.clone()
            }
        }

        fn find_conditional_matches(
            &self,
            _paths: &[std::path::PathBuf],
        ) -> Vec<closeclaw_common::ConditionalSkillMatch> {
            vec![]
        }
    }

    // ------------------------------------------------------------------
    // Dimension: Normal path — activated conditional skills in output
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_generate_includes_activated_conditional_skills() {
        let mock = ActivatedMockProvider {
            base_listing: "- **base_skill**: A base skill".to_string(),
            activated_listing:
                "- **base_skill**: A base skill\n- **cond_skill**: ⚡ A conditional skill"
                    .to_string(),
        };
        let provider = SkillsFragmentProvider::new(Arc::new(mock));
        let mut ctx = FragmentContext::test_default();
        ctx.activated_skills = vec!["cond_skill".to_string()];

        let frag = provider.generate(&ctx).await.expect("expected fragment");
        assert!(
            frag.content.contains("cond_skill"),
            "activated conditional skill should appear"
        );
        assert!(
            frag.content.contains("base_skill"),
            "base skill should still appear"
        );
    }

    #[tokio::test]
    async fn test_generate_excludes_unactivated_conditional_skills() {
        // Mock returns only base when no activated conditional skills match.
        let mock = ActivatedMockProvider {
            base_listing: "- **base_skill**: A base skill".to_string(),
            activated_listing: "- **base_skill**: A base skill".to_string(),
        };
        let provider = SkillsFragmentProvider::new(Arc::new(mock));
        let mut ctx = FragmentContext::test_default();
        ctx.activated_skills = vec!["nonexistent_skill".to_string()];

        let frag = provider.generate(&ctx).await.expect("expected fragment");
        assert!(
            frag.content.contains("base_skill"),
            "base skill should appear"
        );
        assert!(
            !frag.content.contains("nonexistent_skill"),
            "nonexistent skill must not appear"
        );
    }

    // ------------------------------------------------------------------
    // Dimension: Empty activation set — regresses to base listing
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_generate_empty_activated_set_matches_excluding_conditional() {
        let base = "- **alpha**: desc alpha\n- **beta**: desc beta".to_string();
        let mock = ActivatedMockProvider {
            base_listing: base.clone(),
            activated_listing: base.clone(),
        };
        let provider = SkillsFragmentProvider::new(Arc::new(mock));
        let ctx = FragmentContext::test_default(); // activated_skills is empty

        let frag = provider.generate(&ctx).await.expect("expected fragment");
        assert_eq!(
            frag.content, base,
            "empty activated set must produce base listing"
        );
    }

    #[tokio::test]
    async fn test_generate_empty_activated_set_no_conditional_leak() {
        let mock = ActivatedMockProvider {
            base_listing: "- **plain**: A plain skill".to_string(),
            activated_listing: "- **plain**: A plain skill\n- **cond**: ⚡ conditional".to_string(),
        };
        let provider = SkillsFragmentProvider::new(Arc::new(mock));
        let ctx = FragmentContext::test_default();

        let frag = provider.generate(&ctx).await.expect("expected fragment");
        assert!(
            !frag.content.contains("cond"),
            "conditional skill must not leak with empty activated set"
        );
    }

    // ------------------------------------------------------------------
    // Dimension: Error/boundary — nonexistent skill in activated set
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_generate_nonexistent_activated_skill_no_panic() {
        let mock = ActivatedMockProvider {
            base_listing: "- **real_skill**: exists".to_string(),
            activated_listing: "- **real_skill**: exists".to_string(),
        };
        let provider = SkillsFragmentProvider::new(Arc::new(mock));
        let mut ctx = FragmentContext::test_default();
        ctx.activated_skills = vec!["deleted_skill".to_string()];

        // Should not panic; provider passes activated set to listing which silently ignores unknowns.
        let result = provider.generate(&ctx).await;
        assert!(
            result.is_some(),
            "should still produce fragment for base skills"
        );
    }

    // ------------------------------------------------------------------
    // Dimension: Cache key includes activation fingerprint
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_cache_key_distinct_for_different_activated_sets() {
        let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
            output: String::new(),
            rescan_called: Arc::new(AtomicBool::new(false)),
        }));

        let mut ctx_empty = FragmentContext::test_default();
        ctx_empty.agent_id = "agent-1".to_string();

        let mut ctx_a = FragmentContext::test_default();
        ctx_a.agent_id = "agent-1".to_string();
        ctx_a.activated_skills = vec!["skill-x".to_string()];

        let mut ctx_b = FragmentContext::test_default();
        ctx_b.agent_id = "agent-1".to_string();
        ctx_b.activated_skills = vec!["skill-x".to_string(), "skill-y".to_string()];

        assert_ne!(
            provider.cache_key(&ctx_empty),
            provider.cache_key(&ctx_a),
            "empty vs single activation must differ"
        );
        assert_ne!(
            provider.cache_key(&ctx_a),
            provider.cache_key(&ctx_b),
            "different activation sets must differ"
        );
    }

    #[tokio::test]
    async fn test_cache_key_order_independent() {
        let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
            output: String::new(),
            rescan_called: Arc::new(AtomicBool::new(false)),
        }));

        let mut ctx_1 = FragmentContext::test_default();
        ctx_1.agent_id = "agent-1".to_string();
        ctx_1.activated_skills = vec!["b".to_string(), "a".to_string()];

        let mut ctx_2 = FragmentContext::test_default();
        ctx_2.agent_id = "agent-1".to_string();
        ctx_2.activated_skills = vec!["a".to_string(), "b".to_string()];

        assert_eq!(
            provider.cache_key(&ctx_1),
            provider.cache_key(&ctx_2),
            "same set in different order must produce same cache key"
        );
    }

    // ------------------------------------------------------------------
    // Dimension: Fragment section metadata is correct
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_generate_activated_fragment_has_skills_section_type() {
        let mock = ActivatedMockProvider {
            base_listing: "- **skill1**: desc".to_string(),
            activated_listing: "- **skill1**: desc\n- **cond1**: ⚡ cond".to_string(),
        };
        let provider = SkillsFragmentProvider::new(Arc::new(mock));
        let mut ctx = FragmentContext::test_default();
        ctx.activated_skills = vec!["cond1".to_string()];

        let frag = provider.generate(&ctx).await.expect("expected fragment");
        assert_eq!(frag.section_title, "## Skills");
        assert_eq!(frag.section_type, SectionType::Skills);
    }

    // ------------------------------------------------------------------
    // Dimension: Fingerprint — default mock returns "0"
    // ------------------------------------------------------------------

    #[test]
    fn test_mock_default_fingerprint_is_zero() {
        let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
            output: String::new(),
            rescan_called: Arc::new(AtomicBool::new(false)),
        }));
        assert_eq!(provider.listing.fingerprint(), "0");
    }

    // ------------------------------------------------------------------
    // Dimension: Cache key includes fingerprint
    // ------------------------------------------------------------------

    #[test]
    fn test_cache_key_includes_fingerprint() {
        let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
            output: String::new(),
            rescan_called: Arc::new(AtomicBool::new(false)),
        }));
        let mut ctx = FragmentContext::test_default();
        ctx.agent_id = "agent-1".to_string();
        let key = provider.cache_key(&ctx).unwrap();
        assert!(
            key.ends_with(":0"),
            "cache key should end with fingerprint ':0', got: {key}"
        );
    }

    // ------------------------------------------------------------------
    // Dimension: Cache key varies with fingerprint
    // ------------------------------------------------------------------

    /// Mock that returns a configurable fingerprint.
    struct FingerprintMockProvider {
        output: String,
        fp: String,
    }

    impl SkillListingProvider for FingerprintMockProvider {
        fn rescan(&self) {}
        fn fingerprint(&self) -> String {
            self.fp.clone()
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
    fn test_cache_key_varies_with_fingerprint() {
        let provider_a = SkillsFragmentProvider::new(Arc::new(FingerprintMockProvider {
            output: String::new(),
            fp: "fp_a".to_string(),
        }));
        let provider_b = SkillsFragmentProvider::new(Arc::new(FingerprintMockProvider {
            output: String::new(),
            fp: "fp_b".to_string(),
        }));
        let mut ctx = FragmentContext::test_default();
        ctx.agent_id = "agent-1".to_string();
        assert_ne!(
            provider_a.cache_key(&ctx),
            provider_b.cache_key(&ctx),
            "different fingerprints must produce different cache keys"
        );
    }

    #[test]
    fn test_cache_key_same_fingerprint_same_key() {
        let provider_a = SkillsFragmentProvider::new(Arc::new(FingerprintMockProvider {
            output: String::new(),
            fp: "same_fp".to_string(),
        }));
        let provider_b = SkillsFragmentProvider::new(Arc::new(FingerprintMockProvider {
            output: String::new(),
            fp: "same_fp".to_string(),
        }));
        let mut ctx = FragmentContext::test_default();
        ctx.agent_id = "agent-1".to_string();
        assert_eq!(
            provider_a.cache_key(&ctx),
            provider_b.cache_key(&ctx),
            "same fingerprints must produce same cache keys"
        );
    }
}
