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

#[test]
fn test_glm_5_1_chat_extract_reasoning() {
    let json =
        include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-chat.json");
    let resp: GlmResponse = serde_json::from_str(json).unwrap();
    let choice =
        resp.choices.as_ref().and_then(|c| c.first()).unwrap();
    let msg = &choice.message;
    let extracted = GlmProvider::extract_content(msg);
    assert!(
        msg.content.trim().is_empty(),
        "content should be empty/whitespace"
    );
    assert!(
        msg.reasoning_content.is_some()
            && !msg
                .reasoning_content
                .as_ref()
                .unwrap()
                .is_empty(),
        "reasoning_content should be non-empty"
    );
    assert!(!extracted.is_empty(), "should extract reasoning_content");
    assert_eq!(
        extracted,
        msg.reasoning_content.as_ref().unwrap().trim()
    );
    let usage = resp.usage.as_ref().unwrap();
    assert_eq!(usage.completion_tokens, 30);
    assert_eq!(usage.prompt_tokens, 11);
    assert_eq!(usage.total_tokens, 41);
    let details =
        usage.completion_tokens_details.as_ref().unwrap();
    assert_eq!(details.reasoning_tokens, Some(30));
    let prompt_details =
        usage.prompt_tokens_details.as_ref().unwrap();
    assert_eq!(prompt_details.cached_tokens, Some(0));
}

#[test]
fn test_glm_4_7_simple_chat_extract_reasoning() {
    let json = include_str!(
        "../../../../tests/fixtures/llm/glm/glm-4.7-simple-chat.json"
    );
    let resp: GlmResponse = serde_json::from_str(json).unwrap();
    let choice =
        resp.choices.as_ref().and_then(|c| c.first()).unwrap();
    let msg = &choice.message;
    let extracted = GlmProvider::extract_content(msg);
    assert!(msg.content.trim().is_empty(), "content should be empty");
    assert!(
        msg.reasoning_content.is_some()
            && !msg
                .reasoning_content
                .as_ref()
                .unwrap()
                .is_empty(),
        "reasoning_content should be non-empty"
    );
    assert!(!extracted.is_empty(), "should extract reasoning_content");
    assert_eq!(
        extracted,
        msg.reasoning_content.as_ref().unwrap().trim()
    );
    assert_eq!(resp.model, "GLM-4.7");
    let usage = resp.usage.as_ref().unwrap();
    let prompt_details =
        usage.prompt_tokens_details.as_ref().unwrap();
    assert_eq!(prompt_details.cached_tokens, Some(10));
}

#[test]
fn test_glm_4_5_air_chat_extract_reasoning() {
    let json = include_str!(
        "../../../../tests/fixtures/llm/glm/glm-4.5-air-chat.json"
    );
    let resp: GlmResponse = serde_json::from_str(json).unwrap();
    let choice =
        resp.choices.as_ref().and_then(|c| c.first()).unwrap();
    let msg = &choice.message;
    let extracted = GlmProvider::extract_content(msg);
    assert!(msg.content.trim().is_empty(), "content should be empty");
    assert!(
        msg.reasoning_content.is_some()
            && !msg
                .reasoning_content
                .as_ref()
                .unwrap()
                .is_empty(),
        "reasoning_content should be non-empty"
    );
    assert!(!extracted.is_empty(), "should extract reasoning_content");
    assert_eq!(resp.model, "GLM-4.5-Air");
}

#[test]
fn test_glm_5_1_multi_turn() {
    let json = include_str!(
        "../../../../tests/fixtures/llm/glm/glm-5.1-multi-turn.json"
    );
    let resp: GlmResponse = serde_json::from_str(json).unwrap();
    let choice =
        resp.choices.as_ref().and_then(|c| c.first()).unwrap();
    let msg = &choice.message;
    let extracted = GlmProvider::extract_content(msg);
    assert!(
        !extracted.is_empty(),
        "multi-turn should extract reasoning_content"
    );
    assert_eq!(resp.model, "glm-5.1");
    let usage = resp.usage.as_ref().unwrap();
    assert_eq!(usage.prompt_tokens, 20);
    assert_eq!(usage.completion_tokens, 30);
    assert_eq!(usage.total_tokens, 50);
}

