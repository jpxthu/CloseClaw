//! Shared test helpers for constructing protocol-agnostic response fixtures.

use crate::scenario::types::{
    MessageEntry, ReasoningIntensity, ReasoningResponse, ResponseBlock, ResponseShape,
    TextResponse, TurnResponse, UsageResponse,
};
use crate::types::{ProtocolKind, RequestFeatures};

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
    usage_with(Some(10), Some(20), None, None)
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

// ---------------------------------------------------------------------------
// TurnResponse-level helpers
// ---------------------------------------------------------------------------

/// Build a `TurnResponse` carrying `response` with no delay, interrupt,
/// or error injection.
fn turn_with(response: ResponseShape) -> TurnResponse {
    TurnResponse {
        response: response.into(),
        delay: None,
        first_token_delay: None,
        segment_delay: None,
        stream_interrupt_after: None,
        error: None,
    }
}

/// Build a text-only `TurnResponse` with no usage, delay, or error injection.
pub(crate) fn text_turn(content: &str) -> TurnResponse {
    turn_with(ResponseShape::Text(TextResponse {
        content: content.to_string(),
        usage: None,
    }))
}

/// Build a usage-only `TurnResponse` with fixed prompt=10 / completion=20 tokens.
pub(crate) fn usage_turn() -> TurnResponse {
    turn_with(ResponseShape::Usage(default_usage()))
}

/// Build a `TurnResponse` carrying the catch-all `Unknown` shape.
pub(crate) fn unknown_turn() -> TurnResponse {
    turn_with(ResponseShape::Unknown)
}

/// Build a reasoning `TurnResponse` with the given content, reasoning
/// text, and intensity.
pub(crate) fn reasoning_turn(
    content: &str,
    reasoning: &str,
    intensity: ReasoningIntensity,
) -> TurnResponse {
    turn_with(ResponseShape::Reasoning(ReasoningResponse {
        content: content.to_string(),
        reasoning: reasoning.to_string(),
        signature: None,
        usage: None,
        intensity,
    }))
}

// ---------------------------------------------------------------------------
// RequestFeatures-level helpers
// ---------------------------------------------------------------------------

/// Full-parameter entry point for building `RequestFeatures`.
///
/// Every other feature helper delegates here. `stream` is fixed to
/// `false` and `max_tokens` / `temperature` stay unset.
pub(crate) fn features_full(
    model: &str,
    messages: Vec<(&str, &str)>,
    tools: Vec<&str>,
    protocol: ProtocolKind,
) -> RequestFeatures {
    RequestFeatures {
        model: model.to_string(),
        stream: false,
        max_tokens: None,
        temperature: None,
        messages: messages
            .into_iter()
            .map(|(role, content)| MessageEntry {
                role: role.to_string(),
                content: content.to_string(),
            })
            .collect(),
        tools: tools.into_iter().map(str::to_string).collect(),
        protocol,
    }
}

/// Build non-streaming OpenAI `RequestFeatures` with a single user message.
pub(crate) fn features(model: &str, msg: &str) -> RequestFeatures {
    features_full(model, vec![("user", msg)], vec![], ProtocolKind::OpenAi)
}

/// Build `RequestFeatures` with a single user message under `protocol`.
pub(crate) fn features_with_protocol(
    model: &str,
    msg: &str,
    protocol: ProtocolKind,
) -> RequestFeatures {
    features_full(model, vec![("user", msg)], vec![], protocol)
}

/// Build non-streaming OpenAI `RequestFeatures` from role/content pairs.
pub(crate) fn features_multi(model: &str, messages: Vec<(&str, &str)>) -> RequestFeatures {
    features_full(model, messages, vec![], ProtocolKind::OpenAi)
}

/// Build non-streaming OpenAI `RequestFeatures` with user-role messages
/// and the given tool names.
pub(crate) fn features_with_tools(
    model: &str,
    msgs: Vec<&str>,
    tools: Vec<&str>,
) -> RequestFeatures {
    let messages: Vec<(&str, &str)> = msgs.into_iter().map(|m| ("user", m)).collect();
    features_full(model, messages, tools, ProtocolKind::OpenAi)
}

#[cfg(test)]
mod tests {
    use super::{
        default_usage, features, features_full, features_multi, features_with_protocol,
        features_with_tools, reasoning_turn, text_block, text_turn, unknown_turn, usage_response,
        usage_turn, usage_with,
    };
    use crate::scenario::types::{ReasoningIntensity, ResponseShape};
    use crate::types::ProtocolKind;

