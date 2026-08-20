//! `/v1/models` endpoint handler.
//!
//! Returns a model list in OpenAI format. When a scenario declares a
//! `models` field, that list is returned instead of the default placeholder.
//! Supports error injection and delay injection via the scenario engine.

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{extract::State, Json};

use crate::delivery::apply_delay;
use crate::protocol::openai::{build_models_response, ModelsResponse};
use crate::scenario::{ModelsDecision, ScenarioState};

/// Build a `ModelsResponse` from scenario-declared model entries.
fn build_models_from_entries(entries: &[crate::scenario::types::ModelEntry]) -> ModelsResponse {
    use crate::protocol::openai::ModelObject;

    ModelsResponse {
        object: "list".to_string(),
        data: entries
            .iter()
            .map(|e| ModelObject {
                id: e.id.clone(),
                object: "model".to_string(),
                created: 0,
                owned_by: e.owned_by.clone(),
            })
            .collect(),
    }
}

/// Handler for GET `/v1/models`.
///
/// Delegates to the scenario engine for deterministic model lists and
/// error injection. Falls back to the default placeholder list when no
/// scenario declares models.
pub async fn handler(
    State(state): State<ScenarioState>,
) -> Result<Response, (StatusCode, HeaderMap, String)> {
    let outcome = {
        let mut engine = state.engine.lock().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                HeaderMap::new(),
                e.to_string(),
            )
        })?;
        engine.decide_for_models()
    };

    match outcome {
        ModelsDecision::Placeholder => Ok(Json(build_models_response()).into_response()),
        ModelsDecision::Error(e) => {
            let status =
                StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            Err((status, HeaderMap::new(), e.message))
        }
        ModelsDecision::Models(entries) => {
            apply_delay(None).await;
            Ok(Json(build_models_from_entries(&entries)).into_response())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::types::ModelEntry;

    #[test]
    fn build_models_from_entries_basic() {
        let entries = vec![
            ModelEntry {
                id: "gpt-4".to_string(),
                owned_by: "openai".to_string(),
            },
            ModelEntry {
                id: "claude-3".to_string(),
                owned_by: "anthropic".to_string(),
            },
        ];
        let resp = build_models_from_entries(&entries);
        assert_eq!(resp.object, "list");
        assert_eq!(resp.data.len(), 2);
        assert_eq!(resp.data[0].id, "gpt-4");
        assert_eq!(resp.data[0].owned_by, "openai");
        assert_eq!(resp.data[1].id, "claude-3");
        assert_eq!(resp.data[1].owned_by, "anthropic");
    }

    #[test]
    fn build_models_from_entries_empty() {
        let resp = build_models_from_entries(&[]);
        assert_eq!(resp.object, "list");
        assert!(resp.data.is_empty());
    }
}
