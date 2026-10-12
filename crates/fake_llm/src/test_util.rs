//! Shared test helpers for constructing protocol-agnostic response fixtures.

use crate::scenario::types::{ResponseBlock, UsageResponse};

/// Build a text-only `ResponseBlock` with the given content.
pub(crate) fn text_block(content: &str) -> ResponseBlock {
    ResponseBlock {
        block_type: "text".to_string(),
        content: Some(content.to_string()),
        tool_name: None,
        tool_arguments: None,
        reasoning: None,
        signature: None,
    }
}

/// Build a `UsageResponse` with fixed prompt=10 / completion=20 tokens.
pub(crate) fn default_usage() -> UsageResponse {
    UsageResponse {
        prompt_tokens: Some(10),
        completion_tokens: Some(20),
        reasoning_tokens: None,
        cache_hit_tokens: None,
        cache_write_tokens: None,
        cache_fields_missing: false,
    }
}

/// Build a `UsageResponse` from explicit prompt/completion/cache token fields.
pub(crate) fn usage_with(
    prompt: Option<u32>,
    completion: Option<u32>,
    cache_hit: Option<u32>,
    cache_write: Option<u32>,
) -> UsageResponse {
    UsageResponse {
        prompt_tokens: prompt,
        completion_tokens: completion,
        reasoning_tokens: None,
        cache_hit_tokens: cache_hit,
        cache_write_tokens: cache_write,
        cache_fields_missing: false,
    }
}

/// Build a `UsageResponse` carrying only prompt/completion token counts.
pub(crate) fn usage_response(prompt_tokens: u32, completion_tokens: u32) -> UsageResponse {
    usage_with(Some(prompt_tokens), Some(completion_tokens), None, None)
}
