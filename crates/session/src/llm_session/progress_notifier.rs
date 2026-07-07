//! Implementation of [`PlanStateNotifier`] for [`ConversationSession`].
//!
//! Maps execution progress changes to per-session `system_appends`
//! entries using a fixed prefix marker so the progress is automatically
//! included in the system prompt via `AppendSection`.
//!
//! Convention: the progress append item starts with [`PROGRESS_APPEND_PREFIX`]
//! so that subsequent updates **replace** the existing entry instead of
//! appending duplicates.

use async_trait::async_trait;

use closeclaw_common::PlanStateNotifier;

use super::ConversationSession;

/// Prefix marker for progress-related entries in `system_appends`.
///
/// Any `system_appends` entry whose text starts with this prefix is
/// treated as the current execution progress. When
/// [`ConversationSession::on_progress_changed`] fires, it finds the
/// existing entry (by prefix) and replaces it in-place, or appends a
/// new one if none exists yet.
pub const PROGRESS_APPEND_PREFIX: &str = "__progress__:";

#[async_trait]
impl PlanStateNotifier for ConversationSession {
    /// Called by [`ExecutionEngine`](closeclaw_common::execution::ExecutionEngine)
    /// after a step status transition succeeds.
    ///
    /// Replaces the existing progress entry (identified by
    /// [`PROGRESS_APPEND_PREFIX`]) in `system_appends`, or appends a
    /// new one. When `progress_summary` is empty the entry is removed.
    async fn on_progress_changed(&self, progress_summary: &str) {
        let mut lock = self
            .progress_appends
            .lock()
            .expect("progress_appends lock poisoned");

        if progress_summary.is_empty() {
            lock.retain(|s| !s.starts_with(PROGRESS_APPEND_PREFIX));
            return;
        }

        let tagged = format!("{}{}", PROGRESS_APPEND_PREFIX, progress_summary);

        // Replace existing entry in-place if present.
        if let Some(slot) = lock
            .iter_mut()
            .find(|s| s.starts_with(PROGRESS_APPEND_PREFIX))
        {
            *slot = truncate_to_limit(tagged);
        } else {
            lock.push(truncate_to_limit(tagged));
        }
    }
}

/// Truncate `content` to [`super::APPEND_SECTION_MAX_LEN`] chars if needed.
fn truncate_to_limit(content: String) -> String {
    use super::APPEND_SECTION_MAX_LEN;

    if content.chars().count() > APPEND_SECTION_MAX_LEN {
        content.chars().take(APPEND_SECTION_MAX_LEN).collect()
    } else {
        content
    }
}

#[cfg(test)]
#[path = "progress_notifier_tests.rs"]
mod tests;
