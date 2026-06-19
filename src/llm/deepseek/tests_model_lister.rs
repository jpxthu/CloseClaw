//! Model lister tests for DeepSeek Provider.

use super::*;
use crate::llm::ModelLister;

// TODO: Rewrite with v2 fixture (deepseek/provider/model-list.json)
// #[tokio::test]
// async fn test_fetch_model_list_success() { ... }

#[tokio::test]
async fn test_fetch_model_list_auth_error() {
    let mut server = mockito::Server::new_async().await;

    let m = server
        .mock("GET", "/models")
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"invalid_api_key","message":"invalid"}}"#)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), server.url());
    let err = provider.fetch_model_list("bad-key").await.unwrap_err();

    m.assert_async().await;
    assert!(
        matches!(err, LLMError::AuthFailed(_)),
        "expected AuthFailed, got: {:?}",
        err
    );
}
