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
        // Re-scan disk skill directories at every SP assembly boundary
        // so the listing reflects the latest on-disk skill files.
        self.listing.rescan();

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
#[path = "skills_tests.rs"]
mod skills_tests;
