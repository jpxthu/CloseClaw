//! LLM Integration Tests
//!
//! Tests the StubProvider and LLMRegistry integration.
//! These tests verify that the agent+LLM call chain works in CI.

#![allow(deprecated)]

use std::sync::Arc;

use closeclaw::llm::legacy::legacy_provider::LegacyProviderBridge;
use closeclaw::llm::{ChatRequest, LLMProvider, LLMRegistry, Message, StubProvider};

fn stub_bridge() -> LegacyProviderBridge<StubProvider> {
    LegacyProviderBridge::new(
        StubProvider::new(),
        String::new(),
        String::new(),
        vec![],
        reqwest::Client::new(),
        reqwest::header::HeaderMap::new(),
    )
}

#[tokio::test]
async fn test_stub_provider_is_stub() {
    let provider = StubProvider::new();
    assert!(
        provider.is_stub(),
        "StubProvider should return true for is_stub()"
    );
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
    let bridge = Arc::new(stub_bridge());

    registry
        .register("test-stub".to_string(), bridge.clone())
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
        .register("stub-1".to_string(), Arc::new(stub_bridge()))
        .await;
    registry
        .register(
            "stub-2".to_string(),
            Arc::new(LegacyProviderBridge::new(
                StubProvider::with_response("other"),
                String::new(),
                String::new(),
                vec![],
                reqwest::Client::new(),
                reqwest::header::HeaderMap::new(),
            )),
        )
        .await;

    let providers = registry.list().await;
    assert_eq!(providers.len(), 2);
    assert!(providers.contains(&"stub-1".to_string()));
    assert!(providers.contains(&"stub-2".to_string()));
}

#[tokio::test]
async fn test_stub_provider_through_registry_chat() {
    use closeclaw::llm::types::{InternalMessage, InternalRequest};

    let registry = LLMRegistry::new();
    let bridge = Arc::new(LegacyProviderBridge::new(
        StubProvider::with_response("agent response from stub"),
        String::new(),
        String::new(),
        vec![],
        reqwest::Client::new(),
        reqwest::header::HeaderMap::new(),
    ));

    registry
        .register("test-agent".to_string(), bridge.clone())
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
        reasoning_level: closeclaw::session::persistence::ReasoningLevel::default(),
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