    #[test]
    fn test_text_block_sets_text_type_and_content_only() {
        let block = text_block("hello");
        assert_eq!(block.block_type, "text");
        assert_eq!(block.content.as_deref(), Some("hello"));
        assert_eq!(block.tool_name, None);
        assert_eq!(block.tool_arguments, None);
        assert_eq!(block.reasoning, None);
        assert_eq!(block.signature, None);
    }

    #[test]
    fn test_default_usage_uses_fixed_tokens_and_no_cache_fields() {
        let usage = default_usage();
        assert_eq!(usage.prompt_tokens, Some(10));
        assert_eq!(usage.completion_tokens, Some(20));
        assert_eq!(usage.reasoning_tokens, None);
        assert_eq!(usage.cache_hit_tokens, None);
        assert_eq!(usage.cache_write_tokens, None);
        assert!(!usage.cache_fields_missing);
    }

    #[test]
    fn test_usage_with_maps_four_parameters_and_keeps_fixed_fields() {
        let usage = usage_with(Some(1), Some(2), Some(3), Some(4));
        assert_eq!(usage.prompt_tokens, Some(1));
        assert_eq!(usage.completion_tokens, Some(2));
        assert_eq!(usage.cache_hit_tokens, Some(3));
        assert_eq!(usage.cache_write_tokens, Some(4));
        assert_eq!(usage.reasoning_tokens, None);
        assert!(!usage.cache_fields_missing);
    }

    #[test]
    fn test_usage_with_passes_none_through_without_filling_fields() {
        let usage = usage_with(None, None, None, None);
        assert_eq!(usage.prompt_tokens, None);
        assert_eq!(usage.completion_tokens, None);
        assert_eq!(usage.cache_hit_tokens, None);
        assert_eq!(usage.cache_write_tokens, None);
        assert_eq!(usage.reasoning_tokens, None);
        assert!(!usage.cache_fields_missing);
    }

    #[test]
    fn test_usage_response_matches_usage_with_some_none() {
        let usage = usage_response(7, 9);
        let expected = usage_with(Some(7), Some(9), None, None);
        assert_eq!(usage.prompt_tokens, expected.prompt_tokens);
        assert_eq!(usage.completion_tokens, expected.completion_tokens);
        assert_eq!(usage.reasoning_tokens, expected.reasoning_tokens);
        assert_eq!(usage.cache_hit_tokens, expected.cache_hit_tokens);
        assert_eq!(usage.cache_write_tokens, expected.cache_write_tokens);
        assert_eq!(usage.cache_fields_missing, expected.cache_fields_missing);
    }

    #[test]
    fn test_text_turn_produces_text_shape_with_no_delay_or_error() {
        let turn = text_turn("hello");
        let shapes = turn.response.to_shapes();
        assert_eq!(shapes.len(), 1);
        match &shapes[0] {
            ResponseShape::Text(t) => {
                assert_eq!(t.content, "hello");
                assert!(t.usage.is_none());
            }
            other => panic!("expected Text shape, got {:?}", other),
        }
        assert_eq!(turn.delay, None);
        assert_eq!(turn.first_token_delay, None);
        assert_eq!(turn.segment_delay, None);
        assert_eq!(turn.stream_interrupt_after, None);
        assert!(turn.error.is_none());
    }

    #[test]
    fn test_usage_turn_produces_usage_shape_with_fixed_tokens() {
        let turn = usage_turn();
        let shapes = turn.response.to_shapes();
        match &shapes[0] {
            ResponseShape::Usage(u) => {
                assert_eq!(u.prompt_tokens, Some(10));
                assert_eq!(u.completion_tokens, Some(20));
            }
            other => panic!("expected Usage shape, got {:?}", other),
        }
        assert!(turn.error.is_none());
    }

    #[test]
    fn test_unknown_turn_produces_unknown_shape() {
        let turn = unknown_turn();
        let shapes = turn.response.to_shapes();
        assert!(matches!(shapes[0], ResponseShape::Unknown));
        assert!(turn.error.is_none());
    }

