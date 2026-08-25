//! Unit tests for the MiniMax provider.

use super::*;
use crate::cache_adapter::for_provider as cache_for_provider;
use crate::interpreter::{MinimaxInterpreter, ModelInterpreter};
use crate::plugin::ModelPlugin;
use crate::protocol::{AnthropicProtocol, ChatProtocol};
use crate::types::{ContentBlock, InternalRequest, InternalResponse, RawContentBlock, RawUsage};

/// Parse a provider's raw JSON response using Anthropic protocol for assertions.
fn parse_provider_json_anthropic(v: serde_json::Value) -> InternalResponse {
    AnthropicProtocol::default()
        .parse_response(v)
        .expect("test: parse_response should succeed")
}
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
    let resp = parse_provider_json_anthropic(resp);
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
    assert!(matches!(err, ProviderError::Http { .. }));
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
    assert!(matches!(err, ProviderError::Http { .. }));
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
    match err {
        ProviderError::Legacy(msg) => {
            assert!(msg.contains("1004"), "should contain 1004");
        }
        other => panic!("Expected Legacy, got: {:?}", other),
    }
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
    let resp = parse_provider_json_anthropic(resp); // Should have Thinking block from thinking content
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
    assert!(matches!(err, ProviderError::Http { .. }));
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
    assert!(matches!(err, ProviderError::Http { .. }));
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

// Integration test: full call chain with mock HTTP
// ===========================================================================

/// Verify the full MiniMax call chain: CacheAdapter + Plugin + Protocol serialization,
/// all wired together via mock HTTP.
#[tokio::test]
async fn test_full_chain_minimax_provider_protocol_plugin_cache() {
    let mut server = mockito::Server::new_async().await;

    // 1. Apply CacheAdapter — MiniMax reuses AnthropicCacheAdapter (see minimax.md "缓存机制")
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
    // AnthropicCacheAdapter splits system_static into cacheable blocks
    let blocks = request
        .system_blocks
        .as_ref()
        .expect("AnthropicCacheAdapter should set system_blocks");
    assert!(!blocks.is_empty());
    assert!(
        blocks.iter().all(|b| b.cache),
        "all static system blocks should be marked cacheable"
    );

    // 2. Apply MiniMaxM2Plugin (model is MiniMax-M2.7, not M3)
    let plugin = MiniMaxM2Plugin;
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
    // AnthropicCacheAdapter injects system_blocks into body as 'system' array
    let system_arr = body
        .get("system")
        .and_then(|v| v.as_array())
        .expect("body should contain 'system' array from AnthropicCacheAdapter");
    assert!(!system_arr.is_empty());
    assert!(
        system_arr
            .iter()
            .all(|blk| blk.get("cache_control").is_some()),
        "each system block should carry cache_control"
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
    let resp = parse_provider_json_anthropic(resp);
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(
        matches!(&resp.content_blocks[0], RawContentBlock::Text(s) if s == "Hello from MiniMax!")
    );
    assert_eq!(resp.usage.prompt_tokens, 10);
    assert_eq!(resp.usage.completion_tokens, 5);
    assert_eq!(resp.finish_reason.as_deref(), Some("end_turn"));
}

// ===========================================================================
// MinimaxInterpreter::interpret_response unit tests
// ===========================================================================

/// Helper to create an InternalResponse with given content blocks and usage.
fn make_internal_response(
    content_blocks: Vec<RawContentBlock>,
    cache_read: Option<u32>,
    cache_write: Option<u32>,
) -> InternalResponse {
    InternalResponse {
        content_blocks,
        usage: RawUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: Some(150),
            cache_read_tokens: cache_read,
            cache_write_tokens: cache_write,
            reasoning_tokens: None,
        },
        finish_reason: Some("end_turn".to_string()),
    }
}

/// Normal path: thinking block with signature → output preserves signature.
#[test]
fn test_interpreter_thinking_preserves_signature() {
    let resp = make_internal_response(
        vec![
            RawContentBlock::Thinking {
                thinking: "reasoning...".to_string(),
                signature: Some("sig_abc".to_string()),
            },
            RawContentBlock::Text("answer".to_string()),
        ],
        None,
        None,
    );
    let interp = MinimaxInterpreter;
    let out = interp.interpret_response(resp);
    // Minimax merges: text first, then thinking
    assert_eq!(out.content_blocks.len(), 2);
    let thinking = out
        .content_blocks
        .iter()
        .find(|b| matches!(b, ContentBlock::Thinking { .. }));
    assert!(thinking.is_some(), "should have a Thinking block");
    match thinking.unwrap() {
        ContentBlock::Thinking { signature, .. } => {
            assert_eq!(signature.as_deref(), Some("sig_abc"));
        }
        _ => unreachable!(),
    }
}

