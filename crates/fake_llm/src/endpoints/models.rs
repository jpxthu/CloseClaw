//! `/v1/models` endpoint handler.
//!
//! Returns a placeholder model list in OpenAI format via the protocol module.
//! Phase 4 will wire this up to the scenario engine for deterministic,
//! scenario-driven model lists.

use axum::Json;

use crate::protocol::openai::{build_models_response, ModelsResponse};

/// Handler for GET `/v1/models`.
///
/// TODO(Phase 4): delegate to scenario engine for deterministic model list
/// and error injection support.
pub async fn handler() -> Json<ModelsResponse> {
    Json(build_models_response())
}
