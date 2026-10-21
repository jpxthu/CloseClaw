//! Cache prefix snapshot and pure eligibility logic for the cache hit
//! consistency check.
//!
//! The session enhancement layer snapshots each round's actually-sent
//! request prefix materials (whole System Prompt including the dynamic
//! section, tool definitions, and the Thinking-cleaned message list).
//! Once the next request has been built, the snapshot decides whether
//! that round is a tail-append extension of the previous one and thus
//! participates in the numeric cache hit consistency check
//! ([llm-session-enhancements](../../../../docs/design/session/llm-session-enhancements.md)
//! "缓存命中一致性校验").
//!
//! All hashing is process-local deterministic (`DefaultHasher` over the
//! serde JSON representation). Snapshots are session runtime state and
//! are never persisted, so cross-version / cross-process hash stability
//! is not required.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use closeclaw_common::{InternalMessage, ToolDefinition};

// ── Prefix snapshot ──────────────────────────────────────────────

/// Snapshot of one round's actually-sent request prefix materials.
///
/// Holds hashes instead of raw content: only equality against the next
/// round's snapshot is ever needed. Hashes are process-local
/// deterministic; snapshots are runtime state, not persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestPrefixSnapshot {
    /// Hash of the whole System Prompt (static + dynamic as sent).
    pub system_hash: u64,
    /// Hash of the stable representation of the tool definitions.
    ///
    /// Production shape (口径澄清): session-built requests carry
    /// tool definitions inside the System Prompt's ToolsSection and send
    /// the API `tools` parameter as `None` (see
    /// `ConversationSession::build_llm_request`); the tools-via-
    /// System-Prompt / `tools`-parameter split is grounded in design
    /// `docs/design/llm/cache-adapter.md`「工具 Schema 缓存」. The
    /// snapshot shape itself (whole System Prompt incl. the dynamic
    /// section, Thinking-cleaned tool definitions and message list)
    /// follows design
    /// `docs/design/session/llm-session-enhancements.md:73`
    /// (「前置条件（追加扩展轮）」). Given the production sending shape,
    /// any tool-list change is already covered by the `system_hash`
    /// dimension above; this separate dimension keeps the snapshot
    /// correct for callers that do populate the API `tools` parameter.
    pub tools_hash: u64,
    /// Per-message hash sequence of the actually-sent message list.
    pub message_hashes: Vec<u64>,
}

/// Builds a prefix snapshot from the actually-sent request materials.
///
/// `system_static` / `system_dynamic` are the two system prompt fields
/// as sent; the whole System Prompt is their concatenation (dynamic
/// section included). `tools` is the API `tools` parameter (`None` and
/// empty are treated alike). `messages` is the Thinking-cleaned message
/// list about to be sent to the provider.
pub fn build_prefix_snapshot(
    system_static: Option<&str>,
    system_dynamic: Option<&str>,
    tools: Option<&[ToolDefinition]>,
    messages: &[InternalMessage],
) -> RequestPrefixSnapshot {
    RequestPrefixSnapshot {
        system_hash: hash_str(&joined_system(system_static, system_dynamic)),
        tools_hash: hash_tools(tools),
        message_hashes: messages.iter().map(hash_message).collect(),
    }
}

/// Returns whether `current` is a tail-append extension of `previous`.
///
/// The System Prompt and tool definitions must be unchanged (equal
/// hashes), and the current message hash sequence must extend the
/// previous one: same length with all entries equal, or strictly
/// longer with the previous sequence as its leading prefix. A shorter
/// or otherwise diverging sequence is not an extension.
pub fn prefix_matches(previous: &RequestPrefixSnapshot, current: &RequestPrefixSnapshot) -> bool {
    previous.system_hash == current.system_hash
        && previous.tools_hash == current.tools_hash
        && current.message_hashes.len() >= previous.message_hashes.len()
        && current
            .message_hashes
            .iter()
            .zip(previous.message_hashes.iter())
            .all(|(cur, prev)| cur == prev)
}

// ── Eligibility (exemption) judgment ─────────────────────────────

/// Previous round's runtime state relevant to the cache check.
///
/// Bundles the snapshot of the previous round's actually-sent request
/// prefix with its model/provider identifier and call timestamp; all
/// three are advanced together at the unified advancement point (every
/// round once its usage has fully arrived, exempt rounds included).
/// `snapshot` is `None` on the first round after session start /
/// restore / compaction (new baseline); `model` and `called_at_ms`
/// mirror `RunningStats::last_call_model` / `last_call_at` and are
/// `None` before the first accumulated call.
#[derive(Debug, Clone, Copy)]
pub struct PreviousCallState<'a> {
    /// Prefix snapshot of the previous round, if one has been taken.
    pub snapshot: Option<&'a RequestPrefixSnapshot>,
    /// Model/provider identifier actually used by the previous round.
    pub model: Option<&'a str>,
    /// Millisecond timestamp of the previous call.
    pub called_at_ms: Option<u64>,
}