/// Normal path: RawUsage with cache tokens → output preserves them.
#[test]
fn test_interpreter_usage_preserves_cache_tokens() {
    let resp = make_internal_response(
        vec![RawContentBlock::Text("hi".to_string())],
        Some(80),
        Some(20),
    );
    let interp = MinimaxInterpreter;
    let out = interp.interpret_response(resp);
    assert_eq!(out.usage.cache_read_tokens, Some(80));
    assert_eq!(out.usage.cache_write_tokens, Some(20));
}

/// Boundary: mixed thinking blocks, some with signature, some without.
/// Should take the last non-None signature.
#[test]
fn test_interpreter_mixed_thinking_signatures() {
    let resp = make_internal_response(
        vec![
            RawContentBlock::Thinking {
                thinking: "step1".to_string(),
                signature: None,
            },
            RawContentBlock::Thinking {
                thinking: "step2".to_string(),
                signature: Some("sig_first".to_string()),
            },
            RawContentBlock::Thinking {
                thinking: "step3".to_string(),
                signature: None,
            },
            RawContentBlock::Text("done".to_string()),
        ],
        None,
        None,
    );
    let interp = MinimaxInterpreter;
    let out = interp.interpret_response(resp);
    let thinking = out
        .content_blocks
        .iter()
        .find(|b| matches!(b, ContentBlock::Thinking { .. }));
    match thinking.unwrap() {
        ContentBlock::Thinking {
            signature,
            thinking,
        } => {
            // last non-None signature
            assert_eq!(signature.as_deref(), Some("sig_first"));
            // thinking content is concatenated
            assert_eq!(thinking, "step1step2step3");
        }
        _ => unreachable!(),
    }
}

/// Boundary: thinking blocks with no signature at all → output signature is None.
#[test]
fn test_interpreter_thinking_no_signature() {
    let resp = make_internal_response(
        vec![
            RawContentBlock::Thinking {
                thinking: "thinking".to_string(),
                signature: None,
            },
            RawContentBlock::Text("text".to_string()),
        ],
        None,
        None,
    );
    let interp = MinimaxInterpreter;
    let out = interp.interpret_response(resp);
    match out
        .content_blocks
        .iter()
        .find(|b| matches!(b, ContentBlock::Thinking { .. }))
        .unwrap()
    {
        ContentBlock::Thinking { signature, .. } => {
            assert!(signature.is_none());
        }
        _ => unreachable!(),
    }
}

/// Boundary: RawUsage with no cache tokens → output has None (no panic).
#[test]
fn test_interpreter_usage_no_cache_tokens() {
    let resp = make_internal_response(vec![RawContentBlock::Text("hi".to_string())], None, None);
    let interp = MinimaxInterpreter;
    let out = interp.interpret_response(resp);
    assert!(out.usage.cache_read_tokens.is_none());
    assert!(out.usage.cache_write_tokens.is_none());
}

/// Regression: existing text/thinking merge behavior is preserved.
/// Multiple text blocks → single Text, multiple thinking → single Thinking.
#[test]
fn test_interpreter_regression_merge_behavior() {
    let resp = make_internal_response(
        vec![
            RawContentBlock::Text("Part1 ".to_string()),
            RawContentBlock::Text("Part2".to_string()),
            RawContentBlock::Thinking {
                thinking: "think1".to_string(),
                signature: None,
            },
            RawContentBlock::Thinking {
                thinking: "think2".to_string(),
                signature: None,
            },
            RawContentBlock::Text(" Part3".to_string()),
        ],
        None,
        None,
    );
    let interp = MinimaxInterpreter;
    let out = interp.interpret_response(resp);
    // Minimax merges: text first, then thinking
    let text = out
        .content_blocks
        .iter()
        .find(|b| matches!(b, ContentBlock::Text(_)));
    let thinking = out
        .content_blocks
        .iter()
        .find(|b| matches!(b, ContentBlock::Thinking { .. }));
    assert!(text.is_some(), "should have a Text block");
    assert!(thinking.is_some(), "should have a Thinking block");
    match text.unwrap() {
        ContentBlock::Text(s) => assert_eq!(s, "Part1 Part2 Part3"),
        _ => unreachable!(),
    }
    match thinking.unwrap() {
        ContentBlock::Thinking { thinking, .. } => assert_eq!(thinking, "think1think2"),
        _ => unreachable!(),
    }
}
