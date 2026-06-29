//! Unit tests for the MiniMax provider.

use super::*;
use crate::cache_adapter::for_provider as cache_for_provider;
use crate::plugin::ModelPlugin;
use crate::protocol::{AnthropicProtocol, ChatProtocol};
use crate::types::InternalRequest;
use crate::{ModelLister, Provider};
use closeclaw_session::persistence::ReasoningLevel;

// --- Provider trait tests ---

#[test]
fn test_provider_id() {
    let provider = MiniMaxProvider::new("key".into());
    assert_eq!(Provider::id(&provider), "minimax");
}

#[test]
fn test_provider_base_url() {
    let provider = MiniMaxProvider::new("key".into());
    assert_eq!(
        Provider::base_url(&provider),
        "https://api.minimax.chat/v1/messages"
    );
}

#[test]
fn test_provider_api_key() {
    let provider = MiniMaxProvider::new("my-key".into());
    assert_eq!(Provider::api_key(&provider), "my-key");
}

#[test]
fn test_provider_supported_protocols() {
    let provider = MiniMaxProvider::new("key".into());
    let protocols = Provider::supported_protocols(&provider);
    assert_eq!(protocols.len(), 1);
    assert_eq!(protocols[0].as_str(), "anthropic");
}

#[test]
fn test_provider_http_client() {
    let provider = MiniMaxProvider::new("key".into());
    // Just verify it returns a valid reference
    let _ = Provider::http_client(&provider);
}

#[test]
fn test_provider_default_headers() {
    let provider = MiniMaxProvider::new("key".into());
    let headers = Provider::default_headers(&provider);
    assert!(headers.is_empty());
}

// --- Provider send() via mockito ---

fn mock_provider(server: &mockito::Server) -> MiniMaxProvider {
    MiniMaxProvider::with_http_client("test-key".into(), server.url(), reqwest::Client::new())
}

fn create_internal_request(model: &str) -> InternalRequest {
    InternalRequest {
        model: model.to_string(),
        messages: vec![crate::types::InternalMessage {
            role: "user".into(),
            content: "hi".into(),
            ..Default::default()
        }],
        temperature: 0.7,
        max_tokens: None,
        stream: false,
        extra_body: serde_json::Map::new(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        tools: None,
        session_id: None,
        reasoning_level: ReasoningLevel::default(),
        turn_count: None,
    }
}

fn make_minimax_response(
    content: Vec<serde_json::Value>,
    usage: Option<serde_json::Value>,
    stop_reason: Option<&str>,
) -> MiniMaxResponse {
    MiniMaxResponse {
        content: if content.is_empty() {
            None
        } else {
            Some(serde_json::from_value(serde_json::Value::Array(content)).unwrap())
        },
        usage: usage.and_then(|u| serde_json::from_value(u).ok()),
        model: "MiniMax-M2.7".to_string(),
        stop_reason: stop_reason.map(|s| s.to_string()),
        base_resp: None,
    }
}

#[tokio::test]
async fn test_provider_send_success_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .match_header("x-api-key", "test-key")
        .match_header("anthropic-version", "2023-06-01")
        .match_header("Content-Type", "application/json")
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(
            r#"{
            "content":[{"type":"text","text":"hi"}],
            "usage":{"input_tokens":5,"output_tokens":10}
        }"#,
        )
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_internal_request("Abab5.5-chat");
    let body = serde_json::json!({
        "model": "Abab5.5-chat",
        "messages": [{"role": "user", "content": "hi"}],
        "temperature": 0.7,
        "stream": false
    });
    let result = Provider::send(&provider, req, body).await;

    m.assert_async().await;
    assert!(result.is_ok());
    let resp = result.unwrap();
    assert!(!resp.content_blocks.is_empty());
    assert_eq!(resp.usage.prompt_tokens, 5);
    assert_eq!(resp.usage.completion_tokens, 10);
}

#[tokio::test]
async fn test_provider_send_auth_failure_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .match_header("x-api-key", "test-key")
        .with_status(401)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"base_resp":{"status_code":1004,"status_msg":"auth failed"}}"#)
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_internal_request("Abab5.5-chat");
    let body = serde_json::json!({"model": "Abab5.5-chat"});
    let err = Provider::send(&provider, req, body).await.unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, ProviderError::Legacy(_)));
}

#[tokio::test]
async fn test_provider_send_rate_limit_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .with_status(429)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"error":"rate limit exceeded"}"#)
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_internal_request("Abab5.5-chat");
    let body = serde_json::json!({"model": "Abab5.5-chat"});
    let err = Provider::send(&provider, req, body).await.unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, ProviderError::Legacy(_)));
}

