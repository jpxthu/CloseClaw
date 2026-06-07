use super::*;
use crate::llm::types::InternalMessage;
use mockito::Server;

fn provider_url(server: &Server) -> String {
    format!("{}/chat/completions", server.url())
}

fn internal_request(model: &str) -> InternalRequest {
    InternalRequest {
        model: model.to_string(),
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: "Say hi".to_string(),
        }],
        temperature: 0.0,
        max_tokens: None,
        stream: false,
        extra_body: serde_json::Map::new(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        session_id: None,
        reasoning_level: crate::session::persistence::ReasoningLevel::default(),
        turn_count: None,
    }
}

fn chat_body(model: &str) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "Say hi"}],
        "temperature": 0.0
    })
}

fn assert_first_text(resp: &InternalResponse, expected: &str) {
    assert!(!resp.content_blocks.is_empty());
    match &resp.content_blocks[0] {
        RawContentBlock::Text(s) => {
            assert!(s.contains(expected), "expected '{}', got: {}", expected, s)
        }
        RawContentBlock::Thinking(s) => {
            assert!(s.contains(expected), "expected '{}', got: {}", expected, s)
        }
        other => panic!("Expected Text/Thinking, got: {:?}", other),
    }
}

/// Helper: set up mockito server, mock POST, create provider, call send.
async fn send_with_mock(model: &str, fixture: &str) -> InternalResponse {
    let mut server = Server::new_async().await;
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJson(chat_body(model)))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), provider_url(&server));
    let resp = provider
        .send(internal_request(model), chat_body(model))
        .await
        .unwrap();
    m.assert_async().await;
    resp
}

/// Helper: send and expect an error.
async fn send_error_with_mock(model: &str, fixture: &str) -> ProviderError {
    let mut server = Server::new_async().await;
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJson(chat_body(model)))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), provider_url(&server));
    let err = provider
        .send(internal_request(model), chat_body(model))
        .await
        .unwrap_err();
    m.assert_async().await;
    err
}

// --- Normal responses (content extracted from reasoning_content) ---

#[tokio::test]
async fn test_glm_5_1_chat_mock() {
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-chat.json");
    let resp = send_with_mock("glm-5.1", fixture).await;
    assert_first_text(&resp, "Analyze the Request");
    assert_eq!(resp.usage.prompt_tokens, 11);
    assert_eq!(resp.usage.completion_tokens, 30);
}

#[tokio::test]
async fn test_glm_4_7_simple_chat_mock() {
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-4.7-simple-chat.json");
    let resp = send_with_mock("GLM-4.7", fixture).await;
    assert_first_text(&resp, "three words");
    let json: GlmResponse = serde_json::from_str(fixture).unwrap();
    let cached = json.usage.as_ref().and_then(|u| {
        u.prompt_tokens_details
            .as_ref()
            .and_then(|p| p.cached_tokens)
    });
    assert_eq!(cached, Some(10));
}

#[tokio::test]
async fn test_glm_4_5_air_chat_mock() {
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-4.5-air-chat.json");
    let resp = send_with_mock("GLM-4.5-Air", fixture).await;
    assert_first_text(&resp, "three words");
}

#[tokio::test]
async fn test_glm_5_turbo_chat_mock() {
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-5-turbo-chat.json");
    let resp = send_with_mock("glm-5-turbo", fixture).await;
    assert_first_text(&resp, "Analyze the Request");
    assert_eq!(resp.usage.prompt_tokens, 11);
    assert_eq!(resp.usage.completion_tokens, 30);
}

#[tokio::test]
async fn test_glm_5_1_reasoning_mock() {
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-reasoning.json");
    let resp = send_with_mock("glm-5.1", fixture).await;
    assert_first_text(&resp, "Understand the Goal");
    assert_eq!(resp.usage.completion_tokens, 200);
}

#[tokio::test]
async fn test_glm_5_1_code_generation_mock() {
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-code-generation.json");
    let resp = send_with_mock("glm-5.1", fixture).await;
    assert_first_text(&resp, "Hello, World");
    assert_eq!(resp.usage.completion_tokens, 100);
}

