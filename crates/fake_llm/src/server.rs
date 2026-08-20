//! HTTP server module.
//!
//! Configures the Axum router with endpoint routes and starts the server.

use axum::{routing::get, routing::post, Router};
use tokio::net::TcpListener;

/// Placeholder handler for POST `/v1/chat/completions`.
///
/// Will be implemented in Step 1.2 (OpenAI protocol endpoint).
async fn openai_chat_completions() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "id": "chatcmpl-placeholder",
        "object": "chat.completion",
        "created": 0,
        "model": "placeholder",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "placeholder" },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0 }
    }))
}

/// Placeholder handler for POST `/v1/messages`.
///
/// Will be implemented in Step 1.3 (Anthropic protocol endpoint).
async fn anthropic_messages() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "id": "msg-placeholder",
        "type": "message",
        "role": "assistant",
        "content": [{ "type": "text", "text": "placeholder" }],
        "model": "placeholder",
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 0, "output_tokens": 0 }
    }))
}

/// Placeholder handler for GET `/v1/models`.
///
/// Will be implemented in Step 1.4 (model discovery endpoint).
async fn models_list() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "object": "list",
        "data": []
    }))
}

/// Build the Axum router with all endpoint routes.
fn app() -> Router {
    Router::new()
        .route("/v1/chat/completions", post(openai_chat_completions))
        .route("/v1/messages", post(anthropic_messages))
        .route("/v1/models", get(models_list))
}

/// Start the HTTP server, binding to the given address.
///
/// Returns the actual bound address (useful when port 0 is used for automatic
/// port assignment).
pub async fn start_server(addr: &str) -> anyhow::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    tracing::info!("Fake LLM Server listening on {bound_addr}");

    let app = app();
    axum::serve(listener, app).await?;
    Ok(())
}

/// Build the router (exposed for testing).
pub fn build_router() -> Router {
    app()
}
