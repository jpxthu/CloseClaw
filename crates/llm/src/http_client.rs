//! HTTP client abstraction for LLM providers.
//!
//! Provides `HttpClient` trait to allow mock injection in tests.

use async_trait::async_trait;
use reqwest::Client;

/// Trait for executing HTTP requests.
///
/// Implement this trait to provide custom HTTP client behavior for testing.
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// Execute an HTTP request and receive the response.
    async fn execute(&self, request: reqwest::Request)
        -> Result<reqwest::Response, reqwest::Error>;
}

/// Default HTTP client implementation wrapping `reqwest::Client`.
#[derive(Clone)]
pub struct ReqwestHttpClient {
    client: Client,
}

impl ReqwestHttpClient {
    /// Creates a new `ReqwestHttpClient` with a default client.
    ///
    /// The underlying client uses a 60-second timeout.
    pub fn new() -> Result<Self, reqwest::Error> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok(Self { client })
    }

    /// Creates a `ReqwestHttpClient` from an existing `Client`.
    pub fn with_client(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn execute(
        &self,
        request: reqwest::Request,
    ) -> Result<reqwest::Response, reqwest::Error> {
        self.client.execute(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LLMError, ModelLister};

    // --- ReqwestHttpClient construction tests ---

    #[test]
    fn test_reqwest_http_client_new() {
        let client = ReqwestHttpClient::new();
        assert!(client.is_ok());
    }

    #[test]
    fn test_reqwest_http_client_with_client() {
        let inner = reqwest::Client::new();
        let client = ReqwestHttpClient::with_client(inner);
        let _ = client;
    }

    // --- Test with_http_client injection ---

    #[test]
    fn test_with_http_client_injection() {
        let _provider = crate::MiniMaxProvider::with_http_client(
            "test-key".into(),
            "http://localhost".into(),
            reqwest::Client::new(),
        );
    }

    #[tokio::test]
    async fn test_fetch_model_list_routes_through_trait() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/models")
            .match_header(
                "Authorization",
                mockito::Matcher::Regex(r"Bearer .+".to_string()),
            )
            .with_status(401)
            .with_header("Content-Type", "application/json")
            .with_body(r#"{"error":{"message":"invalid api key"}}"#)
            .create_async()
            .await;

        let provider = crate::MiniMaxProvider::with_http_client(
            "test-key".into(),
            server.url(),
            reqwest::Client::new(),
        );

        // The 401 mock proves the full route: provider -> reqwest::Client ->
        // local mock server -> /v1/models endpoint (routed via the
        // `ModelLister` trait), with the Authorization header passing the
        // matcher, and the 401 response mapped to AuthFailed.
        let err = provider.fetch_model_list("test-key").await.unwrap_err();
        mock.assert_async().await;
        assert!(
            matches!(&err, LLMError::AuthFailed(msg) if msg.contains("401")),
            "expected AuthFailed mentioning 401, got: {err:?}"
        );
    }
}