    #[test]
    fn test_reasoning_turn_places_content_reasoning_and_intensity() {
        let turn = reasoning_turn("answer", "thinking", ReasoningIntensity::High);
        let shapes = turn.response.to_shapes();
        match &shapes[0] {
            ResponseShape::Reasoning(r) => {
                assert_eq!(r.content, "answer");
                assert_eq!(r.reasoning, "thinking");
                assert_eq!(r.intensity, ReasoningIntensity::High);
                assert_eq!(r.signature, None);
                assert!(r.usage.is_none());
            }
            other => panic!("expected Reasoning shape, got {:?}", other),
        }
    }

    #[test]
    fn test_features_defaults_to_openai_non_streaming_single_user_message() {
        let f = features("gpt-4", "hello");
        assert_eq!(f.model, "gpt-4");
        assert!(!f.stream);
        assert_eq!(f.max_tokens, None);
        assert_eq!(f.temperature, None);
        assert_eq!(f.protocol, ProtocolKind::OpenAi);
        assert!(f.tools.is_empty());
        assert_eq!(f.messages.len(), 1);
        assert_eq!(f.messages[0].role, "user");
        assert_eq!(f.messages[0].content, "hello");
    }

    #[test]
    fn test_features_with_protocol_applies_protocol_to_single_user_message() {
        let f = features_with_protocol("claude-3", "hi", ProtocolKind::Anthropic);
        assert_eq!(f.protocol, ProtocolKind::Anthropic);
        assert_eq!(f.messages.len(), 1);
        assert_eq!(f.messages[0].role, "user");
        assert_eq!(f.messages[0].content, "hi");
    }

    #[test]
    fn test_features_multi_preserves_message_order_and_content() {
        let f = features_multi(
            "m",
            vec![("user", "q1"), ("assistant", "a1"), ("user", "q2")],
        );
        let roles: Vec<&str> = f.messages.iter().map(|m| m.role.as_str()).collect();
        let contents: Vec<&str> = f.messages.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant", "user"]);
        assert_eq!(contents, vec!["q1", "a1", "q2"]);
        assert!(f.tools.is_empty());
        assert_eq!(f.protocol, ProtocolKind::OpenAi);
    }

    #[test]
    fn test_features_full_supports_empty_messages_with_protocol() {
        let f = features_full("gpt-4o", vec![], vec![], ProtocolKind::Anthropic);
        assert_eq!(f.model, "gpt-4o");
        assert!(!f.stream);
        assert!(f.messages.is_empty());
        assert!(f.tools.is_empty());
        assert_eq!(f.protocol, ProtocolKind::Anthropic);
    }

    #[test]
    fn test_features_full_combines_multi_messages_tools_and_protocol() {
        let f = features_full(
            "claude-3",
            vec![("user", "q"), ("assistant", "a")],
            vec!["web_search", "calc"],
            ProtocolKind::Anthropic,
        );
        assert_eq!(f.model, "claude-3");
        assert!(!f.stream);
        assert_eq!(f.max_tokens, None);
        assert_eq!(f.temperature, None);
        assert_eq!(f.protocol, ProtocolKind::Anthropic);
        assert_eq!(f.tools, vec!["web_search".to_string(), "calc".to_string()]);
        assert_eq!(f.messages.len(), 2);
        assert_eq!(f.messages[0].role, "user");
        assert_eq!(f.messages[0].content, "q");
        assert_eq!(f.messages[1].role, "assistant");
        assert_eq!(f.messages[1].content, "a");
    }

    #[test]
    fn test_features_with_tools_places_tools_in_given_order() {
        let f = features_with_tools("gpt-4o", vec!["q"], vec!["web_search", "calc"]);
        assert_eq!(f.messages.len(), 1);
        assert_eq!(f.messages[0].role, "user");
        assert_eq!(f.messages[0].content, "q");
        assert_eq!(f.tools, vec!["web_search".to_string(), "calc".to_string()]);
    }

    #[test]
    fn test_features_with_tools_empty_tools_matches_features_shape() {
        let f = features_with_tools("gpt-4", vec!["hi"], vec![]);
        let expected = features("gpt-4", "hi");
        assert_eq!(f.messages.len(), expected.messages.len());
        assert_eq!(f.messages[0].role, expected.messages[0].role);
        assert_eq!(f.messages[0].content, expected.messages[0].content);
        assert!(f.tools.is_empty());
        assert_eq!(f.protocol, expected.protocol);
        assert_eq!(f.stream, expected.stream);
    }
}
