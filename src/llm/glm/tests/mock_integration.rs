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

// ============================================================================
// TODO: All fixture-dependent tests have been deprecated.
// Rewrite with v2 fixtures at tests/fixtures/llm/glm/{model}/openai/ and
// tests/fixtures/llm/glm/{model}/anthropic/
// ============================================================================

// #[tokio::test] async fn test_glm_5_1_chat_mock() { ... }
// #[tokio::test] async fn test_glm_4_7_simple_chat_mock() { ... }
// #[tokio::test] async fn test_glm_4_5_air_chat_mock() { ... }
// #[tokio::test] async fn test_glm_5_turbo_chat_mock() { ... }
// #[tokio::test] async fn test_glm_5_1_reasoning_mock() { ... }
// #[tokio::test] async fn test_glm_5_1_code_generation_mock() { ... }
// #[tokio::test] async fn test_glm_4_7_math_temp0_mock() { ... }
// #[tokio::test] async fn test_glm_5_1_unicode_mock() { ... }
// #[tokio::test] async fn test_glm_5_1_temp_1_mock() { ... }
// #[tokio::test] async fn test_glm_5_1_system_prompt_mock() { ... }
// #[tokio::test] async fn test_glm_5_1_multi_turn_mock() { ... }
// #[tokio::test] async fn test_glm_4_7_long_response_mock() { ... }
// #[tokio::test] async fn test_glm_4_7_short_max_tokens_mock() { ... }
// #[tokio::test] async fn test_glm_error_invalid_model_mock() { ... }
// #[tokio::test] async fn test_glm_error_empty_messages_mock() { ... }

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
