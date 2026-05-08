use super::*;
use crate::llm::types::{InternalMessage, RawSseChunk};

fn make_request() -> InternalRequest {
    InternalRequest {
        model: "glm-4".to_string(),
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
        }],
        temperature: 0.7,
        max_tokens: Some(256),
        stream: false,
        extra_body: Default::default(),
    }
}

// ── build_request tests ───────────────────────────────────────────────────

#[test]
fn test_build_request_basic() {
    let proto = GlmProtocol::new();
    let request = make_request();
    let body = proto.build_request(&request).unwrap();

    assert_eq!(body.get("model").unwrap(), "glm-4");
    assert!(body.get("messages").unwrap().is_array());
    let temp_val = body.get("temperature").unwrap().as_f64().unwrap();
    assert!((temp_val - 0.7).abs() < 1e-6);
    assert_eq!(body.get("max_tokens").unwrap(), &serde_json::json!(256));
    assert_eq!(body.get("stream").unwrap(), &serde_json::json!(false));
}

#[test]
fn test_build_request_stream() {
    let proto = GlmProtocol::new();
    let mut request = make_request();
    request.stream = true;
    let body = proto.build_request(&request).unwrap();
    assert_eq!(body.get("stream").unwrap(), &serde_json::json!(true));
}

// ── parse_response tests ──────────────────────────────────────────────────

#[test]
fn test_parse_response_normal() {
    let proto = GlmProtocol::new();
    let body = serde_json::json!({
        "choices": [{
            "message": { "content": "GLM reply" },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15
        }
    });

    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(matches!(resp.content_blocks[0], RawContentBlock::Text(ref s) if s == "GLM reply"));
    assert_eq!(resp.usage.prompt_tokens, 10);
    assert_eq!(resp.usage.completion_tokens, 5);
    assert_eq!(resp.usage.total_tokens, Some(15));
    assert_eq!(resp.finish_reason, Some("stop".to_string()));
}

#[test]
fn test_parse_response_reasoning_content() {
    let proto = GlmProtocol::new();
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "",
                "reasoning_content": "Let me think step by step..."
            },
            "finish_reason": "stop"
        }]
    });

    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(
        matches!(resp.content_blocks[0], RawContentBlock::Thinking(ref s) if s == "Let me think step by step...")
    );
}

#[test]
fn test_parse_response_reasoning_content_prefers_text() {
    let proto = GlmProtocol::new();
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "Final answer",
                "reasoning_content": "Thinking trace"
            },
            "finish_reason": "stop"
        }]
    });

    let resp = proto.parse_response(body).unwrap();
    assert_eq!(resp.content_blocks.len(), 1);
    assert!(matches!(resp.content_blocks[0], RawContentBlock::Text(ref s) if s == "Final answer"));
}

#[test]
fn test_parse_response_error_format() {
    let proto = GlmProtocol::new();
    let body = serde_json::json!({
        "error": {
            "code": "invalid_api_key",
            "message": "API key is invalid"
        }
    });

    let resp = proto.parse_response(body).unwrap();
    assert!(resp.content_blocks.is_empty());
    assert_eq!(resp.usage.prompt_tokens, 0);
    assert_eq!(resp.usage.completion_tokens, 0);
    assert!(resp.finish_reason.is_none());
}

#[test]
fn test_parse_response_empty_choices() {
    let proto = GlmProtocol::new();
    let body = serde_json::json!({ "choices": [] });
    let resp = proto.parse_response(body).unwrap();
    assert!(resp.content_blocks.is_empty());
    assert_eq!(resp.usage.prompt_tokens, 0);
}

// ── decorate_headers tests ────────────────────────────────────────────────

#[test]
fn test_decorate_headers_bearer() {
    std::env::remove_var("GLM_API_KEY");
    let proto = GlmProtocol::new();
    let mut headers = HeaderMap::new();
    proto.decorate_headers(&mut headers).unwrap();

    let auth = headers.get(AUTHORIZATION).unwrap();
    assert!(auth.to_str().unwrap().starts_with("Bearer "));
}

