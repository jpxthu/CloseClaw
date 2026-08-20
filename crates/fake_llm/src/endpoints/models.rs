//! `/v1/models` endpoint handler.
//!
//! Returns a placeholder model list in OpenAI format via the protocol module.
//! The scenario engine (Sequence 2) will replace this with deterministic,
//! scenario-driven model lists for verifying CloseClaw's model discovery chain.

use axum::Json;

use crate::protocol::openai::{build_models_response, ModelsResponse};

/// Handler for GET `/v1/models`.
///
/// Delegates response building to the OpenAI protocol module.
/// The scenario engine (Sequence 2) will replace this with deterministic
/// responses and error injection support.
pub async fn handler() -> Json<ModelsResponse> {
    // TODO(Sequence 2): delegate to scenario engine for deterministic model list
    Json(build_models_response())
}
