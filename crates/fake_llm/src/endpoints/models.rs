//! `/v1/models` endpoint handler.
//!
//! Returns a placeholder model list in OpenAI format. The scenario engine
//! (Sequence 2) will replace this with deterministic, scenario-driven
//! model lists for verifying CloseClaw's model discovery chain.

use axum::Json;
use serde::Serialize;

/// OpenAI-compatible model list response.
#[derive(Debug, Serialize)]
pub struct ModelsResponse {
    /// Always `"list"`.
    object: String,
    /// Array of model objects.
    data: Vec<ModelObject>,
}

/// A single model entry in OpenAI format.
#[derive(Debug, Serialize)]
pub struct ModelObject {
    /// Model ID (e.g. "gpt-4", "claude-3-opus-20240229").
    id: String,
    /// Always `"model"`.
    object: String,
    /// Timestamp of model creation (epoch seconds).
    created: u64,
    /// Owning organization.
    owned_by: String,
}

/// Build a placeholder model list.
///
/// Contains a mix of known and unknown model IDs so that downstream tests
/// can verify CloseClaw's filtering of unrecognized models.
fn build_placeholder_models() -> ModelsResponse {
    ModelsResponse {
        object: "list".to_string(),
        data: vec![
            ModelObject {
                id: "gpt-4".to_string(),
                object: "model".to_string(),
                created: 0,
                owned_by: "openai".to_string(),
            },
            ModelObject {
                id: "gpt-3.5-turbo".to_string(),
                object: "model".to_string(),
                created: 0,
                owned_by: "openai".to_string(),
            },
            ModelObject {
                id: "claude-3-opus-20240229".to_string(),
                object: "model".to_string(),
                created: 0,
                owned_by: "anthropic".to_string(),
            },
        ],
    }
}

/// Handler for GET `/v1/models`.
///
/// Returns a placeholder OpenAI-format model list. The scenario engine
/// (Sequence 2) will replace this with deterministic responses and error
/// injection support.
pub async fn handler() -> Json<ModelsResponse> {
    // TODO(Sequence 2): delegate to scenario engine for deterministic model list
    Json(build_placeholder_models())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_models_structure() {
        let resp = build_placeholder_models();
        assert_eq!(resp.object, "list");
        assert_eq!(resp.data.len(), 3);
        assert_eq!(resp.data[0].id, "gpt-4");
        assert_eq!(resp.data[0].object, "model");
        assert_eq!(resp.data[0].owned_by, "openai");
        assert_eq!(resp.data[2].id, "claude-3-opus-20240229");
        assert_eq!(resp.data[2].owned_by, "anthropic");
    }

    #[test]
    fn placeholder_models_json_shape() {
        let resp = build_placeholder_models();
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["object"], "list");
        assert!(json["data"].is_array());
        assert_eq!(json["data"][0]["id"], "gpt-4");
        assert_eq!(json["data"][0]["object"], "model");
    }
}