#[test]
fn test_decorate_headers_content_type() {
    let proto = GlmProtocol::new();
    let mut headers = HeaderMap::new();
    proto.decorate_headers(&mut headers).unwrap();

    assert_eq!(
        headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "application/json"
    );
}

// ── SSE parsing tests ──────────────────────────────────────────────────────

fn make_sse_chunk(data: &str) -> RawSseChunk {
    RawSseChunk {
        event_type: "message".to_string(),
        data: data.to_string(),
    }
}

#[tokio::test]
async fn test_parse_sse_reasoning_content_delta() {
    let proto = GlmProtocol::new();
    let machine = proto.create_sse_machine();

    let incoming: IncomingSseStream = Box::pin(futures::stream::iter(vec![
        make_sse_chunk(r#"{"choices":[{"delta":{"reasoning_content":"step 1"}}]}"#),
        make_sse_chunk(r#"{"choices":[{"delta":{"reasoning_content":"step 2"}}]}"#),
        make_sse_chunk("[DONE]"),
    ]));

    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    let evt1 = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt1,
        StreamEvent::BlockStart {
            block_type: ContentBlockType::Thinking,
            ..
        }
    ));

    let evt2 = stream.next().await.unwrap().unwrap();
    assert!(
        matches!(evt2, StreamEvent::BlockDelta { delta: ContentDelta::Thinking { thinking: s }, .. } if s == "step 1")
    );

    let evt3 = stream.next().await.unwrap().unwrap();
    assert!(
        matches!(evt3, StreamEvent::BlockDelta { delta: ContentDelta::Thinking { thinking: s }, .. } if s == "step 2")
    );

    let evt4 = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt4,
        StreamEvent::BlockEnd {
            block_type: ContentBlockType::Thinking,
            ..
        }
    ));

    let evt5 = stream.next().await.unwrap().unwrap();
    assert!(matches!(evt5, StreamEvent::MessageEnd { .. }));

    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn test_parse_sse_content_delta_fallback() {
    let proto = GlmProtocol::new();
    let machine = proto.create_sse_machine();

    let incoming: IncomingSseStream = Box::pin(futures::stream::iter(vec![
        make_sse_chunk(r#"{"choices":[{"delta":{"content":"Hello"}}]}"#),
        make_sse_chunk(r#"{"choices":[{"delta":{"content":" world"}}]}"#),
        make_sse_chunk("[DONE]"),
    ]));

    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    let evt1 = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt1,
        StreamEvent::BlockStart {
            block_type: ContentBlockType::Text,
            ..
        }
    ));

    let evt2 = stream.next().await.unwrap().unwrap();
    assert!(
        matches!(evt2, StreamEvent::BlockDelta { delta: ContentDelta::Text { text: s }, .. } if s == "Hello")
    );

    let evt3 = stream.next().await.unwrap().unwrap();
    assert!(
        matches!(evt3, StreamEvent::BlockDelta { delta: ContentDelta::Text { text: s }, .. } if s == " world")
    );

    let evt4 = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        evt4,
        StreamEvent::BlockEnd {
            block_type: ContentBlockType::Text,
            ..
        }
    ));

    let evt5 = stream.next().await.unwrap().unwrap();
    assert!(matches!(evt5, StreamEvent::MessageEnd { .. }));

    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn test_parse_sse_empty_chunk_breaks() {
    let proto = GlmProtocol::new();
    let machine = proto.create_sse_machine();

    let incoming: IncomingSseStream = Box::pin(futures::stream::iter(vec![
        make_sse_chunk(""),
        make_sse_chunk("[DONE]"),
    ]));

    let mut stream = proto.parse_sse_stream(incoming, machine).await;

    let evt = stream.next().await.unwrap().unwrap();
    assert!(matches!(evt, StreamEvent::MessageEnd { .. }));

    assert!(stream.next().await.is_none());
}
