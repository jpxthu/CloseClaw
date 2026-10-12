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

#[cfg(test)]
mod tests {
    use super::{default_usage, text_block, usage_response, usage_with};

    #[test]
    fn text_block_sets_text_type_and_content_only() {
        let block = text_block("hello");
        assert_eq!(block.block_type, "text");
        assert_eq!(block.content.as_deref(), Some("hello"));
        assert_eq!(block.tool_name, None);
        assert_eq!(block.tool_arguments, None);
        assert_eq!(block.reasoning, None);
        assert_eq!(block.signature, None);
    }

    #[test]
    fn default_usage_uses_fixed_tokens_and_no_cache_fields() {
        let usage = default_usage();
        assert_eq!(usage.prompt_tokens, Some(10));
        assert_eq!(usage.completion_tokens, Some(20));
        assert_eq!(usage.reasoning_tokens, None);
        assert_eq!(usage.cache_hit_tokens, None);
        assert_eq!(usage.cache_write_tokens, None);
        assert!(!usage.cache_fields_missing);
    }

    #[test]
    fn usage_with_maps_four_parameters_and_keeps_fixed_fields() {
        let usage = usage_with(Some(1), Some(2), Some(3), Some(4));
        assert_eq!(usage.prompt_tokens, Some(1));
        assert_eq!(usage.completion_tokens, Some(2));
        assert_eq!(usage.cache_hit_tokens, Some(3));
        assert_eq!(usage.cache_write_tokens, Some(4));
        assert_eq!(usage.reasoning_tokens, None);
        assert!(!usage.cache_fields_missing);
    }

    #[test]
    fn usage_with_passes_none_through_without_filling_fields() {
        let usage = usage_with(None, None, None, None);
        assert_eq!(usage.prompt_tokens, None);
        assert_eq!(usage.completion_tokens, None);
        assert_eq!(usage.cache_hit_tokens, None);
        assert_eq!(usage.cache_write_tokens, None);
        assert_eq!(usage.reasoning_tokens, None);
        assert!(!usage.cache_fields_missing);
    }

    #[test]
    fn usage_response_matches_usage_with_some_none() {
        let usage = usage_response(7, 9);
        let expected = usage_with(Some(7), Some(9), None, None);
        assert_eq!(usage.prompt_tokens, expected.prompt_tokens);
        assert_eq!(usage.completion_tokens, expected.completion_tokens);
        assert_eq!(usage.reasoning_tokens, expected.reasoning_tokens);
        assert_eq!(usage.cache_hit_tokens, expected.cache_hit_tokens);
        assert_eq!(usage.cache_write_tokens, expected.cache_write_tokens);
        assert_eq!(usage.cache_fields_missing, expected.cache_fields_missing);
    }
}