/// Outcome of the cache check eligibility judgment for one round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheCheckEligibility {
    /// Prefix preserved, same model/provider, and within the provider
    /// cache TTL — the round is a comparable append-extension and
    /// enters the numeric check.
    Participate,
    /// The round is exempt from the numeric check (this round only).
    Exempt(CacheExemptReason),
}

/// Why a round is exempt from the cache hit consistency check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheExemptReason {
    /// No previous snapshot (first round, or after restore /
    /// compaction): new baseline, nothing to compare against.
    NoPreviousSnapshot,
    /// Request prefix not preserved (System Prompt, tools, or message
    /// history changed): no valid reference.
    PrefixMismatch,
    /// Model/provider differs from the previous round: token counts
    /// are not comparable.
    ModelSwitch,
    /// Interval since the previous call exceeds the provider cache
    /// TTL: the cache is expected to have expired naturally.
    CacheExpired,
}

/// Decides whether the current round participates in the cache hit
/// consistency check.
///
/// Exemption checks, first match wins:
/// 1. no previous snapshot →
///    [`NoPreviousSnapshot`](CacheExemptReason::NoPreviousSnapshot);
/// 2. prefix mismatch → [`PrefixMismatch`](CacheExemptReason::PrefixMismatch);
/// 3. model/provider identifier differs →
///    [`ModelSwitch`](CacheExemptReason::ModelSwitch);
/// 4. interval since the previous call exceeds `ttl_ms` →
///    [`CacheExpired`](CacheExemptReason::CacheExpired) (an unknown previous
///    timestamp cannot prove freshness and counts as expired).
///
/// Pure decision only: the exemption covers this round; snapshot
/// advancement is the caller's responsibility and happens regardless
/// of the outcome. Missing or zero reference token counts are handled
/// by the numeric check itself (`detect_cache_anomaly`), not here.
pub fn cache_check_eligibility(
    previous: &PreviousCallState<'_>,
    current: &RequestPrefixSnapshot,
    current_model: Option<&str>,
    now_ms: u64,
    ttl_ms: u64,
) -> CacheCheckEligibility {
    let Some(prev_snapshot) = previous.snapshot else {
        return CacheCheckEligibility::Exempt(CacheExemptReason::NoPreviousSnapshot);
    };
    if !prefix_matches(prev_snapshot, current) {
        return CacheCheckEligibility::Exempt(CacheExemptReason::PrefixMismatch);
    }
    if previous.model != current_model {
        return CacheCheckEligibility::Exempt(CacheExemptReason::ModelSwitch);
    }
    let expired = previous
        .called_at_ms
        .is_none_or(|called_at| now_ms.saturating_sub(called_at) > ttl_ms);
    if expired {
        return CacheCheckEligibility::Exempt(CacheExemptReason::CacheExpired);
    }
    CacheCheckEligibility::Participate
}

// ── Hashing helpers ──────────────────────────────────────────────

/// Concatenates the two system prompt fields into the whole System
/// Prompt as sent (dynamic section included).
fn joined_system(system_static: Option<&str>, system_dynamic: Option<&str>) -> String {
    match (system_static, system_dynamic) {
        (Some(s), Some(d)) => format!("{s}\n{d}"),
        (Some(s), None) => s.to_string(),
        (None, Some(d)) => d.to_string(),
        (None, None) => String::new(),
    }
}

/// Runs one hashing session: creates the process-local hasher, lets
/// `feed` write the hash inputs in order, and returns the digest.
fn digest_with(feed: impl FnOnce(&mut DefaultHasher)) -> u64 {
    let mut hasher = DefaultHasher::new();
    feed(&mut hasher);
    hasher.finish()
}

/// Hashes one string via its `Hash` impl.
fn hash_str(value: &str) -> u64 {
    digest_with(|hasher| value.hash(hasher))
}

/// Hashes the stable representation of the `tools` parameter.
///
/// Covers name / description / input schema in order; the per-round
/// cache marking flag (`ToolDefinition::cache`) is not definition
/// content and is excluded. In production the `tools` parameter is
/// `None` (tool definitions travel via the System Prompt ToolsSection,
/// covered by `system_hash`); see
/// [`tools_hash`](RequestPrefixSnapshot::tools_hash).
fn hash_tools(tools: Option<&[ToolDefinition]>) -> u64 {
    digest_with(|hasher| {
        let list = tools.unwrap_or(&[]);
        list.len().hash(hasher);
        for tool in list {
            tool.name.hash(hasher);
            tool.description.hash(hasher);
            serde_json::to_string(&tool.input_schema)
                .unwrap_or_default()
                .hash(hasher);
        }
    })
}

/// Hashes one message via its serde JSON form (messages carry serde;
/// the representation is stable within a process).
fn hash_message(message: &InternalMessage) -> u64 {
    digest_with(|hasher| {
        serde_json::to_string(message)
            .unwrap_or_default()
            .hash(hasher);
    })
}
