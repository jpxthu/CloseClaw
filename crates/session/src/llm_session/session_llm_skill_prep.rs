//! Per-turn skill listing preparation for LLM invocation.
//!
//! Provides [`prepare_turn_skill_listing`](ConversationSession::prepare_turn_skill_listing), the
//! per-turn orchestration run before each LLM request: compaction
//! snapshot reset, conditional skill activation detection, listing
//! computation with incremental diff, and activation-turn filtering.
//! Consumed by the invoke family in `super::session_llm`; the listing
//! computation primitives it builds on live in `super::skill_listing`.

use super::ConversationSession;

/// Per-turn skill listing preparation (main flow).
impl ConversationSession {
    /// Prepare the skill listing for the current turn.
    ///
    /// Handles conditional activation detection, state update, and
    /// listing computation. Newly activated skills are NOT injected
    /// in the current turn — only base-listing changes are included;
    /// the activation entries will naturally appear in the next
    /// turn's diff.
    ///
    /// Returns the listing content to inject (`None` when nothing
    /// to inject).
    pub(super) fn prepare_turn_skill_listing(&mut self, content: &str) -> Option<String> {
        self.reset_compaction_snapshot_if_pending();
        let newly_activated = self.detect_conditional_activations(content);
        self.apply_conditional_activations(&newly_activated);
        let (listing, new_snapshot) = self.compute_skill_listing_for_turn();
        self.maybe_update_snapshot(&newly_activated, new_snapshot);
        self.filter_listing_on_activation_turns(listing, &newly_activated)
    }

    /// Reset the skill listing snapshot after compaction.
    fn reset_compaction_snapshot_if_pending(&mut self) {
        if self.pending_compaction_listing_reset && self.skill_listing_snapshot.is_some() {
            tracing::debug!(
                session_id = %self.session_id,
                "prepare_turn_skill_listing: clearing snapshot after compaction"
            );
            self.skill_listing_snapshot = None;
            self.pending_compaction_listing_reset = false;
        }
    }

    /// Extract file paths from the content and find newly activated
    /// conditional skills.
    fn detect_conditional_activations(&self, content: &str) -> std::collections::HashSet<String> {
        let paths = Self::extract_file_paths(content);
        let mut newly_activated = std::collections::HashSet::new();
        if !paths.is_empty() {
            if let Some(provider) = self.skill_listing_provider.as_ref() {
                let matches = provider.find_conditional_matches(&paths);
                for m in matches {
                    if !self.is_skill_already_activated(&m.name) {
                        newly_activated.insert(m.name);
                    }
                }
            }
        }
        if !newly_activated.is_empty() {
            tracing::info!(
                session_id = %self.session_id,
                event = "conditional_skill_activation",
                activated = ?newly_activated.iter().collect::<Vec<_>>(),
                "conditionally activated skills for current turn"
            );
        }
        newly_activated
    }

    /// Check if a skill name is not yet in the activated set.
    fn is_skill_already_activated(&self, name: &str) -> bool {
        self.activated_conditional_skills.contains(name)
    }

    /// Apply newly activated conditional skills to session state.
    ///
    /// Passes `None` for the snapshot to avoid updating it — the
    /// next turn's diff will pick up the new entries.
    fn apply_conditional_activations(
        &mut self,
        newly_activated: &std::collections::HashSet<String>,
    ) {
        if !newly_activated.is_empty() {
            self.apply_skill_listing_update(None, newly_activated);
        }
    }

    /// Conditionally update the snapshot based on activation state.
    ///
    /// On activation turns, skip saving so the next turn's diff
    /// naturally includes the newly activated entry.
    fn maybe_update_snapshot(
        &mut self,
        newly_activated: &std::collections::HashSet<String>,
        new_snapshot: Option<String>,
    ) {
        if newly_activated.is_empty() {
            if let Some(snapshot) = new_snapshot {
                self.skill_listing_snapshot = Some(snapshot);
            }
        }
    }
}

/// Skill listing filter helpers (activation-turn handling).
impl ConversationSession {
    /// Strip newly activated entries from the listing on activation
    /// turns so only base-listing changes are injected this turn.
    fn filter_listing_on_activation_turns(
        &self,
        listing: Option<String>,
        newly_activated: &std::collections::HashSet<String>,
    ) -> Option<String> {
        if newly_activated.is_empty() {
            return listing;
        }
        listing.and_then(|l| Self::filter_listing_excluding_entries(l, newly_activated))
    }

    /// Filter a skill listing to exclude entries matching given skill
    /// names.
    ///
    /// Removes lines that are formatted skill entries (starting with
    /// `- **`) for the specified skill names, and returns `None` if
    /// the result is empty.
    fn filter_listing_excluding_entries(
        listing: String,
        exclude_names: &std::collections::HashSet<String>,
    ) -> Option<String> {
        let filtered: Vec<&str> = listing
            .lines()
            .filter(|line| !line.is_empty() && !Self::is_entry_for_skill(line, exclude_names))
            .collect();
        let result = filtered.join("\n");
        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// Check if a listing line is an entry for one of the given skill
    /// names.
    ///
    /// Matches the pattern `- **{name**:` to avoid substring false
    /// matches on partial skill names.
    fn is_entry_for_skill(line: &str, skill_names: &std::collections::HashSet<String>) -> bool {
        line.starts_with("- **")
            && skill_names
                .iter()
                .any(|name| line.contains(&format!("**{}**:", name)))
    }
}
