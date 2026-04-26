//! Unit tests for the GLM provider.

use super::*;
use mockito::Server;

// ---------------------------------------------------------------------------//
// mock_integration — HTTP-level mock tests covering the full chat() pipeline  //
// ---------------------------------------------------------------------------//
mod mock_extra;
mod mock_integration;

// --- Fixture-based deserialization and content extraction tests ---

#[test]
fn test_glm_5_1_chat_extract_reasoning() {
    // glm-5.1-chat.json: content empty → extract reasoning_content
    let json = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-chat.json");
    let resp: GlmResponse = serde_json::from_str(json).unwrap();
    let choice = resp.choices.as_ref().and_then(|c| c.first()).unwrap();
    let msg = &choice.message;
    let extracted = GlmProvider::extract_content(msg);
    assert!(
        !msg.content.trim().is_empty() == false,
        "content should be empty/whitespace in this fixture"
    );
    assert!(
        msg.reasoning_content.is_some() && !msg.reasoning_content.as_ref().unwrap().is_empty(),
        "reasoning_content should be non-empty in this fixture"
    );
    assert!(
        !extracted.is_empty(),
        "should extract from reasoning_content"
    );
    assert_eq!(extracted, msg.reasoning_content.as_ref().unwrap().trim());
    // Verify usage fields
    let usage = resp.usage.as_ref().unwrap();
    assert_eq!(usage.completion_tokens, 30);
    assert_eq!(usage.prompt_tokens, 11);
    assert_eq!(usage.total_tokens, 41);
    let details = usage.completion_tokens_details.as_ref().unwrap();
    assert_eq!(details.reasoning_tokens, Some(30));
    let prompt_details = usage.prompt_tokens_details.as_ref().unwrap();
    assert_eq!(prompt_details.cached_tokens, Some(0));
}

#[test]
fn test_glm_4_7_simple_chat_extract_reasoning() {
    // glm-4.7-simple-chat.json: content empty → extract reasoning_content
    let json = include_str!("../../../../tests/fixtures/llm/glm/glm-4.7-simple-chat.json");
    let resp: GlmResponse = serde_json::from_str(json).unwrap();
    let choice = resp.choices.as_ref().and_then(|c| c.first()).unwrap();
    let msg = &choice.message;
    let extracted = GlmProvider::extract_content(msg);
    assert!(msg.content.trim().is_empty(), "content should be empty");
    assert!(
        msg.reasoning_content.is_some() && !msg.reasoning_content.as_ref().unwrap().is_empty(),
        "reasoning_content should be non-empty"
    );
    assert!(
        !extracted.is_empty(),
        "should extract from reasoning_content"
    );
    assert_eq!(extracted, msg.reasoning_content.as_ref().unwrap().trim());
    // GLM-4.7 model name
    assert_eq!(resp.model, "GLM-4.7");
    // Verify cached_tokens in prompt_tokens_details
    let usage = resp.usage.as_ref().unwrap();
    let prompt_details = usage.prompt_tokens_details.as_ref().unwrap();
    assert_eq!(prompt_details.cached_tokens, Some(10));
}

#[test]
fn test_glm_4_5_air_chat_extract_reasoning() {
    // glm-4.5-air-chat.json: AIR model, content empty → extract reasoning_content
    let json = include_str!("../../../../tests/fixtures/llm/glm/glm-4.5-air-chat.json");
    let resp: GlmResponse = serde_json::from_str(json).unwrap();
    let choice = resp.choices.as_ref().and_then(|c| c.first()).unwrap();
    let msg = &choice.message;
    let extracted = GlmProvider::extract_content(msg);
    assert!(msg.content.trim().is_empty(), "content should be empty");
    assert!(
        msg.reasoning_content.is_some() && !msg.reasoning_content.as_ref().unwrap().is_empty(),
        "reasoning_content should be non-empty"
    );
    assert!(
        !extracted.is_empty(),
        "should extract from reasoning_content"
    );
    assert_eq!(resp.model, "GLM-4.5-Air");
}

#[test]
fn test_glm_5_1_multi_turn() {
    // glm-5.1-multi-turn.json: multi-turn conversation parsing
    let json = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-multi-turn.json");
    let resp: GlmResponse = serde_json::from_str(json).unwrap();
    let choice = resp.choices.as_ref().and_then(|c| c.first()).unwrap();
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
    // glm-error-invalid-model.json: code="1211" → ModelNotFound
    let json = include_str!("../../../../tests/fixtures/llm/glm/glm-error-invalid-model.json");
    let resp: GlmResponse = serde_json::from_str(json).unwrap();
    let err_body = resp.error.as_ref().unwrap();
    assert_eq!(err_body.code, "1211");
    let err = GlmProvider::map_glm_error(&err_body.code, &err_body.message);
    matches!(err, LLMError::ModelNotFound(msg) if msg.contains("模型不存在"));
}

#[test]
fn test_glm_error_empty_messages() {
    // glm-error-empty-messages.json: code="1214" → InvalidRequest
    let json = include_str!("../../../../tests/fixtures/llm/glm/glm-error-empty-messages.json");
    let resp: GlmResponse = serde_json::from_str(json).unwrap();
    let err_body = resp.error.as_ref().unwrap();
    assert_eq!(err_body.code, "1214");
    let err = GlmProvider::map_glm_error(&err_body.code, &err_body.message);
    matches!(err, LLMError::InvalidRequest(msg) if msg.contains("输入不能为空"));
}

#[test]
fn test_glm_error_unknown_code() {
    // Unknown code maps to ApiError
    let err = GlmProvider::map_glm_error("9999", "some unknown error");
    matches!(err, LLMError::ApiError(msg) if msg.contains("9999"));
}

// --- extract_content edge cases ---

#[test]
fn test_extract_content_prefers_non_empty_content() {
    // When content is non-empty, reasoning_content should be ignored
    let msg = GlmMessage {
        role: "assistant".to_string(),
        content: "Hello, World!".to_string(),
        reasoning_content: Some("I am thinking...".to_string()),
    };
    let extracted = GlmProvider::extract_content(&msg);
    assert_eq!(extracted, "Hello, World!");
}

#[test]
fn test_extract_content_falls_back_to_reasoning_when_content_empty() {
    // content empty/whitespace → fall back to reasoning_content
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
    // content empty, reasoning_content whitespace-only → returns empty
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
    // content empty, no reasoning_content → returns empty
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
    // glm-5.1-reasoning.json: verify completion_tokens_details and prompt_tokens_details
    let json = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-reasoning.json");
    let resp: GlmResponse = serde_json::from_str(json).unwrap();
    let usage = resp.usage.as_ref().unwrap();
    assert_eq!(usage.completion_tokens, 200);
    assert_eq!(usage.prompt_tokens, 17);
    let details = usage.completion_tokens_details.as_ref().unwrap();
    assert_eq!(details.reasoning_tokens, Some(200));
    let prompt_details = usage.prompt_tokens_details.as_ref().unwrap();
    assert_eq!(prompt_details.cached_tokens, Some(0));
}