#[tokio::test]
async fn test_provider_send_business_error_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"base_resp":{"status_code":1004,"status_msg":"token invalid"}}"#)
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_internal_request("Abab5.5-chat");
    let body = serde_json::json!({"model": "Abab5.5-chat"});
    let err = Provider::send(&provider, req, body).await.unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, ProviderError::Legacy(ref msg) if msg.contains("1004")));
}

#[tokio::test]
async fn test_provider_send_reasoning_content_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(
            r#"{
            "content":[
                {"type":"thinking","thinking":"thinking..."},
                {"type":"text","text":"response"}
            ],
            "usage":{"input_tokens":5,"output_tokens":10}
        }"#,
        )
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_internal_request("Abab5.5-chat");
    let body = serde_json::json!({"model": "Abab5.5-chat"});
    let result = Provider::send(&provider, req, body).await;

    m.assert_async().await;
    assert!(result.is_ok());
    let resp = result.unwrap();
    // Should have Thinking block from thinking content
    assert!(resp
        .content_blocks
        .iter()
        .any(|b| matches!(b, RawContentBlock::Thinking { .. })));
}

// --- Provider send_streaming() via mockito ---

fn create_streaming_request(model: &str) -> InternalRequest {
    InternalRequest {
        model: model.to_string(),
        messages: vec![crate::types::InternalMessage {
            role: "user".into(),
            content: "hi".into(),
            ..Default::default()
        }],
        temperature: 0.7,
        max_tokens: None,
        stream: true,
        extra_body: serde_json::Map::new(),
        system_static: None,
        system_dynamic: None,
        tools: None,
        system_blocks: None,
        session_id: None,
        reasoning_level: ReasoningLevel::default(),
        turn_count: None,
    }
}

#[tokio::test]
async fn test_provider_send_streaming_success_mock() {
    let mut server = mockito::Server::new_async().await;
    let sse_body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg-1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"MiniMax-M2.7\",\"stop_reason\":null,\"usage\":{\"input_tokens\":5,\"output_tokens\":0}}}\n",
        "\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n",
        "\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n",
        "\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n",
        "\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":5}}\n",
        "\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n",
        "\n",
    );
    let m = server
        .mock("POST", "/")
        .match_header("x-api-key", "test-key")
        .match_header("anthropic-version", "2023-06-01")
        .with_status(200)
        .with_header("Content-Type", "text/event-stream")
        .with_chunked_body(move |w| {
            w.write_all(sse_body.as_bytes()).unwrap();
            Ok(())
        })
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_streaming_request("Abab5.5-chat");
    let body = serde_json::json!({
        "model": "Abab5.5-chat",
        "messages": [{"role": "user", "content": "hi"}],
        "temperature": 0.7,
        "stream": true
    });
    let result = Provider::send_streaming(&provider, req, body).await;

    m.assert_async().await;
    assert!(result.is_ok());
    let mut rx = result.unwrap();
    let mut chunks = Vec::new();
    while let Some(chunk) = rx.recv().await {
        chunks.push(chunk);
    }
    // Anthropic SSE: message_start, content_block_start, content_block_delta,
    // content_block_stop, message_delta, message_stop = 6 events
    assert!(
        chunks.len() >= 4,
        "should have at least 4 data chunks (message_start, content_block_start, content_block_delta, content_block_stop)"
    );
    // Verify we got Anthropic-format events
    let event_types: Vec<&str> = chunks.iter().map(|c| c.event_type.as_str()).collect();
    assert!(event_types.contains(&"content_block_delta"));
}

#[tokio::test]
async fn test_provider_send_streaming_reasoning_mock() {
    let mut server = mockito::Server::new_async().await;
    let sse_body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg-1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"MiniMax-M2.7\",\"stop_reason\":null,\"usage\":{\"input_tokens\":5,\"output_tokens\":0}}}\n",
        "\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n",
        "\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"thinking...\"}}\n",
        "\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n",
        "\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":5}}\n",
        "\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n",
        "\n",
    );
    let m = server
        .mock("POST", "/")
        .with_status(200)
        .with_header("Content-Type", "text/event-stream")
        .with_chunked_body(move |w| {
            w.write_all(sse_body.as_bytes()).unwrap();
            Ok(())
        })
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_streaming_request("Abab5.5-chat");
    let body = serde_json::json!({
        "model": "Abab5.5-chat",
        "stream": true
    });
    let result = Provider::send_streaming(&provider, req, body).await;

    m.assert_async().await;
    assert!(result.is_ok());
    let mut rx = result.unwrap();
    let mut chunks = Vec::new();
    while let Some(chunk) = rx.recv().await {
        chunks.push(chunk);
    }
    // Should have thinking_delta in the events
    let has_thinking = chunks.iter().any(|c| c.data.contains("thinking_delta"));
    assert!(
        has_thinking,
        "streaming should include thinking_delta events"
    );
}

