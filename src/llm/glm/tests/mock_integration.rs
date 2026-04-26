use super::*;
use crate::llm::{ChatRequest, Message};

/// The GLM provider POSTs to `base_url` directly (no path appended).
/// We construct the base_url to include /chat/completions so the mock registered
/// at that path matches.
fn provider_url(server: &Server) -> String {
    format!("{}/chat/completions", server.url())
}

/// Build a minimal ChatRequest for the given model.
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

/// Verify basic fields that are always present in a successful response.
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

// --- Normal responses (content extracted from reasoning_content) ---

#[tokio::test]
async fn test_glm_5_1_chat_mock() {
    // glm-5.1-chat.json: content empty → reasoning_content extracted
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-chat.json");
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
    let resp = provider.chat(chat_request("glm-5.1")).await.unwrap();

    m.assert_async().await;
    assert_response_fields(&resp, "glm-5.1", "Analyze the Request");
    assert_eq!(resp.usage.prompt_tokens, 11);
    assert_eq!(resp.usage.completion_tokens, 30);
}

#[tokio::test]
async fn test_glm_4_7_simple_chat_mock() {
    // glm-4.7-simple-chat.json: GLM-4.7 model name verification
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-4.7-simple-chat.json");
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
    assert_response_fields(&resp, "GLM-4.7", "three words");
    // Verify cached_tokens from prompt_tokens_details (deserialized separately)
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
    // glm-4.5-air-chat.json: GLM-4.5-Air model
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-4.5-air-chat.json");
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJson(serde_json::json!({
            "model": "GLM-4.5-Air",
            "messages": [{"role": "user", "content": "Say hi"}],
            "temperature": 0.0
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), provider_url(&server));
    let resp = provider.chat(chat_request("GLM-4.5-Air")).await.unwrap();

    m.assert_async().await;
    assert_response_fields(&resp, "GLM-4.5-Air", "three words");
}

#[tokio::test]
async fn test_glm_5_turbo_chat_mock() {
    // glm-5-turbo-chat.json: GLM-5-Turbo model
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-5-turbo-chat.json");
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJson(serde_json::json!({
            "model": "glm-5-turbo",
            "messages": [{"role": "user", "content": "Say hi"}],
            "temperature": 0.0
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), provider_url(&server));
    let resp = provider.chat(chat_request("glm-5-turbo")).await.unwrap();

    m.assert_async().await;
    assert_response_fields(&resp, "glm-5-turbo", "Analyze the Request");
    assert_eq!(resp.usage.prompt_tokens, 11);
    assert_eq!(resp.usage.completion_tokens, 30);
}

#[tokio::test]
async fn test_glm_5_1_reasoning_mock() {
    // glm-5.1-reasoning.json: reasoning_tokens=200
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-reasoning.json");
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
    let resp = provider.chat(chat_request("glm-5.1")).await.unwrap();

    m.assert_async().await;
    assert_response_fields(&resp, "glm-5.1", "Understand the Goal");
    // Verify reasoning_tokens from fixture
    let json: GlmResponse = serde_json::from_str(fixture).unwrap();
    let rt = json.usage.as_ref().and_then(|u| {
        u.completion_tokens_details
            .as_ref()
            .and_then(|d| d.reasoning_tokens)
    });
    assert_eq!(rt, Some(200));
    assert_eq!(resp.usage.completion_tokens, 200);
}

#[tokio::test]
async fn test_glm_5_1_code_generation_mock() {
    // glm-5.1-code-generation.json: long answer, code-gen scenario
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-code-generation.json");
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
    let resp = provider.chat(chat_request("glm-5.1")).await.unwrap();

    m.assert_async().await;
    assert_response_fields(&resp, "glm-5.1", "Hello, World");
    assert_eq!(resp.usage.completion_tokens, 100);
}

#[tokio::test]
async fn test_glm_4_7_math_temp0_mock() {
    // glm-4.7-math-temp0.json: reasoning scenario (math, temp=0)
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-4.7-math-temp0.json");
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
    assert_response_fields(&resp, "GLM-4.7", "arithmetic");
    let json: GlmResponse = serde_json::from_str(fixture).unwrap();
    let rt = json.usage.as_ref().and_then(|u| {
        u.completion_tokens_details
            .as_ref()
            .and_then(|d| d.reasoning_tokens)
    });
    assert_eq!(rt, Some(20));
}

#[tokio::test]
async fn test_glm_5_1_unicode_mock() {
    // glm-5.1-unicode-chat.json: Chinese content
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-unicode-chat.json");
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
    let resp = provider.chat(chat_request("glm-5.1")).await.unwrap();

    m.assert_async().await;
    assert_response_fields(&resp, "glm-5.1", "Spring");
    // Verify Unicode content is preserved
    let json: GlmResponse = serde_json::from_str(fixture).unwrap();
    let content = json
        .choices
        .as_ref()
        .and_then(|c| c.first())
        .map(|ch| &ch.message.content);
    let reasoning = json
        .choices
        .as_ref()
        .and_then(|c| c.first())
        .and_then(|ch| ch.message.reasoning_content.as_ref());
    assert!(reasoning.unwrap().contains("分析请求") || reasoning.unwrap().contains("分析"));
}

#[tokio::test]
async fn test_glm_5_1_temp_1_mock() {
    // glm-5.1-temp-1.0.json: temperature=1.0 scenario
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-temp-1.0.json");
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
    let resp = provider.chat(chat_request("glm-5.1")).await.unwrap();

    m.assert_async().await;
    assert_response_fields(&resp, "glm-5.1", "Analyze");
    assert_eq!(resp.usage.prompt_tokens, 10);
    assert_eq!(resp.usage.completion_tokens, 10);
}

#[tokio::test]
async fn test_glm_5_1_system_prompt_mock() {
    // glm-5.1-system-prompt.json: system prompt scenario
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-system-prompt.json");
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
    let resp = provider.chat(chat_request("glm-5.1")).await.unwrap();

    m.assert_async().await;
    assert_response_fields(&resp, "glm-5.1", "Analyze");
    // System prompt generally results in higher prompt_tokens
    let json: GlmResponse = serde_json::from_str(fixture).unwrap();
    assert_eq!(json.usage.as_ref().unwrap().prompt_tokens, 18);
}

#[tokio::test]
async fn test_glm_5_1_multi_turn_mock() {
    // glm-5.1-multi-turn.json: multi-turn conversation
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-5.1-multi-turn.json");
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
    let resp = provider.chat(chat_request("glm-5.1")).await.unwrap();

    m.assert_async().await;
    assert_response_fields(&resp, "glm-5.1", "Analyze");
    let json: GlmResponse = serde_json::from_str(fixture).unwrap();
    assert_eq!(json.usage.as_ref().unwrap().prompt_tokens, 20);
    assert_eq!(json.usage.as_ref().unwrap().completion_tokens, 30);
}

#[tokio::test]
async fn test_glm_4_7_long_response_mock() {
    // glm-4.7-long-response.json: long answer (300 completion tokens)
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-4.7-long-response.json");
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
    assert_response_fields(&resp, "GLM-4.7", "狗");
    assert_eq!(resp.usage.completion_tokens, 300);
    let json: GlmResponse = serde_json::from_str(fixture).unwrap();
    let rt = json.usage.as_ref().and_then(|u| {
        u.completion_tokens_details
            .as_ref()
            .and_then(|d| d.reasoning_tokens)
    });
    assert_eq!(rt, Some(295));
}

#[tokio::test]
async fn test_glm_4_7_short_max_tokens_mock() {
    // glm-4.7-short-max-tokens.json: finish_reason=length (short due to max_tokens)
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-4.7-short-max-tokens.json");
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
    assert_response_fields(&resp, "GLM-4.7", "joke");
    // Short response with max_tokens limit
    assert!(resp.usage.completion_tokens <= 10);
}