#[tokio::test]
async fn test_glm_4_7_math_temp0_mock() {
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-4.7-math-temp0.json");
    let resp = send_with_mock("GLM-4.7", fixture).await;
    assert_first_text(&resp, "arithmetic");
}

#[tokio::test]
async fn test_glm_5_1_unicode_mock() {
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-unicode-chat.json");
    let resp = send_with_mock("glm-5.1", fixture).await;
    assert_first_text(&resp, "Spring");
    let json: GlmResponse = serde_json::from_str(fixture).unwrap();
    let reasoning = json
        .choices
        .as_ref()
        .and_then(|c| c.first())
        .and_then(|ch| ch.message.reasoning_content.as_ref());
    assert!(reasoning.unwrap().contains("分析请求") || reasoning.unwrap().contains("分析"));
}

#[tokio::test]
async fn test_glm_5_1_temp_1_mock() {
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-temp-1.0.json");
    let resp = send_with_mock("glm-5.1", fixture).await;
    assert_first_text(&resp, "Analyze");
    assert_eq!(resp.usage.prompt_tokens, 10);
    assert_eq!(resp.usage.completion_tokens, 10);
}

#[tokio::test]
async fn test_glm_5_1_system_prompt_mock() {
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-system-prompt.json");
    let resp = send_with_mock("glm-5.1", fixture).await;
    assert_first_text(&resp, "Analyze");
    let json: GlmResponse = serde_json::from_str(fixture).unwrap();
    assert_eq!(json.usage.as_ref().unwrap().prompt_tokens, 18);
}

#[tokio::test]
async fn test_glm_5_1_multi_turn_mock() {
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-multi-turn.json");
    let resp = send_with_mock("glm-5.1", fixture).await;
    assert_first_text(&resp, "Analyze");
    let json: GlmResponse = serde_json::from_str(fixture).unwrap();
    assert_eq!(json.usage.as_ref().unwrap().prompt_tokens, 20);
    assert_eq!(json.usage.as_ref().unwrap().completion_tokens, 30);
}

#[tokio::test]
async fn test_glm_4_7_long_response_mock() {
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-4.7-long-response.json");
    let resp = send_with_mock("GLM-4.7", fixture).await;
    assert_first_text(&resp, "狗");
    assert_eq!(resp.usage.completion_tokens, 300);
}

#[tokio::test]
async fn test_glm_4_7_short_max_tokens_mock() {
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-4.7-short-max-tokens.json");
    let resp = send_with_mock("GLM-4.7", fixture).await;
    assert_first_text(&resp, "joke");
    assert!(resp.usage.completion_tokens <= 10);
}

// --- Error responses (GLM error codes in body, HTTP 200) ---

#[tokio::test]
async fn test_glm_error_invalid_model_mock() {
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-error-invalid-model.json");
    let err = send_error_with_mock("glm-nonexistent", fixture).await;
    match err {
        ProviderError::Legacy(msg) => {
            assert!(msg.contains("1211"), "should contain 1211");
        }
        other => panic!("Expected Legacy, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_glm_error_empty_messages_mock() {
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-error-empty-messages.json");
    let err = send_error_with_mock("glm-5.1", fixture).await;
    match err {
        ProviderError::Legacy(msg) => {
            assert!(msg.contains("1214"), "should contain 1214");
        }
        other => panic!("Expected Legacy, got: {:?}", other),
    }
}

// --- HTTP error responses (non-200 status) ---

#[tokio::test]
async fn test_glm_http_500_error_mock() {
    let mut server = Server::new_async().await;
    let m = server
        .mock("POST", "/chat/completions")
        .with_status(500)
        .with_body("internal server error")
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), provider_url(&server));
    let err = provider
        .send(internal_request("glm-5.1"), chat_body("glm-5.1"))
        .await
        .unwrap_err();
    m.assert_async().await;
    match err {
        ProviderError::Legacy(msg) => {
            assert!(msg.contains("500"), "should contain 500");
        }
        other => panic!("Expected Legacy, got: {:?}", other),
    }
}