#[tokio::test]
async fn test_provider_send_streaming_auth_failure_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .match_header("x-api-key", "test-key")
        .with_status(401)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"base_resp":{"status_code":1004,"status_msg":"auth failed"}}"#)
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_streaming_request("Abab5.5-chat");
    let body = serde_json::json!({"model": "Abab5.5-chat"});
    let err = Provider::send_streaming(&provider, req, body)
        .await
        .unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, ProviderError::Legacy(_)));
}

#[tokio::test]
async fn test_provider_send_streaming_rate_limit_mock() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/")
        .with_status(429)
        .with_header("Content-Type", "application/json")
        .with_body(r#"{"error":"rate limit exceeded"}"#)
        .create_async()
        .await;

    let provider = mock_provider(&server);
    let req = create_streaming_request("Abab5.5-chat");
    let body = serde_json::json!({"model": "Abab5.5-chat"});
    let err = Provider::send_streaming(&provider, req, body)
        .await
        .unwrap_err();

    m.assert_async().await;
    assert!(matches!(err, ProviderError::Legacy(_)));
}

// --- fetch_model_list knowledge base filling tests ---

#[tokio::test]
async fn test_fetch_model_list_uses_knowledge_base() {
    let mut server = mockito::Server::new_async().await;
    let api_response = serde_json::json!({
        "data": [
            {"id": "MiniMax-M2.7", "owned_by": "minimax"},
            {"id": "MiniMax-M2", "owned_by": "minimax"}
        ]
    });
    let m = server
        .mock("GET", "/v1/models")
        .match_header(
            "Authorization",
            mockito::Matcher::Regex(r"Bearer .+".to_string()),
        )
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(api_response.to_string())
        .create_async()
        .await;

    let provider =
        MiniMaxProvider::with_http_client("test-key".into(), server.url(), reqwest::Client::new());
    let models = ModelLister::fetch_model_list(&provider, "test-key")
        .await
        .unwrap();

    m.assert_async().await;
    assert_eq!(models.len(), 2);

    // MiniMax-M2.7: knowledge base has reasoning=true, context_window=204800
    let m27 = models.iter().find(|m| m.id == "MiniMax-M2.7").unwrap();
    assert!(
        m27.reasoning,
        "knowledge base should set reasoning=true for M2.7"
    );
    assert_eq!(m27.context_window, 204_800);

    // MiniMax-M2: knowledge base has reasoning=true, context_window=204800
    let m2 = models.iter().find(|m| m.id == "MiniMax-M2").unwrap();
    assert!(
        m2.reasoning,
        "knowledge base should set reasoning=true for M2"
    );
    assert_eq!(m2.context_window, 204_800);
}

#[tokio::test]
async fn test_fetch_model_list_unknown_model_uses_fallback() {
    let mut server = mockito::Server::new_async().await;
    let api_response = serde_json::json!({
        "data": [
            {"id": "unknown-future-model", "owned_by": "minimax"}
        ]
    });
    let m = server
        .mock("GET", "/v1/models")
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(api_response.to_string())
        .create_async()
        .await;

    let provider =
        MiniMaxProvider::with_http_client("test-key".into(), server.url(), reqwest::Client::new());
    let models = ModelLister::fetch_model_list(&provider, "test-key")
        .await
        .unwrap();

    m.assert_async().await;
    assert_eq!(models.len(), 1);
    // Unknown model: fallback defaults (context_window=32768, reasoning=false)
    let model = &models[0];
    assert_eq!(model.id, "unknown-future-model");
    assert_eq!(model.context_window, 32_768);
    assert!(!model.reasoning);
}

// ===========================================================================
// parse_chat_response unit tests (Anthropic protocol format)
// ===========================================================================

#[test]
fn test_parse_chat_response_text_only() {
    let resp = make_minimax_response(
        vec![serde_json::json!({"type": "text", "text": "Hello world"})],
        Some(serde_json::json!({"input_tokens": 10, "output_tokens": 5})),
        Some("end_turn"),
    );
    let result = MiniMaxProvider::parse_chat_response(resp).unwrap();
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], RawContentBlock::Text(s) if s == "Hello world"));
    assert_eq!(result.usage.prompt_tokens, 10);
    assert_eq!(result.usage.completion_tokens, 5);
    assert_eq!(result.finish_reason.as_deref(), Some("end_turn"));
}

