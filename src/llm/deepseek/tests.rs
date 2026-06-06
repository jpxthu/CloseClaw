//! Unit tests for the DeepSeek provider.

use super::*;
use mockito::Server;

// ---------------------------------------------------------------------------
// Helper utilities
// ---------------------------------------------------------------------------

fn provider_url(server: &mockito::Server) -> String {
    format!("http://{}", server.address())
}

fn chat_request(model: &str) -> LLMRequest {
    LLMRequest {
        model: model.to_string(),
        messages: vec![LLMMessage::user("Hello")],
        temperature: None,
        top_p: None,
        max_tokens: None,
        stream: false,
        stop: None,
    }
}

// ---------------------------------------------------------------------------
// Chat integration tests via mockito
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_chat_success_mock() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../tests/fixtures/llm/deepseek/chat-success.json");

    let m = server
        .mock("POST", "/chat/completions")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer sk-.*".to_string()),
        )
        .match_header("content-type", "application/json")
        .match_header("accept", "application/json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), provider_url(&server));
    let result = provider.chat(chat_request("deepseek-v4-flash")).await;

    m.assert_async().await;
    assert!(result.is_ok(), "expected ok, got {:?}", result);
}

#[tokio::test]
async fn test_chat_error_not_found_mock() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../tests/fixtures/llm/deepseek/error-not-found.json");

    let m = server
        .mock("POST", "/chat/completions")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .match_header("content-type", "application/json")
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("fake-key".into(), provider_url(&server));
    let err = provider
        .chat(chat_request("unknown-model"))
        .await
        .unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::ModelNotFound(_));
}

#[tokio::test]
async fn test_chat_error_auth_mock() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../tests/fixtures/llm/deepseek/error-auth.json");

    let m = server
        .mock("POST", "/chat/completions")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .match_header("content-type", "application/json")
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("fake-key".into(), provider_url(&server));
    let err = provider
        .chat(chat_request("deepseek-v4-flash"))
        .await
        .unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::AuthFailed(_));
}

#[tokio::test]
async fn test_chat_rate_limit_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::Any)
        .with_status(429)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"rate_limit_exceeded","message":"rate limit exceeded"}}"#)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("fake-key".into(), provider_url(&server));
    let err = provider
        .chat(chat_request("deepseek-v4-flash"))
        .await
        .unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::RateLimitExceeded);
}

// ---------------------------------------------------------------------------
// fetch_model_list() tests via mockito
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_fetch_model_list_success_mock() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../tests/fixtures/llm/deepseek/models-list.json");

    let m = server
        .mock("GET", "/models")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("fake-key".into(), server.url());
    let models = provider.fetch_model_list("fake-key").await.unwrap();

    m.assert_async().await;
    assert!(!models.is_empty(), "expected at least one model");
    // Verify deprecated models are filtered out
    for model in &models {
        assert!(
            !model.name.to_lowercase().contains("deprecated"),
            "Deprecated model {} should be filtered",
            model.name
        );
    }
}

#[tokio::test]
async fn test_fetch_model_list_http_error_mock() {
    let mut server = mockito::Server::new_async().await;

    let m = server
        .mock("GET", "/models")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"invalid_api_key","message":"invalid api key"}}"#)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("fake-key".into(), server.url());
    let err = provider.fetch_model_list("fake-key").await.unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::AuthFailed(_));
}

#[tokio::test]
async fn test_fetch_model_list_timeout_mock() {
    let mut server = mockito::Server::new_async().await;

    // Delay response by 500ms — client timeout is 1ms, so request will timeout reliably
    let m = server
        .mock("GET", "/models")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"data":[]}"#)
        .with_delay(500)
        .create_async()
        .await;

    // Use a client with 1ms timeout to trigger NetworkError
    let short_timeout_client = Client::builder()
        .timeout(Duration::from_millis(1))
        .build()
        .unwrap();
    let provider = DeepSeekProvider::with_base_url("fake-key".into(), server.url());
    // Replace http_client with short-timeout client
    let provider = DeepSeekProviderWithCustomClient {
        provider,
        client: short_timeout_client,
    };
    let err = provider.fetch_model_list("fake-key").await.unwrap_err();

    m.assert_async().await;
    matches!(err, LLMError::NetworkError(_));
}

// Helper to inject a custom client for timeout testing
struct DeepSeekProviderWithCustomClient {
    provider: DeepSeekProvider,
    client: Client,
}

impl DeepSeekProviderWithCustomClient {
    async fn fetch_model_list(&self, bearer_token: &str) -> Result<Vec<ModelInfo>, LLMError> {
        let url = format!("{}/models", self.provider.base_url.trim_end_matches('/'));
        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", bearer_token))
            .send()
            .await
            .map_err(|e| LLMError::NetworkError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(DeepSeekProvider::map_http_error(status, &body));
        }

        let api_resp: DeepSeekModelsResponse = response.json().await.map_err(|e| {
            LLMError::ApiError(format!("failed to parse DeepSeek /models response: {}", e))
        })?;

        let models: Vec<ModelInfo> = api_resp
            .data
            .into_iter()
            .filter(|m| {
                m.status
                    .as_ref()
                    .map(|s| {
                        !s.eq_ignore_ascii_case("deprecated") && !s.eq_ignore_ascii_case("shutdown")
                    })
                    .unwrap_or(true)
            })
            .map(|m| {
                let input_types: Vec<crate::llm::InputType> = m
                    .input_modalities
                    .iter()
                    .filter_map(|m| match m.to_lowercase().as_str() {
                        "image" => Some(crate::llm::InputType::Image),
                        _ => Some(crate::llm::InputType::Text),
                    })
                    .collect();
                let input_types = if input_types.is_empty() {
                    vec![crate::llm::InputType::Text]
                } else {
                    input_types
                };
                ModelInfo {
                    id: m.id.clone(),
                    name: m.display_name.clone().unwrap_or_else(|| m.id.clone()),
                    context_window: m.context_window.unwrap_or(64_000),
                    max_tokens: m.max_output_tokens.unwrap_or(8_192),
                    default_temperature: None,
                    reasoning: false,
                    input_types,
                }
            })
            .collect();

        Ok(models)
    }
}

// -------------------------------------------------------------------------
// name / provider_display_name / models unit tests
// -------------------------------------------------------------------------

#[test]
fn test_name() {
    let provider = DeepSeekProvider::new("fake-key".into());
    assert_eq!(provider.name(), "deepseek");
}

#[test]
fn test_provider_display_name() {
    let provider = DeepSeekProvider::new("fake-key".into());
    assert_eq!(provider.provider_display_name(), "DeepSeek");
}

#[test]
fn test_models() {
    let provider = DeepSeekProvider::new("fake-key".into());
    let models = provider.models();
    assert!(!models.is_empty());
    assert!(models.contains(&"deepseek-v4-flash"));
}
