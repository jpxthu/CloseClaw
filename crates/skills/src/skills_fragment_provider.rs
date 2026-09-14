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

/// Maximum length of the skills section (in bytes).
///
/// Skills are truncated atomically at entry boundaries when the listing
/// exceeds this byte limit, mirroring `TOOLS_SECTION_MAX_LEN` for tools.
/// Value is intentionally lower than the tools limit to respect the
/// compression priority: tools first, then skills.
///
/// Limit is in **bytes** (UTF-8 length), not characters, to avoid
/// per-character counting overhead.
pub(crate) const SKILLS_SECTION_MAX_LEN: usize = 4000;

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
        // Re-scan disk skill directories and generate the listing in a
        // single blocking task. Both rescan() (sync disk I/O) and
        // generate_listing_with_activated() (sync under an async-lock)
        // must not run on async-worker threads; spawn_blocking isolates
        // them on a dedicated blocking pool thread where a runtime context
        // exists, avoiding the "Cannot start a runtime from within a
        // runtime" panic.
        let listing = Arc::clone(&self.listing);
        let agent_id = ctx.agent_id.clone();
        let activated_skills = ctx.activated_skills.clone();
        let content = tokio::task::spawn_blocking(move || {
            listing.rescan();
            listing.generate_listing_with_activated(Some(&agent_id), None, &activated_skills)
        })
        .await
        .ok()?;

        if content.is_empty() {
            return None;
        }

        let content = truncate_listing(&content, SKILLS_SECTION_MAX_LEN);

        Some(PromptFragment {
            section_title: "## Skills".to_string(),
            section_type: SectionType::Skills,
            content,
        })
    }

    async fn cache_key(&self, ctx: &FragmentContext) -> Option<String> {
        // Include activated skills fingerprint so different activation
        // states produce distinct cache entries.
        let mut sorted_activated = ctx.activated_skills.clone();
        sorted_activated.sort();
        let listing = Arc::clone(&self.listing);
        let fingerprint = tokio::task::spawn_blocking(move || listing.fingerprint())
            .await
            .ok()?;
        Some(format!(
            "skill_listing:{}:{}:{}",
            ctx.agent_id,
            sorted_activated.join(","),
            fingerprint
        ))
    }
}

/// Truncate a skill listing to fit within `max_len` bytes (UTF-8),
/// preserving whole skill entries (one entry per line).
///
/// At least one entry is always kept, even if it exceeds the limit.
/// No truncation hint text is appended (matches ToolsSection behavior).
pub(crate) fn truncate_listing(listing: &str, max_len: usize) -> String {
    let total_len = listing.len();
    if total_len <= max_len {
        return listing.to_string();
    }

    let lines: Vec<&str> = listing.lines().collect();
    if lines.is_empty() {
        return listing.to_string();
    }

    let mut kept: Vec<&str> = Vec::new();
    let mut running_len: usize = 0;

    for line in lines.iter() {
        let line_len = line.len();
        let new_len = if kept.is_empty() {
            line_len
        } else {
            running_len + 1 + line_len // +1 for the \n separator
        };

        if new_len > max_len && !kept.is_empty() {
            break;
        }

        // Always keep at least 1 entry, even if it exceeds the limit.
        if new_len > max_len && kept.is_empty() {
            kept.push(line);
            break;
        }

        kept.push(line);
        running_len = new_len;
    }

    kept.join("\n")
}

#[cfg(test)]
#[path = "skills_fragment_provider_tests.rs"]
mod tests;