#[test]
fn test_parse_chat_response_thinking_and_text() {
    let resp = make_minimax_response(
        vec![
            serde_json::json!({"type": "thinking", "thinking": "reasoning...", "signature": "sig_123"}),
            serde_json::json!({"type": "text", "text": "Final answer"}),
        ],
        Some(serde_json::json!({"input_tokens": 20, "output_tokens": 15})),
        Some("end_turn"),
    );
    let result = MiniMaxProvider::parse_chat_response(resp).unwrap();
    assert_eq!(result.content_blocks.len(), 2);
    assert!(matches!(
        &result.content_blocks[0],
        RawContentBlock::Thinking { thinking, signature }
            if thinking == "reasoning..." && signature.as_deref() == Some("sig_123")
    ));
    assert!(matches!(&result.content_blocks[1], RawContentBlock::Text(s) if s == "Final answer"));
}

#[test]
fn test_parse_chat_response_empty_content() {
    let resp = make_minimax_response(
        vec![],
        Some(serde_json::json!({"input_tokens": 5, "output_tokens": 0})),
        Some("end_turn"),
    );
    let result = MiniMaxProvider::parse_chat_response(resp).unwrap();
    assert!(result.content_blocks.is_empty());
    assert_eq!(result.usage.prompt_tokens, 5);
    assert_eq!(result.usage.completion_tokens, 0);
}

#[test]
fn test_parse_chat_response_cache_usage_fields() {
    let resp = make_minimax_response(
        vec![serde_json::json!({"type": "text", "text": "cached"})],
        Some(serde_json::json!({
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_read_input_tokens": 80,
            "cache_creation_input_tokens": 20
        })),
        Some("end_turn"),
    );
    let result = MiniMaxProvider::parse_chat_response(resp).unwrap();
    assert_eq!(result.usage.prompt_tokens, 100);
    assert_eq!(result.usage.completion_tokens, 50);
    assert_eq!(result.usage.cache_read_tokens, Some(80));
    assert_eq!(result.usage.cache_write_tokens, Some(20));
}

#[test]
fn test_parse_chat_response_no_usage() {
    let resp = make_minimax_response(
        vec![serde_json::json!({"type": "text", "text": "hi"})],
        None,
        Some("end_turn"),
    );
    let result = MiniMaxProvider::parse_chat_response(resp).unwrap();
    assert_eq!(result.usage.prompt_tokens, 0);
    assert_eq!(result.usage.completion_tokens, 0);
}

#[test]
fn test_parse_chat_response_empty_thinking_is_skipped() {
    let resp = make_minimax_response(
        vec![
            serde_json::json!({"type": "thinking", "thinking": ""}),
            serde_json::json!({"type": "text", "text": "response"}),
        ],
        None,
        None,
    );
    let result = MiniMaxProvider::parse_chat_response(resp).unwrap();
    // Empty thinking block should be skipped
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], RawContentBlock::Text(s) if s == "response"));
}

#[test]
fn test_parse_chat_response_unknown_block_type_is_skipped() {
    let resp = make_minimax_response(
        vec![
            serde_json::json!({"type": "unknown_type", "text": "ignored"}),
            serde_json::json!({"type": "text", "text": "kept"}),
        ],
        None,
        None,
    );
    let result = MiniMaxProvider::parse_chat_response(resp).unwrap();
    assert_eq!(result.content_blocks.len(), 1);
    assert!(matches!(&result.content_blocks[0], RawContentBlock::Text(s) if s == "kept"));
}

#[test]
fn test_parse_chat_response_business_error() {
    let resp = MiniMaxResponse {
        content: None,
        usage: None,
        model: "MiniMax-M2.7".to_string(),
        stop_reason: None,
        base_resp: Some(MiniMaxBaseResp {
            status_code: 1004,
            status_msg: "token invalid".to_string(),
        }),
    };
    let err = MiniMaxProvider::parse_chat_response(resp).unwrap_err();
    assert!(matches!(err, ProviderError::Legacy(ref msg) if msg.contains("1004")));
}

#[test]
fn test_parse_chat_response_multiple_text_blocks() {
    let resp = make_minimax_response(
        vec![
            serde_json::json!({"type": "text", "text": "Part 1"}),
            serde_json::json!({"type": "text", "text": "Part 2"}),
            serde_json::json!({"type": "text", "text": "Part 3"}),
        ],
        None,
        None,
    );
    let result = MiniMaxProvider::parse_chat_response(resp).unwrap();
    assert_eq!(result.content_blocks.len(), 3);
    assert!(matches!(&result.content_blocks[0], RawContentBlock::Text(s) if s == "Part 1"));
    assert!(matches!(&result.content_blocks[1], RawContentBlock::Text(s) if s == "Part 2"));
    assert!(matches!(&result.content_blocks[2], RawContentBlock::Text(s) if s == "Part 3"));
}

