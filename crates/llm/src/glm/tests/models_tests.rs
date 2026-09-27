//! GLM fetch_model_list mock HTTP tests.

use super::*;
use crate::LLMError;

// --- fetch_model_list mock HTTP tests ---

// TODO: Rewrite with v2 fixture (glm/provider/model-list.json)
// #[tokio::test]
// async fn test_fetch_model_list_success_mock() { ... }

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
        .with_body(r#"{"error":{"code":"1210","message":"invalid api key"}}"#)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url(
        "fake-key".into(),
        Some(format!("{}/api/coding/paas/v4/chat/completions", server.url()).as_str()),
    );
    let err = provider.fetch_model_list("fake-key").await.unwrap_err();

    m.assert_async().await;
    match err {
        LLMError::AuthFailed(msg) => {
            assert!(msg.contains("401"), "should contain 401");
        }
        other => panic!("Expected AuthFailed, got: {:?}", other),
    }
}
