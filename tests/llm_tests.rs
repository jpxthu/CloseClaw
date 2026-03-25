//! LLM Integration Tests
//!
//! Tests the StubProvider and LLMRegistry integration.
//! These tests verify that the agent+LLM call chain works in CI.

use std::sync::Arc;

use closeclaw::llm::{
    ChatRequest, LLMProvider, LLMRegistry, Message, StubProvider,
};

#[tokio::test]
async fn test_stub_provider_is_stub() {
    let provider = StubProvider::new();
    assert!(provider.is_stub(), "StubProvider should return true for is_stub()");
}

#[tokio::test]
async fn test_stub_provider_chat_returns_fixed_response() {
    let provider = StubProvider::new();
    let request = ChatRequest {
        model: "gpt-4".to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: "Hello, world!".to_string(),
        }],
        temperature: 0.7,
        max_tokens: None,
    };

    let response = provider.chat(request).await.unwrap();
    assert_eq!(response.content, "stub response");
    assert_eq!(response.model, "stub-model");
    assert!(response.usage.total_tokens > 0);
}

#[tokio::test]
async fn test_stub_provider_custom_response() {
    let provider = StubProvider::with_response("custom test response");
    let request = ChatRequest {
        model: "claude-3".to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: "test input".to_string(),
        }],
        temperature: 0.0,
        max_tokens: Some(100),
    };

    let response = provider.chat(request).await.unwrap();
    assert_eq!(response.content, "custom test response");
}

#[tokio::test]
async fn test_llm_registry_register_and_retrieve_stub_provider() {
    let registry = LLMRegistry::new();
    let provider = Arc::new(StubProvider::new());

    registry.register("test-stub".to_string(), provider.clone()).await;

    let retrieved = registry.get("test-stub").await;
    assert!(retrieved.is_some(), "Should retrieve the registered provider");

    let retrieved_provider = retrieved.unwrap();
    assert_eq!(retrieved_provider.name(), "stub");
    assert!(retrieved_provider.is_stub());
}

#[tokio::test]
async fn test_llm_registry_list_includes_stub() {
    let registry = LLMRegistry::new();
    let provider = Arc::new(StubProvider::new());

    registry.register("stub-1".to_string(), provider.clone()).await;
    registry.register("stub-2".to_string(), Arc::new(StubProvider::with_response("other"))).await;

    let providers = registry.list().await;
    assert_eq!(providers.len(), 2);
    assert!(providers.contains(&"stub-1".to_string()));
    assert!(providers.contains(&"stub-2".to_string()));
}

#[tokio::test]
async fn test_stub_provider_through_registry_chat() {
    let registry = LLMRegistry::new();
    let provider = Arc::new(StubProvider::with_response("agent response from stub"));

    registry.register("test-agent".to_string(), provider.clone()).await;

    let agent_provider = registry.get("test-agent").await.unwrap();
    let request = ChatRequest {
        model: "stub-model".to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: "Hello, agent!".to_string(),
        }],
        temperature: 0.5,
        max_tokens: Some(50),
    };

    let response = agent_provider.chat(request).await.unwrap();
    assert_eq!(response.content, "agent response from stub");
    assert!(agent_provider.is_stub());
}