// ===========================================================================
// Integration test: full call chain with mock HTTP
// ===========================================================================

/// Verify the full MiniMax call chain: CacheAdapter + Plugin + Protocol serialization,
/// all wired together via mock HTTP.
#[tokio::test]
async fn test_full_chain_minimax_provider_protocol_plugin_cache() {
    let mut server = mockito::Server::new_async().await;

    // 1. Apply CacheAdapter
    let adapter = cache_for_provider("minimax");
    assert_eq!(adapter.name(), "anthropic");
    let mut request = InternalRequest {
        model: "MiniMax-M2.7".into(),
        messages: vec![
            crate::types::InternalMessage {
                role: "user".into(),
                content: "hi".into(),
                ..Default::default()
            },
            crate::types::InternalMessage {
                role: "tool".into(),
                content: "sunny".into(),
                tool_call_id: Some("call_001".into()),
            },
        ],
        temperature: 0.7,
        max_tokens: Some(1024),
        stream: false,
        extra_body: serde_json::Map::new(),
        system_static: Some("You are a helpful assistant.".to_string()),
        system_dynamic: None,
        system_blocks: None,
        tools: Some(vec![crate::types::ToolDefinition {
            name: "get_weather".into(),
            description: "Get weather".into(),
            input_schema: None,
            cache: false,
        }]),
        session_id: None,
        reasoning_level: ReasoningLevel::default(),
        turn_count: None,
    };
    adapter.apply(&mut request);
    assert!(
        request.system_blocks.is_some(),
        "CacheAdapter should have set system_blocks"
    );

    // 2. Apply MiniMaxPlugin
    let plugin = MiniMaxPlugin;
    plugin.before_request(&mut request);
    assert_eq!(
        request.extra_body.get("reasoning_split"),
        Some(&serde_json::Value::Bool(true)),
        "Plugin should inject reasoning_split"
    );

    // 3. Build request via AnthropicProtocol
    let protocol = AnthropicProtocol::new();
    let body = protocol.build_request(&request).unwrap();
    assert_eq!(body.get("model").unwrap(), "MiniMax-M2.7");
    assert_eq!(body.get("max_tokens").unwrap(), &serde_json::json!(1024));
    // reasoning_split should be in extra_body
    assert_eq!(
        body.get("reasoning_split").unwrap(),
        &serde_json::json!(true)
    );
    // system blocks should be present
    assert!(body.get("system").is_some());
    let system = body.get("system").unwrap().as_array().unwrap();
    assert_eq!(system.len(), 1);
    assert_eq!(
        system[0].get("cache_control"),
        Some(&serde_json::json!({"type": "ephemeral"})),
        "static system block should have cache_control"
    );
    // last message should have cache_control
    let messages = body.get("messages").unwrap().as_array().unwrap();
    let last_msg = messages.last().unwrap();
    let last_content = last_msg.get("content").unwrap().as_array().unwrap();
    assert_eq!(
        last_content.last().unwrap().get("cache_control"),
        Some(&serde_json::json!({"type": "ephemeral"})),
        "last message should have cache_control"
    );

    // 4. Mock the HTTP response
    let m = server
        .mock("POST", "/")
        .match_header("x-api-key", "test-key")
        .match_header("anthropic-version", "2023-06-01")
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(
            serde_json::json!({
                "content": [{"type": "text", "text": "Hello from MiniMax!"}],
                "usage": {"input_tokens": 10, "output_tokens": 5},
                "stop_reason": "end_turn",
                "model": "MiniMax-M2.7"
            })
            .to_string(),
        )
        .create_async()
        .await;

    // 5. Send via MiniMaxProvider
    let provider =
        MiniMaxProvider::with_http_client("test-key".into(), server.url(), reqwest::Client::new());
    let result = Provider::send(&provider, request, body).await;

    m.assert_async().await;
    assert!(result.is_ok(), "send should succeed: {:?}", result.err());
    let resp = result.unwrap();
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(
        matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s == "Hello from MiniMax!")
    );
    assert_eq!(resp.usage.prompt_tokens, 10);
    assert_eq!(resp.usage.completion_tokens, 5);
    assert_eq!(resp.finish_reason.as_deref(), Some("end_turn"));
}
