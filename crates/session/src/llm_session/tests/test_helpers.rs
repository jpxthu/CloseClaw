//! Shared helper factories for the `llm_session` test suites (cache
//! prefix check, message push dedup, KV cache, stats integration).
//!
//! Mounted by `tests/mod.rs` and re-exposed there (`use
//! self::test_helpers::{…}`, mirroring the
//! `closeclaw_test_support::log_capture` re-export pattern) so callers
//! always use the plain `super::` form. Suites that
//! used the narrow 3-parameter usage builder import it aliased
//! (`use super::make_usage_basic as make_usage;`) to keep call sites
//! unchanged.

use crate::llm_session::cache_prefix::{build_prefix_snapshot, RequestPrefixSnapshot};
use closeclaw_common::{ContentBlock, InternalMessage, UnifiedUsage};

/// Provider cache TTL magnitude default (cache-adapter doc :27: 5 min).
pub(super) const TTL_MS: u64 = 300_000;

/// Current Unix epoch time in milliseconds (shared
/// `closeclaw_common::current_epoch_millis` clock — the same one
/// `RunningStats::accumulate` uses).
pub(super) fn now_ms() -> u64 {
    closeclaw_common::current_epoch_millis()
}

/// User-role internal message with plain text content.
pub(super) fn user_msg(content: &str) -> InternalMessage {
    InternalMessage {
        role: "user".to_string(),
        content: content.to_string(),
        content_blocks: None,
        tool_call_id: None,
    }
}

/// Assistant-role internal message with plain text content.
pub(super) fn assistant_msg(content: &str) -> InternalMessage {
    InternalMessage {
        role: "assistant".to_string(),
        content: content.to_string(),
        content_blocks: None,
        tool_call_id: None,
    }
}

/// Empty tool-role placeholder message: the converted shape of a
/// tool-role transcript entry itself (`inject_tool_result` history
/// conversion emits it before the promoted ToolResult message).
pub(super) fn tool_placeholder() -> InternalMessage {
    InternalMessage {
        role: "tool".to_string(),
        content: String::new(),
        content_blocks: None,
        tool_call_id: None,
    }
}

/// Tool-role message carrying a promoted ToolResult block
/// (`inject_tool_result` shape) with its `tool_call_id`.
pub(super) fn tool_result_msg(id: &str, content: &str) -> InternalMessage {
    InternalMessage {
        role: "tool".to_string(),
        content: content.to_string(),
        content_blocks: None,
        tool_call_id: Some(id.to_string()),
    }
}

/// Workflow-role control message (`inject_workflow_message` shape).
pub(super) fn workflow_msg(content: &str) -> InternalMessage {
    InternalMessage {
        role: "workflow".to_string(),
        content: content.to_string(),
        content_blocks: None,
        tool_call_id: None,
    }
}

/// User-role message with structured content blocks and empty
/// flattened text (the image-round pre-append shape, where Image
/// blocks are excluded from the text flatten).
pub(super) fn user_blocks_msg(blocks: &[ContentBlock]) -> InternalMessage {
    InternalMessage {
        role: "user".to_string(),
        content: String::new(),
        content_blocks: Some(blocks.to_vec()),
        tool_call_id: None,
    }
}

/// Build a `UnifiedUsage` with explicit total / cache fields.
pub(super) fn make_usage(
    prompt: u32,
    completion: u32,
    total: Option<u32>,
    cache_read: Option<u32>,
    cache_write: Option<u32>,
) -> UnifiedUsage {
    UnifiedUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        reasoning_tokens: None,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
    }
}

/// Narrow form used by suites that always send an exact total and no
/// cache-write: `total = prompt + completion`, `cache_write = None`.
pub(super) fn make_usage_basic(
    prompt: u32,
    completion: u32,
    cache_read: Option<u32>,
) -> UnifiedUsage {
    make_usage(
        prompt,
        completion,
        Some(prompt + completion),
        cache_read,
        None,
    )
}

/// Prefix snapshot from a constant system prompt and message list
/// (no dynamic section, no tools — the common test shape).
pub(super) fn snapshot(system: &str, msgs: &[InternalMessage]) -> RequestPrefixSnapshot {
    build_prefix_snapshot(Some(system), None, None, msgs)
}
