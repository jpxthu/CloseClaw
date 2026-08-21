//! HTTP server module.
//!
//! Configures the Axum router with endpoint routes and starts the server.
//! Supports optional scenario directory loading for the scenario engine.

use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::{routing::get, routing::post, Router};
use tokio::net::TcpListener;

use crate::endpoints::anthropic_messages;
use crate::endpoints::models;
use crate::endpoints::openai_chat;
use crate::scenario::{ScenarioEngine, ScenarioState};

/// Build the Axum router with all endpoint routes.
fn app(state: ScenarioState) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(openai_chat::handler))
        .route("/v1/messages", post(anthropic_messages::handler))
        .route("/v1/models", get(models::handler))
        .with_state(state)
}

/// Create a `ScenarioState` from an optional scenario directory path.
///
/// If `dir` is `None` or the path does not exist, an empty engine is used
/// (all requests fall through to the default placeholder response).
fn create_state(scenarios_dir: Option<&Path>) -> ScenarioState {
    let engine = match scenarios_dir {
        Some(dir) if dir.exists() => match ScenarioEngine::from_dir(dir) {
            Ok(e) => {
                tracing::info!("Loaded scenario engine from {}", dir.display());
                e
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "Failed to load scenarios from {}, using empty engine",
                    dir.display()
                );
                ScenarioEngine::new(vec![]).expect("empty scenario list cannot conflict")
            }
        },
        _ => ScenarioEngine::new(vec![]).expect("empty scenario list cannot conflict"),
    };
    ScenarioState {
        engine: Arc::new(Mutex::new(engine)),
    }
}

/// Start the HTTP server, binding to the given address.
///
/// If `scenarios_dir` is provided, scenario files are loaded from that directory.
pub async fn start_server(addr: &str, scenarios_dir: Option<&Path>) -> anyhow::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    tracing::info!("Fake LLM Server listening on {bound_addr}");

    let state = create_state(scenarios_dir);
    let app = app(state);
    axum::serve(listener, app).await?;
    Ok(())
}

/// Start the HTTP server, returning the actual bound address.
///
/// Used by tests to bind on port 0 and discover the assigned port.
pub async fn start_server_addr(
    addr: &str,
    scenarios_dir: Option<&Path>,
) -> anyhow::Result<std::net::SocketAddr> {
    let listener = TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    tracing::info!("Fake LLM Server listening on {bound_addr}");

    let state = create_state(scenarios_dir);
    let app = app(state);
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "Fake LLM Server failed");
        }
    });

    Ok(bound_addr)
}

/// Build the router (exposed for testing).
pub fn build_router() -> Router {
    let state = ScenarioState {
        engine: Arc::new(Mutex::new(
            ScenarioEngine::new(vec![]).expect("empty scenario list cannot conflict"),
        )),
    };
    app(state)
}
