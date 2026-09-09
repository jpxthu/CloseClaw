//! Skill listing incremental injection logic.
//!
//! Provides per-turn skill listing computation with incremental diff,
//! conditional skill activation, and file-path-based matching. Used
//! by [`super::ConversationSession::prepare_turn_skill_listing`].
//!
//! Implements the "增量更新" (incremental update) section of the
//! design doc (`docs/design/skills/skill-listing-injection.md`).

use std::collections::{BTreeSet, HashSet};

use super::ConversationSession;
use closeclaw_common::SkillListingProvider;
use std::sync::Arc;

impl ConversationSession {
    /// Compute the skill listing for the current turn without
    /// mutating session state.
    ///
    /// Implements the design doc's conditional activation injection:
    /// when `newly_activated` is non-empty, the complete formatted
    /// entries (with ⚡) for those skills are injected as-is, not as
    /// a diff. The snapshot is still updated via the diff mechanism
    /// to track overall state.
    ///
    /// When `newly_activated` is empty, falls back to the original
    /// incremental diff behavior: computes a line-level diff against
    /// the previous snapshot and injects additions/deletions.
    ///
    /// On the first turn (no snapshot), generates a full listing
    /// regardless of `newly_activated`.
    ///
    /// Returns `(listing_to_inject, new_snapshot)` where
    /// `listing_to_inject` is the content for the system-role attachment
    /// (`None` when nothing to inject) and `new_snapshot` is the
    /// updated snapshot to persist.
    pub(crate) fn compute_skill_listing_for_turn(
        &self,
        newly_activated: &HashSet<String>,
    ) -> (Option<String>, Option<String>) {
        let Some(provider) = self.skill_listing_provider.as_ref() else {
            return (None, None);
        };

        // Generate the current listing. When newly_activated is
        // non-empty, use a temporary combined set so the listing
        // includes the new skills' entries (they haven't been added
        // to activated_conditional_skills yet).
        let combined_activated: HashSet<String> = if newly_activated.is_empty() {
            self.activated_conditional_skills.clone()
        } else {
            self.activated_conditional_skills
                .union(newly_activated)
                .cloned()
                .collect()
        };
        let current_listing = self.generate_listing_with_activated(provider, &combined_activated);
        if current_listing.is_empty() {
            return (None, None);
        }

        match self.skill_listing_snapshot.as_deref() {
            None => {
                // First turn — inject full listing
                (Some(current_listing.clone()), Some(current_listing))
            }
            Some(old_snapshot) => {
                if !newly_activated.is_empty() {
                    // Conditional activation: inject complete entries
                    // for newly activated skills (per design doc: "以
                    // 系统消息形式注入该 skill 的清单条目（含 ⚡ 标记，
                    // 不含正文）").
                    // Use BTreeSet for deterministic iteration order
                    // across turns and platforms.
                    let new_lines: BTreeSet<&str> =
                        current_listing.lines().filter(|l| !l.is_empty()).collect();
                    let entries: Vec<String> = new_lines
                        .iter()
                        .filter(|line| {
                            // Match complete entry lines (e.g.
                            // `- **name**: ...`) to avoid substring
                            // false matches on partial skill names.
                            line.starts_with("- **")
                                && newly_activated
                                    .iter()
                                    .any(|name| line.contains(&format!("**{}**:", name)))
                        })
                        .map(|l| l.to_string())
                        .collect();
                    if entries.is_empty() {
                        // Newly activated skills not found in listing;
                        // fall back to diff.
                        let diff = Self::compute_listing_diff(old_snapshot, &current_listing);
                        if diff.is_empty() {
                            (None, Some(current_listing))
                        } else {
                            (Some(diff), Some(current_listing))
                        }
                    } else {
                        (Some(entries.join("\n")), Some(current_listing))
                    }
                } else {
                    // No newly activated skills: incremental diff.
                    let diff = Self::compute_listing_diff(old_snapshot, &current_listing);
                    if diff.is_empty() {
                        (None, Some(current_listing))
                    } else {
                        (Some(diff), Some(current_listing))
                    }
                }
            }
        }
    }

    /// Compute a line-level diff between old and new listings.
    ///
    /// Returns a diff string with additions and deletions, or an
    /// empty string if there are no changes.
    fn compute_listing_diff(old_snapshot: &str, current_listing: &str) -> String {
        let old_lines: HashSet<&str> = old_snapshot.lines().filter(|l| !l.is_empty()).collect();
        let new_lines: HashSet<&str> = current_listing.lines().filter(|l| !l.is_empty()).collect();
        let additions: Vec<String> = current_listing
            .lines()
            .filter(|l| !l.is_empty() && !old_lines.contains(*l))
            .map(|l| l.to_string())
            .collect();
        let deletions: Vec<String> = old_snapshot
            .lines()
            .filter(|l| !l.is_empty() && !new_lines.contains(*l))
            .map(|l| format!("- {}", l))
            .collect();
        let mut parts = additions;
        parts.extend(deletions);
        parts.join("\n")
    }

    /// Preserve skill listing state across conversation compaction.
    ///
    /// Implements the design doc's description of "对话压缩时受
    /// Session 模块保护" (conversation compaction is protected by the
    /// Session module). See
    /// `docs/design/skills/skill-listing-injection.md`.
    ///
    /// Skill listing state (`skill_listing_snapshot` and
    /// `activated_conditional_skills`) is session-level state that
    /// must survive compaction. During compaction, the transcript is
    /// rewritten via [`super::transcript_ops::apply_transcript_op`],
    /// but these fields are independent of the transcript and are not
    /// affected by that operation. This method explicitly documents
    /// the protection intent and ensures the state remains intact:
    ///
    /// - `skill_listing_snapshot` remains valid for the next turn's
    ///   incremental diff computation in [`compute_skill_listing_for_turn`].
    /// - `activated_conditional_skills` persists so conditionally
    ///   activated skills remain available in subsequent turns.
    ///
    /// Called by the gateway layer after compaction completes, before
    /// the next turn re-computes the listing.
    pub fn preserve_listing_on_compaction(&self) {
        // skill_listing_snapshot and activated_conditional_skills are
        // session-level fields independent of the transcript. They are
        // not cleared by apply_transcript_op (which only replaces
        // self.messages and updates last_activity_at). This method
        // makes the protection explicit per the design doc.
        //
        // Future-proofing: if transcript_ops ever clears additional
        // session state, this method provides a single place to add
        // preservation logic for skill listing fields.
        tracing::debug!(
            session_id = %self.session_id,
            has_snapshot = self.skill_listing_snapshot.is_some(),
            activated_count = self.activated_conditional_skills.len(),
            "preserve_listing_on_compaction: skill listing state retained"
        );
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
