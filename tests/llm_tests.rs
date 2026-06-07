//! LLM Integration Tests
//!
//! Tests the StubProvider and LLMRegistry integration.
//! These tests verify that the agent+LLM call chain works in CI.

use std::sync::Arc;

use closeclaw::llm::provider::Provider;
use closeclaw::llm::types::{InternalMessage, InternalRequest, RawContentBlock};
use closeclaw::llm::{LLMRegistry, StubProvider};
use closeclaw::session::ReasoningLevel;

fn stub_provider() -> Arc<dyn Provider> {
    Arc::new(StubProvider::new())
}

#[tokio::test]
async fn test_stub_provider_is_stub() {
    let provider = StubProvider::new();
    assert_eq!(provider.id(), "stub", "StubProvider id should be 'stub'");
}

#[tokio::test]
async fn test_stub_provider_chat_returns_fixed_response() {
    let provider = StubProvider::new();
    let request = InternalRequest {
        model: "gpt-4".to_string(),
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: "Hello, world!".to_string(),
        }],
        temperature: 0.7,
        max_tokens: None,
        stream: false,
        extra_body: serde_json::Map::new(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        session_id: None,
        reasoning_level: ReasoningLevel::default(),
        turn_count: None,
    };
    let body = serde_json::to_value(&request).unwrap();

    let response = provider.send(request, body).await.unwrap();
    assert_eq!(response.content_blocks.len(), 1);
    match &response.content_blocks[0] {
        RawContentBlock::Text(s) => {
            assert_eq!(s, "stub response");
        }
        _ => panic!("Expected Text content block"),
    }
    assert!(response.usage.total_tokens.is_some());
    assert!(response.usage.total_tokens.unwrap() > 0);
}

#[tokio::test]
async fn test_stub_provider_custom_response() {
    let provider = StubProvider::with_response("custom test response");
    let request = InternalRequest {
        model: "claude-3".to_string(),
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: "test input".to_string(),
        }],
        temperature: 0.0,
        max_tokens: Some(100),
        stream: false,
        extra_body: serde_json::Map::new(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        session_id: None,
        reasoning_level: ReasoningLevel::default(),
        turn_count: None,
    };
    let body = serde_json::to_value(&request).unwrap();

    let response = provider.send(request, body).await.unwrap();
    assert_eq!(response.content_blocks.len(), 1);
    match &response.content_blocks[0] {
        RawContentBlock::Text(s) => {
            assert_eq!(s, "custom test response");
        }
        _ => panic!("Expected Text content block"),
    }
}

#[tokio::test]
async fn test_llm_registry_register_and_retrieve_stub_provider() {
    let registry = LLMRegistry::new();
    let provider = stub_provider();

    registry
        .register("test-stub".to_string(), provider.clone())
        .await;

    let retrieved = registry.get("test-stub").await;
    assert!(
        retrieved.is_some(),
        "Should retrieve the registered provider"
    );

    let retrieved_provider = retrieved.unwrap();
    assert_eq!(retrieved_provider.id(), "stub");
}

#[tokio::test]
async fn test_llm_registry_list_includes_stub() {
    let registry = LLMRegistry::new();

    registry
        .register("stub-1".to_string(), stub_provider())
        .await;
    registry
        .register(
            "stub-2".to_string(),
            Arc::new(StubProvider::with_response("other")),
        )
        .await;

    let providers = registry.list().await;
    assert_eq!(providers.len(), 2);
    assert!(providers.contains(&"stub-1".to_string()));
    assert!(providers.contains(&"stub-2".to_string()));
}

#[tokio::test]
async fn test_stub_provider_through_registry_chat() {
    let registry = LLMRegistry::new();
    let provider = Arc::new(StubProvider::with_response("agent response from stub"));

    registry
        .register("test-agent".to_string(), provider.clone())
        .await;

    let agent_provider = registry.get("test-agent").await.unwrap();
    let internal_req = InternalRequest {
        model: "stub-model".to_string(),
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: "Hello, agent!".to_string(),
        }],
        temperature: 0.5,
        max_tokens: Some(50),
        stream: false,
        extra_body: serde_json::Map::new(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        session_id: None,
        reasoning_level: ReasoningLevel::default(),
        turn_count: None,
    };
    let body = serde_json::to_value(&internal_req).unwrap();

    let response = agent_provider.send(internal_req, body).await;
    assert!(response.is_ok());
    let resp = response.unwrap();
    assert_eq!(resp.content_blocks.len(), 1);
    match &resp.content_blocks[0] {
        closeclaw::llm::types::RawContentBlock::Text(s) => {
            assert_eq!(s, "agent response from stub");
        }
        _ => panic!("Expected Text content block"),
    }
}
