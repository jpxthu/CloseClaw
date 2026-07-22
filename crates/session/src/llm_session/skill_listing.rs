//! Skill listing incremental injection logic.
//!
//! Provides per-turn skill listing computation with incremental diff,
//! conditional skill activation, and file-path-based matching. Used
//! by [`super::ConversationSession::prepare_turn_skill_listing`].

use std::collections::HashSet;

use super::ConversationSession;
use closeclaw_common::SkillListingProvider;
use std::sync::Arc;

impl ConversationSession {
    /// Compute the skill listing for the current turn without
    /// mutating session state.
    ///
    /// On the first turn (no snapshot), generates a full listing
    /// excluding conditional skills. On subsequent turns, generates the
    /// current listing (including activated conditional skills) and
    /// computes a line-level diff against the previous snapshot.
    ///
    /// Uses the current `activated_conditional_skills` set.
    ///
    /// Returns `(listing_to_inject, new_snapshot)` where
    /// `listing_to_inject` is the content for the tool-role attachment
    /// (`None` when nothing to inject) and `new_snapshot` is the
    /// updated snapshot to persist.
    pub(crate) fn compute_skill_listing_for_turn(&self) -> (Option<String>, Option<String>) {
        let Some(provider) = self.skill_listing_provider.as_ref() else {
            return (None, None);
        };

        // Generate the current listing (including activated
        // conditional skills).
        let current_listing =
            self.generate_listing_with_activated(provider, &self.activated_conditional_skills);
        if current_listing.is_empty() {
            return (None, None);
        }

        // Compute incremental diff against snapshot.
        match self.skill_listing_snapshot.as_deref() {
            None => {
                // First turn — inject full listing
                (Some(current_listing.clone()), Some(current_listing))
            }
            Some(old_snapshot) => {
                let old_lines: HashSet<&str> =
                    old_snapshot.lines().filter(|l| !l.is_empty()).collect();
                let new_lines: Vec<&str> =
                    current_listing.lines().filter(|l| !l.is_empty()).collect();
                let additions: Vec<String> = new_lines
                    .iter()
                    .filter(|l| !old_lines.contains(*l))
                    .map(|l| l.to_string())
                    .collect();
                // Update snapshot to reflect current state
                let new_snapshot = current_listing;
                if additions.is_empty() {
                    (None, Some(new_snapshot))
                } else {
                    (Some(additions.join("\n")), Some(new_snapshot))
                }
            }
        }
    }

    /// Apply the skill listing state update after a turn.
    ///
    /// Updates the snapshot and activated conditional skills set.
    /// Called by [`super::session_llm::ConversationSession::invoke_llm`]
    /// after [`compute_skill_listing_for_turn`].
    pub(crate) fn apply_skill_listing_update(
        &mut self,
        new_snapshot: Option<String>,
        newly_activated: &HashSet<String>,
    ) {
        if let Some(snapshot) = new_snapshot {
            self.skill_listing_snapshot = Some(snapshot);
        }
        self.activated_conditional_skills
            .extend(newly_activated.iter().cloned());
    }

    /// Generate the skill listing for the current turn.
    ///
    /// Combines the base listing (excluding conditional skills) with
    /// the activated conditional skills' listing lines.
    fn generate_listing_with_activated(
        &self,
        provider: &Arc<dyn SkillListingProvider>,
        activated: &HashSet<String>,
    ) -> String {
        let base =
            provider.generate_listing_excluding_conditional(None, self.agent_skills.as_deref());
        if activated.is_empty() {
            return base;
        }
        // Generate a listing including ALL skills (conditional +
        // non-conditional), then filter to only base + activated
        // conditional lines.
        let all_listing = provider.generate_listing(None, self.agent_skills.as_deref());
        if all_listing.is_empty() {
            return base;
        }
        let base_set: HashSet<&str> = base.lines().filter(|l| !l.is_empty()).collect();
        let filtered: Vec<&str> = all_listing
            .lines()
            .filter(|l| {
                if l.is_empty() {
                    return false;
                }
                if base_set.contains(l) {
                    return true;
                }
                // Check if this line is for an activated
                // conditional skill
                activated
                    .iter()
                    .any(|name| l.contains(&format!("**{}**", name)))
            })
            .collect();
        filtered.join("\n")
    }

    /// Extract file paths from user message content.
    ///
    /// Looks for path-like patterns (strings containing `/` with a
    /// filename component) to identify potential file paths for
    /// conditional skill matching.
    pub(crate) fn extract_file_paths(content: &str) -> Vec<std::path::PathBuf> {
        use std::path::PathBuf;
        content
            .split_whitespace()
            .filter_map(|token| {
                // Must contain at least one `/` and end with a
                // word-like component (not just punctuation)
                if !token.contains('/') {
                    return None;
                }
                // Strip surrounding punctuation
                let cleaned = token.trim_matches(|c: char| {
                    c == '"'
                        || c == '\''
                        || c == '('
                        || c == ')'
                        || c == '['
                        || c == ']'
                        || c == '<'
                        || c == '>'
                        || c == ','
                        || c == ';'
                });
                if cleaned.is_empty() || cleaned.len() < 3 {
                    return None;
                }
                // Must have at least one non-slash char after the
                // last slash (i.e. a filename component)
                let after_last_slash = cleaned.rsplit('/').next()?;
                if after_last_slash.is_empty()
                    || !after_last_slash.chars().any(|c| c.is_alphanumeric())
                {
                    return None;
                }
                Some(PathBuf::from(cleaned))
            })
            .collect()
    }
}
