//! Cache hit consistency check: snapshot staging, eligibility judgment,
//! and unified snapshot advancement.
//!
//! Data flow (see `docs/design/session/llm-session-enhancements.md`,
//! "缓存命中一致性校验", doc :73):
//! 1. **Staging** — both LLM invocation paths
//!    ([`invoke_llm_with_target`](ConversationSession::invoke_llm_with_target) /
//!    [`invoke_llm_streaming_with_target`](ConversationSession::invoke_llm_streaming_with_target))
//!    stage the actually-sent request's prefix snapshot once the request
//!    is built (messages already Thinking-cleaned, system static/dynamic
//!    finalized).
//! 2. **Judgment + advancement** —
//!    [`check_cache_anomaly_and_advance`](ConversationSession::check_cache_anomaly_and_advance)
//!    runs at the unified advancement point (usage fully arrived), compares the staged
//!    snapshot against the previous one (plus model/provider identity and
//!    TTL interval), runs the existing numeric comparison only for
//!    participating rounds, then advances the snapshot — exempt rounds
//!    advance too, so an exemption covers that round only.
//!
//! Compaction calls go through the injected `chat_fn` and never stage or
//! advance; the first round after compaction / restore becomes a new
//! baseline naturally (doc :85). Snapshots are session runtime state and
//! are never persisted to the checkpoint.

use closeclaw_common::{CacheAnomalyInfo, InternalMessage, ToolDefinition, UnifiedUsage};

use super::{
    cache_prefix::{
        build_prefix_snapshot, cache_check_eligibility, CacheCheckEligibility, PreviousCallState,
    },
    ConversationSession,
};

impl ConversationSession {
    /// Stages the current round's prefix snapshot (request-build point).
    ///
    /// Called by both invocation paths once the
    /// [`InternalRequest`](closeclaw_common::InternalRequest) is built; materials
    /// are the actually-sent system prompt (static + dynamic as sent), the
    /// API `tools` parameter, and the Thinking-cleaned message list.
    /// Compaction calls bypass the invocation paths and never stage.
    pub(crate) fn stage_cache_prefix_snapshot(
        &mut self,
        system_static: Option<&str>,
        system_dynamic: Option<&str>,
        tools: Option<&[ToolDefinition]>,
        messages: &[InternalMessage],
    ) {
        let snapshot = build_prefix_snapshot(system_static, system_dynamic, tools, messages);
        self.cache_prefix_staging = Some(snapshot);
    }

    /// Judges this round's cache hit consistency and advances the prefix
    /// snapshot (the unified advancement point).
    ///
    /// Reads the staged snapshot plus the previous call's runtime state
    /// ([`RunningStats`](closeclaw_common::RunningStats)
    /// `last_prompt_tokens` / `last_call_model` / `last_call_at`) and runs
    /// the eligibility judgment (tail-append extension, model/provider
    /// identity, provider cache TTL). Only participating rounds enter the
    /// existing numeric comparison; exempt rounds return `None`.
    ///
    /// Regardless of the outcome the snapshot is advanced to this round's
    /// staged snapshot (exempt rounds included — the exemption covers this
    /// round only). A round without a staged snapshot (compaction-style
    /// calls never stage) leaves the previous snapshot untouched.
    ///
    /// Must run **before** [`accumulate_usage`](ConversationSession::accumulate_usage) so
    /// `last_prompt_tokens` / `last_call_model` / `last_call_at` still
    /// hold the previous call's values; `current_model` must be the same
    /// identifier later passed to `accumulate_usage` (the actual
    /// `provider/model` string, falling back to the session-config model
    /// when the caller reports no target), otherwise rounds would be
    /// wrongly flagged as model switches.
    ///
    /// `ttl_ms` is the provider cache TTL resolved by the gateway from
    /// the actually-used provider id (session does not depend on the llm
    /// crate); `now_ms` is the current Unix epoch time in milliseconds.
    pub fn check_cache_anomaly_and_advance(
        &mut self,
        usage: &UnifiedUsage,
        current_model: Option<&str>,
        now_ms: u64,
        ttl_ms: u64,
    ) -> Option<CacheAnomalyInfo> {
        // Take the staging slot: only rounds whose request was built
        // through the session invocation paths advance the snapshot.
        let Some(current) = self.cache_prefix_staging.take() else {
            tracing::debug!(
                session_id = %self.session_id,
                "cache check: no staged snapshot (compaction-style call), skipping"
            );
            return None;
        };
        let previous = PreviousCallState {
            snapshot: self.cache_prefix_prev.as_ref(),
            model: self.stats.last_call_model.as_deref(),
            called_at_ms: self.stats.last_call_at,
        };
        let eligibility =
            cache_check_eligibility(&previous, &current, current_model, now_ms, ttl_ms);
        let info = match eligibility {
            CacheCheckEligibility::Participate => self.stats.detect_cache_anomaly_and_update(
                usage.cache_read_tokens,
                Some(usage.prompt_tokens),
            ),
            CacheCheckEligibility::Exempt(reason) => {
                tracing::debug!(
                    session_id = %self.session_id,
                    reason = ?reason,
                    "cache check: round exempt from numeric comparison"
                );
                None
            }
        };
        // Unified advancement: exempt rounds advance too, so the
        // exemption applies to this round only.
        self.cache_prefix_prev = Some(current);
        info
    }
}

#[cfg(test)]
use super::cache_prefix::RequestPrefixSnapshot;

/// Test audit accessors for the advancement-timing guarantees
/// (`#[cfg(test)]`-only; kept in a separate impl block so the main
/// implementation block stays within the 100-line limit).
#[cfg(test)]
impl ConversationSession {
    /// Returns the previous round's advanced prefix snapshot, if any
    /// (test audit accessor for the advancement-timing guarantees).
    pub(crate) fn cache_prefix_prev_snapshot(&self) -> Option<&RequestPrefixSnapshot> {
        self.cache_prefix_prev.as_ref()
    }

    /// Peeks at the staged snapshot without consuming it (test audit
    /// accessor for the staging-slot lifecycle).
    pub(crate) fn staged_cache_prefix_snapshot(&self) -> Option<&RequestPrefixSnapshot> {
        self.cache_prefix_staging.as_ref()
    }
}
