//! Model lister tests for DeepSeek Provider.

use super::super::{DeepSeekProvider, LLMError, DEEPSEEK_API_URL};
use crate::llm::provider::ModelLister;

#[tokio::test]
async fn test_fetch_model_list_success() {
    let mut server = mockito::Server::new_async().await;
    let fixture = include_str!("../../../tests/fixtures/llm/deepseek/models-list.json");

    let m = server
        .mock("GET", "/models")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer .+".into()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = DeepSeekProvider::with_base_url("sk-test".into(), server.url());
    let models = provider.fetch_model_list("sk-test").await.unwrap();

    m.assert_async().await;
    assert!(!models.is_empty(), "expected at least one model");
    for model in &models {
        assert!(
            !model.name.to_lowercase().contains("deprecated"),
            "Deprecated model {} should be filtered",
            model.name
        );
    }
}

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
