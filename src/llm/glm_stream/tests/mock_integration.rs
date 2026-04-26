use super::*;
use mockito::Server;

/// The GLM provider POSTs to `base_url` directly (no path appended).
/// We construct the base_url to include /chat/completions so the mock registered
/// at that path matches.
fn provider_url(server: &Server) -> String {
    format!("{}/chat/completions", server.url())
}

/// Build a minimal ChatRequest for the given model.
fn chat_request(model: &str) -> ChatRequest {
    ChatRequest {
        model: model.to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: "Say hi".to_string(),
        }],
        temperature: 0.0,
        max_tokens: None,
    }
}

/// Collect all ChatStreamChunk items from a StreamingResponse receiver.
async fn collect_stream_chunks(
    mut rx: tokio::sync::mpsc::Receiver<ChatStreamChunk>,
) -> Vec<ChatStreamChunk> {
    let mut chunks = Vec::new();
    while let Some(chunk) = rx.recv().await {
        chunks.push(chunk);
    }
    chunks
}

/// Collect all text deltas and the final Done chunk from streaming chunks.
fn extract_text_and_done(chunks: Vec<ChatStreamChunk>) -> (String, Option<(String, Usage)>) {
    let mut texts = Vec::new();
    let mut done = None;
    for chunk in chunks {
        match chunk {
            ChatStreamChunk::Text(t) => texts.push(t),
            ChatStreamChunk::Done { model, usage } => done = Some((model, usage)),
            ChatStreamChunk::Error(e) => panic!("unexpected streaming error: {}", e),
        }
    }
    (texts.join(""), done)
}

// --- Fixture: streaming-glm-4.7 ---
// Delta: reasoning_content only (glm-4.7 reasoning model)
// Usage: prompt_tokens=9, completion_tokens=30, total_tokens=39, cached_tokens=2, reasoning_tokens=30

#[tokio::test]
async fn test_streaming_glm_4_7_mock() {
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/streaming-glm-4.7.txt");

    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), provider_url(&server));
    let resp = provider
        .chat_streaming(chat_request("glm-4.7"))
        .await
        .expect("chat_streaming should succeed");
    let chunks = collect_stream_chunks(resp).await;

    m.assert_async().await;

    let (text, done) = extract_text_and_done(chunks);
    let (model, usage) = done.expect("should have a Done chunk");

    // Verify merged text content (glm-4.7 uses reasoning_content)
    assert!(
        text.starts_with("1.  **Analyze the Request:"),
        "text should start with expected prefix, got: {}",
        text
    );
    assert!(
        text.contains("Count to 3"),
        "text should contain 'Count to 3', got: {}",
        text
    );

    // Verify model name from final chunk
    assert_eq!(model, "glm-4.7");

    // Verify usage
    assert_eq!(usage.prompt_tokens, 9);
    assert_eq!(usage.completion_tokens, 30);
    assert_eq!(usage.total_tokens, 39);
}

// --- Fixture: streaming-glm-5.1 (short) ---
// Delta: reasoning_content only (glm-5.1 reasoning model)
// Usage: prompt_tokens=12, completion_tokens=50, total_tokens=62, cached_tokens=0, reasoning_tokens=50

#[tokio::test]
async fn test_streaming_glm_5_1_mock() {
    let mut server = Server::new_async().await;
    let fixture =
        include_str!("../../../../tests/fixtures/llm/glm/mock/streaming-glm-5.1-short.txt");

    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), provider_url(&server));
    let resp = provider
        .chat_streaming(chat_request("glm-5.1"))
        .await
        .expect("chat_streaming should succeed");
    let chunks = collect_stream_chunks(resp).await;

    m.assert_async().await;

    let (text, done) = extract_text_and_done(chunks);
    let (model, usage) = done.expect("should have a Done chunk");

    // Verify merged text content (reasoning_content deltas)
    assert_eq!(text, "Hello world!");

    // Verify model name from final chunk
    assert_eq!(model, "glm-5.1");

    // Verify usage
    assert_eq!(usage.prompt_tokens, 12);
    assert_eq!(usage.completion_tokens, 50);
    assert_eq!(usage.total_tokens, 62);
}

// --- Fixture: streaming-glm-5.1-v2 (short) ---
// Usage: prompt_tokens=12, completion_tokens=50, total_tokens=62, cached_tokens=0, reasoning_tokens=50

#[tokio::test]
async fn test_streaming_glm_5_1_v2_mock() {
    let mut server = Server::new_async().await;
    let fixture =
        include_str!("../../../../tests/fixtures/llm/glm/mock/streaming-glm-5.1-v2-short.txt");

    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), provider_url(&server));
    let resp = provider
        .chat_streaming(chat_request("glm-5.1"))
        .await
        .expect("chat_streaming should succeed");
    let chunks = collect_stream_chunks(resp).await;

    m.assert_async().await;

    let (text, done) = extract_text_and_done(chunks);
    let (model, usage) = done.expect("should have a Done chunk");

    // Verify merged text content (reasoning_content deltas)
    assert_eq!(text, "Test response.");

    // Verify model name from final chunk
    assert_eq!(model, "glm-5.1");

    // Verify usage
    assert_eq!(usage.prompt_tokens, 12);
    assert_eq!(usage.completion_tokens, 50);
    assert_eq!(usage.total_tokens, 62);
}
