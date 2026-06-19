//! Unit tests for the GLM provider.

use super::*;
use crate::llm::LLMError;

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
