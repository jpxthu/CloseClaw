//! Integration tests for Fake LLM Server: startup, port binding, and endpoint routing.
//!
//! Verifies that the HTTP server starts correctly, binds ports, and all three
//! endpoints respond with the expected JSON structures.

use std::net::SocketAddr;

use closeclaw_fake_llm::server::start_server_addr;

/// Spawn a server on a random port and return the bound address.
///
/// Uses port 0 for automatic port assignment by the OS. The server runs in a
/// background tokio task; dropping the returned handle does NOT shut it down
/// (the runtime drop does).
async fn spawn_server() -> SocketAddr {
    let addr = start_server_addr("127.0.0.1:0")
        .await
        .expect("failed to start server on 127.0.0.1:0");
    // Brief yield to let the spawned task start accepting connections.
    tokio::task::yield_now().await;
    addr
}

// ---------------------------------------------------------------------------
// Server startup & port binding
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_server_starts_on_specified_port() {
    let addr = spawn_server().await;
    let client = reqwest::Client::new();
    let url = format!("http://{addr}/v1/models");
    let resp = client.get(&url).send().await.expect("request failed");
    assert_eq!(
        resp.status(),
        200,
        "expected HTTP 200 from /v1/models after startup"
    );
}

#[tokio::test]
async fn test_port_zero_auto_assigns_free_port() {
    let addr = spawn_server().await;
    // Port 0 was requested; the assigned port must be non-zero.
    assert_ne!(
        addr.port(),
        0,
        "port 0 should be auto-assigned to a non-zero value"
    );
    // The address must be on localhost.
    assert_eq!(
        addr.ip(),
        "127.0.0.1".parse::<std::net::IpAddr>().unwrap(),
        "bound address must be 127.0.0.1"
    );
}

// ---------------------------------------------------------------------------
// Endpoint reachability (HTTP 200)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_openai_chat_endpoint_reachable() {
    let addr = spawn_server().await;
    let client = reqwest::Client::new();
    let url = format!("http://{addr}/v1/chat/completions");
    let body = serde_json::json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "hi"}]
    });
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .expect("request failed");
    assert_eq!(
        resp.status(),
        200,
        "POST /v1/chat/completions should return 200"
    );
}

#[tokio::test]
async fn test_anthropic_messages_endpoint_reachable() {
    let addr = spawn_server().await;
    let client = reqwest::Client::new();
    let url = format!("http://{addr}/v1/messages");
    let body = serde_json::json!({
        "model": "claude-3-opus-20240229",
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 100
    });
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200, "POST /v1/messages should return 200");
}

#[tokio::test]
async fn test_models_endpoint_reachable() {
    let addr = spawn_server().await;
    let client = reqwest::Client::new();
    let url = format!("http://{addr}/v1/models");
    let resp = client.get(&url).send().await.expect("request failed");
    assert_eq!(resp.status(), 200, "GET /v1/models should return 200");
}

// ---------------------------------------------------------------------------
// OpenAI /v1/chat/completions — response format
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_openai_chat_response_format() {
    let addr = spawn_server().await;
    let client = reqwest::Client::new();
    let url = format!("http://{addr}/v1/chat/completions");
    let body = serde_json::json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "hello"}]
    });
    let resp: serde_json::Value = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .expect("request failed")
        .json()
        .await
        .expect("invalid JSON");

    assert_eq!(resp["object"], "chat.completion");
    assert!(resp["choices"].is_array(), "choices must be an array");
    assert_eq!(
        resp["choices"].as_array().unwrap().len(),
        1,
        "must have exactly one choice"
    );
    assert_eq!(resp["model"], "gpt-4");
}

// ---------------------------------------------------------------------------
// OpenAI /v1/chat/completions — request parsing correctness
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_openai_chat_request_model_extracted() {
    let addr = spawn_server().await;
    let client = reqwest::Client::new();
    let url = format!("http://{addr}/v1/chat/completions");
    let body = serde_json::json!({
        "model": "gpt-4-turbo",
        "messages": [
            {"role": "system", "content": "You are helpful."},
            {"role": "user", "content": "What is 2+2?"}
        ],
        "max_tokens": 512,
        "temperature": 0.5,
        "stream": false
    });
    let resp: serde_json::Value = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .expect("request failed")
        .json()
        .await
        .expect("invalid JSON");

    // The response model field should echo back the requested model.
    assert_eq!(
        resp["model"], "gpt-4-turbo",
        "response model should match requested model"
    );
}

