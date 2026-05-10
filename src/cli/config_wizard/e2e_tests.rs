//! E2E integration tests for config wizard HTTP fetching.
//!
//! Uses mockito to mock the provider's HTTP endpoint and verifies that
//! `fetch_models_with_retry` handles success, transient retry, and auth fallback
//! correctly.

use super::*;
use async_trait::async_trait;
use mockito::{Matcher, Server};
use std::sync::Arc;

/// Test provider that sends real HTTP requests to a mockito server.
struct MockHttpProvider {
    name: String,
    server_url: String,
}

impl MockHttpProvider {
    fn new(name: &str, server_url: String) -> Self {
        Self {
            name: name.to_string(),
            server_url,
        }
    }
}

#[async_trait]
impl LLMProvider for MockHttpProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(
        &self,
        _request: crate::llm::ChatRequest,
    ) -> Result<crate::llm::ChatResponse, LLMError> {
        Err(LLMError::ApiError("mock not implemented".to_string()))
    }

    async fn fetch_model_list(&self, bearer_token: &str) -> Result<Vec<ModelInfo>, LLMError> {
        let url = format!("{}/models", self.server_url);
        let client = reqwest::Client::new();
        let resp = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", bearer_token))
            .send()
            .await
            .map_err(|e| LLMError::NetworkError(e.to_string()))?;

        let status = resp.status().as_u16();
        if status == 401 {
            return Err(LLMError::AuthFailed("HTTP 401".to_string()));
        }
        if status == 500 {
            return Err(LLMError::ApiError("HTTP 500".to_string()));
        }
        if !resp.status().is_success() {
            return Err(LLMError::ApiError(format!("HTTP {}", status)));
        }

        #[derive(serde::Deserialize)]
        struct ApiResponse {
            data: Vec<ModelData>,
        }
        #[derive(serde::Deserialize)]
        struct ModelData {
            id: String,
            name: String,
            context_window: Option<u32>,
            max_tokens: Option<u32>,
        }

        let api_resp: ApiResponse = resp
            .json()
            .await
            .map_err(|e| LLMError::ApiError(format!("failed to parse response: {}", e)))?;

        Ok(api_resp
            .data
            .into_iter()
            .map(|m| ModelInfo {
                id: m.id,
                name: m.name,
                context_window: m.context_window.unwrap_or(4096),
                max_tokens: m.max_tokens.unwrap_or(4096),
                default_temperature: None,
                reasoning: false,
                input_types: vec![],
            })
            .collect())
    }

    fn models(&self) -> Vec<&str> {
        vec![]
    }
}

#[cfg(test)]
mod e2e_fetch_tests {
    use super::*;

    /// Scenario 1: Mock HTTP 200 → fetch_models_with_retry returns model list.
    #[tokio::test]
    async fn test_http_200_success() {
        let mut server = Server::new_async().await;
        let url = format!("http://{}", server.socket_address());
        let _mock = server
            .mock("GET", "/models")
            .match_header("authorization", Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":[{"id":"gpt-4","name":"GPT-4","context_window":8192,"max_tokens":8192},{"id":"gpt-3.5-turbo","name":"GPT-3.5 Turbo","context_window":4096,"max_tokens":4096}]}"#,
            )
            .create_async()
            .await;

        let provider: Arc<dyn LLMProvider> = Arc::new(MockHttpProvider::new("test", url));
        let result = fetch_models_with_retry(&provider, "test-token").await;

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id, "gpt-4");
        assert_eq!(result[1].id, "gpt-3.5-turbo");
    }

    /// Scenario 2: Mock HTTP 500 → Transient → retry → 200 success.
    #[tokio::test]
    async fn test_http_500_then_retry_success() {
        let mut server = Server::new_async().await;
        let url = format!("http://{}", server.socket_address());

        // First request: 500 error
        let _mock1 = server
            .mock("GET", "/models")
            .match_header(
                "authorization",
                Matcher::Regex(r"Bearer test-token".to_string()),
            )
            .with_status(500)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error":"internal server error"}"#)
            .create_async()
            .await;

        // Second request: success
        let _mock2 = server
            .mock("GET", "/models")
            .match_header("authorization", Matcher::Regex(r"Bearer test-token".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":[{"id":"gpt-4","name":"GPT-4","context_window":8192,"max_tokens":8192}]}"#,
            )
            .create_async()
            .await;

        let provider: Arc<dyn LLMProvider> = Arc::new(MockHttpProvider::new("test", url));
        let result = fetch_models_with_retry(&provider, "test-token").await;

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "gpt-4");
    }

    /// Scenario 3: Mock HTTP 401 → Auth error → immediately falls back to knowledge base.
    #[tokio::test]
    async fn test_http_401_immediate_fallback() {
        let mut server = Server::new_async().await;
        let url = format!("http://{}", server.socket_address());

        let _mock = server
            .mock("GET", "/models")
            .with_status(401)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error":"unauthorized"}"#)
            .create_async()
            .await;

        let provider: Arc<dyn LLMProvider> = Arc::new(MockHttpProvider::new("minimax", url));
        let result = fetch_models_with_retry(&provider, "bad-token").await;

        // Falls back to knowledge base — result should contain minimax models
        assert!(
            !result.is_empty(),
            "Fallback should return models from knowledge base"
        );
    }
}
