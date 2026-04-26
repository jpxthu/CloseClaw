use super::*;
use crate::llm::{ChatRequest, Message};
use mockito::Server;

fn provider_url(server: &Server) -> String {
    format!("{}/chat/completions", server.url())
}

fn chat_request(model: &str) -> ChatRequest {
    ChatRequest {
        model: model.to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: "Say hi".to_string(),
        }],
        temperature: 0.0,
        max_tokens: None,
    }
}

fn assert_response_fields(
    resp: &ChatResponse,
    expected_model: &str,
    expected_content_contains: &str,
) {
    assert_eq!(resp.model, expected_model);
    assert!(
        resp.content.contains(expected_content_contains),
        "content should contain '{}', got: {}",
        expected_content_contains,
        resp.content
    );
    assert!(resp.usage.prompt_tokens > 0, "prompt_tokens should be > 0");
    assert!(
        resp.usage.completion_tokens > 0,
        "completion_tokens should be > 0"
    );
    assert!(
        resp.usage.total_tokens >= resp.usage.prompt_tokens + resp.usage.completion_tokens,
        "total_tokens should >= prompt + completion"
    );
}

#[tokio::test]
async fn test_glm_4_7_multi_turn_mock() {
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-4.7-multi-turn.json");
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJson(serde_json::json!({
            "model": "GLM-4.7",
            "messages": [{"role": "user", "content": "Say hi"}],
            "temperature": 0.0
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), provider_url(&server));
    let resp = provider.chat(chat_request("GLM-4.7")).await.unwrap();

    m.assert_async().await;
    assert_response_fields(&resp, "GLM-4.7", "name");
    let json: GlmResponse = serde_json::from_str(fixture).unwrap();
    assert_eq!(json.usage.as_ref().unwrap().prompt_tokens, 20);
    assert_eq!(json.usage.as_ref().unwrap().completion_tokens, 30);
}

#[tokio::test]
async fn test_glm_error_invalid_model_mock() {
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-error-invalid-model.json");
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJson(serde_json::json!({
            "model": "glm-nonexistent",
            "messages": [{"role": "user", "content": "Say hi"}],
            "temperature": 0.0
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), provider_url(&server));
    let err = provider
        .chat(chat_request("glm-nonexistent"))
        .await
        .unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::ModelNotFound(msg) if msg.contains("模型不存在"));
}

#[tokio::test]
async fn test_glm_error_empty_messages_mock() {
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-error-empty-messages.json");
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJson(serde_json::json!({
            "model": "glm-5.1",
            "messages": [{"role": "user", "content": "Say hi"}],
            "temperature": 0.0
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), provider_url(&server));
    let err = provider.chat(chat_request("glm-5.1")).await.unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::InvalidRequest(msg) if msg.contains("输入不能为空"));
}