#[tokio::test]
async fn test_openai_chat_request_messages_parsed() {
    let addr = spawn_server().await;
    let client = reqwest::Client::new();
    let url = format!("http://{addr}/v1/chat/completions");
    // Send a request with multiple messages including tool_calls.
    let body = serde_json::json!({
        "model": "gpt-4",
        "messages": [
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "foo", "arguments": "{}"}}
            ]},
            {"role": "tool", "tool_call_id": "call_1", "content": "result"}
        ]
    });
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .expect("request failed");
    assert_eq!(
        resp.status(),
        200,
        "complex message array should be accepted"
    );
}

// ---------------------------------------------------------------------------
// Anthropic /v1/messages — response format
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_anthropic_messages_response_format() {
    let addr = spawn_server().await;
    let client = reqwest::Client::new();
    let url = format!("http://{addr}/v1/messages");
    let body = serde_json::json!({
        "model": "claude-3-opus-20240229",
        "messages": [{"role": "user", "content": "hello"}],
        "max_tokens": 100
    });
    let resp: serde_json::Value = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .expect("request failed")
        .json()
        .await
        .expect("invalid JSON");

    assert_eq!(resp["type"], "message");
    assert_eq!(resp["role"], "assistant");
    assert!(
        resp["content"].is_array(),
        "content must be an array of blocks"
    );
    assert_eq!(
        resp["model"], "claude-3-opus-20240229",
        "response model should match request"
    );
}

// ---------------------------------------------------------------------------
// Anthropic /v1/messages — request parsing correctness
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_anthropic_messages_request_model_extracted() {
    let addr = spawn_server().await;
    let client = reqwest::Client::new();
    let url = format!("http://{addr}/v1/messages");
    let body = serde_json::json!({
        "model": "claude-3-sonnet-20240229",
        "messages": [
            {"role": "user", "content": "What is the capital of France?"}
        ],
        "max_tokens": 2048,
        "system": "You are a helpful assistant.",
        "temperature": 0.3,
        "stream": false
    });
    let resp: serde_json::Value = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .expect("request failed")
        .json()
        .await
        .expect("invalid JSON");

    assert_eq!(
        resp["model"], "claude-3-sonnet-20240229",
        "response model should match requested model"
    );
}

#[tokio::test]
async fn test_anthropic_messages_request_with_tools() {
    let addr = spawn_server().await;
    let client = reqwest::Client::new();
    let url = format!("http://{addr}/v1/messages");
    let body = serde_json::json!({
        "model": "claude-3-opus-20240229",
        "messages": [{"role": "user", "content": "Weather?"}],
        "max_tokens": 1024,
        "tools": [{
            "name": "get_weather",
            "description": "Get weather",
            "input_schema": {"type": "object", "properties": {}}
        }],
        "stop_sequences": ["\n\n"]
    });
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .expect("request failed");
    assert_eq!(
        resp.status(),
        200,
        "request with tools and stop_sequences should be accepted"
    );
}

// ---------------------------------------------------------------------------
// /v1/models — JSON structure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_models_response_structure() {
    let addr = spawn_server().await;
    let client = reqwest::Client::new();
    let url = format!("http://{addr}/v1/models");
    let resp: serde_json::Value = client
        .get(&url)
        .send()
        .await
        .expect("request failed")
        .json()
        .await
        .expect("invalid JSON");

    assert_eq!(resp["object"], "list", "top-level object must be 'list'");
    assert!(resp["data"].is_array(), "data must be an array");
    let data = resp["data"].as_array().unwrap();
    assert!(!data.is_empty(), "data must contain at least one model");

    // Each model entry must have required fields.
    for model in data {
        assert!(model["id"].is_string(), "model id must be a string");
        assert_eq!(
            model["object"], "model",
            "model object field must be 'model'"
        );
        assert!(
            model["owned_by"].is_string(),
            "model owned_by must be a string"
        );
    }
}

#[tokio::test]
async fn test_models_contains_expected_models() {
    let addr = spawn_server().await;
    let client = reqwest::Client::new();
    let url = format!("http://{addr}/v1/models");
    let resp: serde_json::Value = client
        .get(&url)
        .send()
        .await
        .expect("request failed")
        .json()
        .await
        .expect("invalid JSON");

    let model_ids: Vec<String> = resp["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();

    assert!(
        model_ids.contains(&"gpt-4".to_string()),
        "must include gpt-4"
    );
    assert!(
        model_ids.contains(&"gpt-3.5-turbo".to_string()),
        "must include gpt-3.5-turbo"
    );
    assert!(
        model_ids.contains(&"claude-3-opus-20240229".to_string()),
        "must include claude-3-opus-20240229"
    );
}
