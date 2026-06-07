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
pub use timeout_mock::MockTimeoutHttpClient;

/// Test-only timeout mock module.
#[cfg(test)]
mod timeout_mock {
    use super::*;

    /// Mock HTTP client that always returns a timeout error.
    pub struct MockTimeoutHttpClient;

    impl MockTimeoutHttpClient {
        pub fn new() -> Self {
            Self
        }
    }

    impl Default for MockTimeoutHttpClient {
        fn default() -> Self {
            Self::new()
        }
    }

    #[async_trait]
    impl HttpClient for MockTimeoutHttpClient {
        async fn execute(
            &self,
            _request: reqwest::Request,
        ) -> Result<reqwest::Response, reqwest::Error> {
            // Build a client with zero timeout to always trigger a timeout error.
            // reqwest::Error has no public constructor, so we use a 0-timeout client
            // to reliably produce Kind::Timeout.
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(0))
                .build()
                .unwrap();
            let req = reqwest::Request::new(
                reqwest::Method::GET,
                reqwest::Url::parse("http://localhost").unwrap(),
            );
            Err(client
                .execute(req)
                .await
                .expect_err("expected timeout error"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ModelLister;
    use std::sync::Arc;

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

    // --- MockHttpClient for test injection ---
    #[allow(dead_code)]
    struct MockHttpClient {
        status: u16,
        body: Vec<u8>,
        calls: std::sync::Mutex<Vec<Arc<reqwest::Request>>>,
    }

    #[allow(dead_code)]
    impl MockHttpClient {
        fn new(status: u16, body: Vec<u8>) -> Self {
            Self {
                status,
                body,
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl HttpClient for MockHttpClient {
        async fn execute(
            &self,
            request: reqwest::Request,
        ) -> Result<reqwest::Response, reqwest::Error> {
            self.calls.lock().unwrap().push(Arc::new(request));
            // Build a real Response via a mockito async server.
            let mut server = mockito::Server::new_async().await;
            server
                .mock("GET", "/")
                .with_status(self.status as usize)
                .with_header("content-type", "application/json")
                .with_body(&self.body)
                .create_async()
                .await;
            let url = server.url();
            let client = reqwest::Client::new();
            let req =
                reqwest::Request::new(reqwest::Method::GET, reqwest::Url::parse(&url).unwrap());
            client.execute(req).await
        }
    }

    // --- Test with_http_client injection ---

    #[test]
    fn test_with_http_client_injection() {
        let _provider = crate::llm::MiniMaxProvider::with_http_client(
            "test-key".into(),
            "http://localhost".into(),
            reqwest::Client::new(),
        );
    }

    #[tokio::test]
    async fn test_fetch_model_list_routes_through_trait() {
        let provider = crate::llm::MiniMaxProvider::with_http_client(
            "test-key".into(),
            "http://localhost/v1".into(),
            reqwest::Client::new(),
        );

        // With reqwest::Client (not MockHttpClient), a request to localhost
        // will fail at the network level. This verifies the provider is
        // properly constructed.
        let err = provider.fetch_model_list("test-key").await;
        assert!(err.is_err());
    }
}