// --- Error mapping tests ---

#[test]
fn test_glm_error_invalid_model() {
    let json = include_str!(
        "../../../../tests/fixtures/llm/glm/glm-error-invalid-model.json"
    );
    let resp: GlmResponse = serde_json::from_str(json).unwrap();
    let err_body = resp.error.as_ref().unwrap();
    assert_eq!(err_body.code, "1211");
    let err =
        GlmProvider::map_glm_error(&err_body.code, &err_body.message);
    match err {
        ProviderError::Legacy(msg) => {
            assert!(msg.contains("1211"), "should contain 1211");
        }
        other => panic!("Expected Legacy error, got: {:?}", other),
    }
}

#[test]
fn test_glm_error_empty_messages() {
    let json = include_str!(
        "../../../../tests/fixtures/llm/glm/glm-error-empty-messages.json"
    );
    let resp: GlmResponse = serde_json::from_str(json).unwrap();
    let err_body = resp.error.as_ref().unwrap();
    assert_eq!(err_body.code, "1214");
    let err =
        GlmProvider::map_glm_error(&err_body.code, &err_body.message);
    match err {
        ProviderError::Legacy(msg) => {
            assert!(msg.contains("1214"), "should contain 1214");
        }
        other => panic!("Expected Legacy error, got: {:?}", other),
    }
}

#[test]
fn test_glm_error_unknown_code() {
    let err =
        GlmProvider::map_glm_error("9999", "some unknown error");
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

#[test]
fn test_glm_5_1_reasoning_tokens_details() {
    let json = include_str!(
        "../../../../tests/fixtures/llm/glm/glm-5.1-reasoning.json"
    );
    let resp: GlmResponse = serde_json::from_str(json).unwrap();
    let usage = resp.usage.as_ref().unwrap();
    assert_eq!(usage.completion_tokens, 200);
    assert_eq!(usage.prompt_tokens, 17);
    let details =
        usage.completion_tokens_details.as_ref().unwrap();
    assert_eq!(details.reasoning_tokens, Some(200));
    let prompt_details =
        usage.prompt_tokens_details.as_ref().unwrap();
    assert_eq!(prompt_details.cached_tokens, Some(0));
}

// --- fetch_model_list mock HTTP tests ---

#[tokio::test]
async fn test_fetch_model_list_success_mock() {
    let mut server = mockito::Server::new_async().await;
    let fixture =
        include_str!("../../../../tests/fixtures/llm/glm/models-list.json");
    let m = server
        .mock("GET", "/api/paas/v4/models")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url(
        "fake-key".into(),
        format!(
            "{}/api/coding/paas/v4/chat/completions",
            server.url()
        ),
    );
    let models = provider.fetch_model_list("fake-key").await.unwrap();

    m.assert_async().await;
    assert!(!models.is_empty(), "expected at least one model");
    let glm_5_1 =
        models.iter().find(|m| m.id == "glm-5.1").unwrap();
    assert!(
        glm_5_1.reasoning,
        "glm-5.1 should be marked as reasoning"
    );
    let glm_4_7 =
        models.iter().find(|m| m.id == "glm-4.7").unwrap();
    assert!(
        glm_4_7.reasoning,
        "glm-4.7 should be marked as reasoning"
    );
    let glm_4_5 =
        models.iter().find(|m| m.id == "glm-4.5-air").unwrap();
    assert!(
        !glm_4_5.reasoning,
        "glm-4.5-air should not be reasoning"
    );
}

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
        .with_body(
            r#"{"error":{"code":"1210","message":"invalid api key"}}"#,
        )
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url(
        "fake-key".into(),
        format!(
            "{}/api/coding/paas/v4/chat/completions",
            server.url()
        ),
    );
    let err =
        provider.fetch_model_list("fake-key").await.unwrap_err();

    m.assert_async().await;
    match err {
        LLMError::ApiError(msg) => {
            assert!(msg.contains("401"), "should contain 401");
        }
        other => panic!("Expected ApiError, got: {:?}", other),
    }
}
