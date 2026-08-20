//! HTTP server module.
//!
//! Configures the Axum router with endpoint routes and starts the server.

use axum::{routing::get, routing::post, Router};
use tokio::net::TcpListener;

use crate::endpoints::anthropic_messages;
use crate::endpoints::models;
use crate::endpoints::openai_chat;

/// Build the Axum router with all endpoint routes.
fn app() -> Router {
    Router::new()
        .route("/v1/chat/completions", post(openai_chat::handler))
        .route("/v1/messages", post(anthropic_messages::handler))
        .route("/v1/models", get(models::handler))
}

/// Start the HTTP server, binding to the given address.
pub async fn start_server(addr: &str) -> anyhow::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    tracing::info!("Fake LLM Server listening on {bound_addr}");

    let app = app();
    axum::serve(listener, app).await?;
    Ok(())
}

/// Start the HTTP server, returning the actual bound address.
///
/// Used by tests to bind on port 0 and discover the assigned port.
pub async fn start_server_addr(addr: &str) -> anyhow::Result<std::net::SocketAddr> {
    let listener = TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    tracing::info!("Fake LLM Server listening on {bound_addr}");

    let app = app();
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "Fake LLM Server failed");
        }
    });

    Ok(bound_addr)
}

/// Build the router (exposed for testing).
pub fn build_router() -> Router {
    app()
}
