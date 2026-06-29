//! Unit tests for the GLM provider.

use super::*;
use crate::LLMError;

// ---------------------------------------------------------------------------//
// mock_integration — HTTP-level mock tests covering the full send() pipeline  //
// ---------------------------------------------------------------------------//
mod mock_extra;
mod mock_integration;
mod mock_usage;

// --- Fixture-based deserialization and content extraction tests ---

// TODO: Rewrite with v2 fixtures (glm/{model}/openai/ and glm/{model}/anthropic/)
// #[test]
// fn test_glm_5_1_chat_extract_reasoning() { ... }
// #[test]
// fn test_glm_4_7_simple_chat_extract_reasoning() { ... }
// #[test]
// fn test_glm_4_5_air_chat_extract_reasoning() { ... }
// #[test]
// fn test_glm_5_1_multi_turn() { ... }

// --- Error mapping tests ---

// TODO: Rewrite with v2 error fixtures (glm/{model}/openai/error-*.json)
// #[test]
// fn test_glm_error_invalid_model() { ... }
// #[test]
// fn test_glm_error_empty_messages() { ... }

#[test]
fn test_glm_error_unknown_code() {
    let err = GlmProvider::map_glm_error("9999", "some unknown error");
    match err {
        ProviderError::Legacy(msg) => {
            assert!(msg.contains("9999"), "should contain 9999");
        }
        other => panic!("Expected Legacy error, got: {:?}", other),
    }
}

// --- extract_content edge cases ---

#[test]
fn test_extract_content_prefers_non_empty_content() {
    let msg = GlmMessage {
        role: "assistant".to_string(),
        content: "Hello, World!".to_string(),
        reasoning_content: Some("I am thinking...".to_string()),
    };
    let extracted = GlmProvider::extract_content(&msg);
    assert_eq!(extracted, "Hello, World!");
}

#[test]
fn test_extract_content_falls_back_to_reasoning() {
    let msg = GlmMessage {
        role: "assistant".to_string(),
        content: "   ".to_string(),
        reasoning_content: Some("Thinking process...".to_string()),
    };
    let extracted = GlmProvider::extract_content(&msg);
    assert_eq!(extracted, "Thinking process...");
}

#[test]
fn test_extract_content_whitespace_only_reasoning() {
    let msg = GlmMessage {
        role: "assistant".to_string(),
        content: "".to_string(),
        reasoning_content: Some("   ".to_string()),
    };
    let extracted = GlmProvider::extract_content(&msg);
    assert_eq!(extracted, "");
}

#[test]
fn test_extract_content_both_empty() {
    let msg = GlmMessage {
        role: "assistant".to_string(),
        content: "".to_string(),
        reasoning_content: None,
    };
    let extracted = GlmProvider::extract_content(&msg);
    assert_eq!(extracted, "");
}

// --- parse_chat_response tests ---

/// Build a GlmResponse with one choice, optional content, and
/// optional reasoning_content.
fn make_glm_response(content: &str, reasoning: Option<&str>) -> GlmResponse {
    GlmResponse {
        choices: Some(vec![GlmChoice {
            message: GlmMessage {
                role: "assistant".to_string(),
                content: content.to_string(),
                reasoning_content: reasoning.map(String::from),
            },
        }]),
        usage: Some(GlmUsage {
            prompt_tokens: 10,
            completion_tokens: 20,
            total_tokens: 30,
            completion_tokens_details: None,
            prompt_tokens_details: None,
        }),
        model: "glm-5.1".to_string(),
        error: None,
    }
}

/// 1. Normal path: content + reasoning both present and long enough
///    → Text(content) + Thinking(reasoning)
#[test]
fn test_parse_chat_response_content_and_reasoning() {
    let resp = GlmProvider::parse_chat_response(make_glm_response(
        "Final answer",
        Some("Let me think step by step..."),
    ));
    let resp = resp.expect("should succeed");
    assert_eq!(resp.content_blocks.len(), 2);
    assert_eq!(
        resp.content_blocks[0],
        RawContentBlock::Text("Final answer".to_string())
    );
    assert_eq!(
        resp.content_blocks[1],
        RawContentBlock::Thinking {
            thinking: "Let me think step by step...".to_string(),
            signature: None,
        }
    );
}

/// 2. Degrade path: content empty + reasoning non-empty
///    → Text(reasoning) only, no Thinking block
#[test]
fn test_parse_chat_response_content_empty_reasoning_degraded() {
    let resp = GlmProvider::parse_chat_response(make_glm_response(
        "",
        Some("Hidden reasoning that becomes visible"),
    ));
    let resp = resp.expect("should succeed");
    assert_eq!(resp.content_blocks.len(), 1);
    assert_eq!(
        resp.content_blocks[0],
        RawContentBlock::Text("Hidden reasoning that becomes visible".to_string())
    );
}

/// 3. Short reasoning filtered: reasoning_content is 1 char (below
///    MIN_REASONING_LENGTH=2) → Thinking block not emitted.
#[test]
fn test_parse_chat_response_short_reasoning_filtered() {
    let resp = GlmProvider::parse_chat_response(make_glm_response("Hello", Some(".")));
    let resp = resp.expect("should succeed");
    assert_eq!(resp.content_blocks.len(), 1);
    assert_eq!(
        resp.content_blocks[0],
        RawContentBlock::Text("Hello".to_string())
    );
}

/// 4. Plain text: content non-empty, no reasoning_content
///    → Text(content) only
#[test]
fn test_parse_chat_response_text_only() {
    let resp = GlmProvider::parse_chat_response(make_glm_response("Just text", None));
    let resp = resp.expect("should succeed");
    assert_eq!(resp.content_blocks.len(), 1);
    assert_eq!(
        resp.content_blocks[0],
        RawContentBlock::Text("Just text".to_string())
    );
}

/// 5. Empty response: content empty, no reasoning_content
///    → empty Text block (fallback)
#[test]
fn test_parse_chat_response_empty_response() {
    let resp = GlmProvider::parse_chat_response(make_glm_response("", None));
    let resp = resp.expect("should succeed");
    assert_eq!(resp.content_blocks.len(), 1);
    assert_eq!(resp.content_blocks[0], RawContentBlock::Text(String::new()));
}

// --- Token details deserialization ---

// TODO: Rewrite with v2 fixture (glm/glm-5.2/openai/glm-thinking.json, etc.)
// #[test]
// fn test_glm_5_1_reasoning_tokens_details() { ... }

// --- fetch_model_list mock HTTP tests ---

// TODO: Rewrite with v2 fixture (glm/provider/model-list.json)
// #[tokio::test]
// async fn test_fetch_model_list_success_mock() { ... }

#[tokio::test]
async fn test_fetch_model_list_http_auth_failure_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/paas/v4/models")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"1210","message":"invalid api key"}}"#)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url(
        "fake-key".into(),
        format!("{}/api/coding/paas/v4/chat/completions", server.url()),
    );
    let err = provider.fetch_model_list("fake-key").await.unwrap_err();

    m.assert_async().await;
    match err {
        LLMError::ApiError(msg) => {
            assert!(msg.contains("401"), "should contain 401");
        }
        other => panic!("Expected ApiError, got: {:?}", other),
    }
}
