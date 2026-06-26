//! Embedded knowledge base of recommended model parameters.
//!
//! Sources:
//! - MiniMax:实测 API 响应 + 官方文档
//! - GLM: 实测 API 响应 + 官方文档 (bigmodel.cn)
//! - VolcEngine: 火山引擎 ARTVC API 文档
//! - DeepSeek: 实测 API 响应 + 官方文档 (deepseek.com)

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::model_info::InputType;
use super::types::ProtocolId;

// ---------------------------------------------------------------------------
// ReasoningLevels
// ---------------------------------------------------------------------------

/// Levels of structured reasoning supported by a model.
///
/// Different providers use different schemes:
/// - DeepSeek: `off` (no reasoning) / `base` / `reasoner`
/// - GLM: `off` / `on` (toggle via enable_reaching in API request)
/// - Others: `None` (not supported)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "type", content = "value")]
pub enum ReasoningLevels {
    /// Provider does not support configurable reasoning levels.
    None,
    /// On/off toggle (GLM style).
    Toggle { on: bool },
    /// Multi-level reasoning (DeepSeek style).
    Levels {
        off: bool,
        base: bool,
        reasoner: bool,
    },
}

// ---------------------------------------------------------------------------
// ModelRecommendParams
// ---------------------------------------------------------------------------

/// Recommended parameters for a specific model.
/// These values come from 实测 API 响应 and official provider docs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRecommendParams {
    /// Context window size in tokens.
    pub context_window: u32,
    /// Maximum output tokens the model can produce.
    pub max_tokens: u32,
    /// Default sampling temperature.
    pub default_temperature: f32,
    /// Whether the model supports structured reasoning / tool use.
    pub reasoning: bool,
    /// Supported reasoning level scheme.
    pub reasoning_levels: ReasoningLevels,
    /// Supported input modalities.
    pub input_types: Vec<InputType>,
    /// Recommended protocol for this model.
    pub recommended_protocol: ProtocolId,
}

// ---------------------------------------------------------------------------
// ProviderModelKnowledge
// ---------------------------------------------------------------------------

/// In-memory knowledge base of recommended model parameters.
/// Maps provider_id -> (model_id -> ModelRecommendParams).
pub struct ProviderModelKnowledge {
    inner: HashMap<String, HashMap<&'static str, ModelRecommendParams>>,
}

impl ProviderModelKnowledge {
    /// Create the knowledge base populated with all known providers.
    ///
    /// Each provider's models are loaded from a JSON file at compile time
    /// via `include_str!` and parsed at runtime with `serde_json`.
    pub fn new() -> Self {
        let mut inner: HashMap<String, HashMap<&'static str, ModelRecommendParams>> =
            HashMap::new();

        macro_rules! load_provider {
            ($provider:expr, $file:expr) => {{
                let json = include_str!(concat!("assets/", $file));
                let models: HashMap<String, ModelRecommendParams> = serde_json::from_str(json)
                    .unwrap_or_else(|e| panic!("failed to parse {}: {}", $file, e));
                let static_models: HashMap<&'static str, ModelRecommendParams> = models
                    .into_iter()
                    .map(|(k, v)| (&*Box::leak(k.into_boxed_str()), v))
                    .collect();
                inner.insert($provider.to_string(), static_models);
            }};
        }

        load_provider!("deepseek", "deepseek.json");
        load_provider!("glm", "glm.json");
        load_provider!("minimax", "minimax.json");
        load_provider!("volcengine", "volcengine.json");

        Self { inner }
    }

    /// Find recommended params for a specific provider + model.
    pub fn find(&self, provider_id: &str, model_id: &str) -> Option<ModelRecommendParams> {
        self.inner
            .get(&provider_id.to_lowercase())
            .and_then(|models| models.get(model_id).cloned())
    }

    /// List all model ids for a given provider.
    pub fn all_models(&self, provider_id: &str) -> Vec<&'static str> {
        self.inner
            .get(&provider_id.to_lowercase())
            .map(|models| models.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Returns the recommended protocol for a given provider + model.
    ///
    /// Falls back to `"openai"` when the model is not found.
    pub fn recommended_protocol(&self, provider_id: &str, model_id: &str) -> ProtocolId {
        self.find(provider_id, model_id)
            .map(|p| p.recommended_protocol.clone())
            .unwrap_or_else(|| ProtocolId::from("openai"))
    }
}

impl Default for ProviderModelKnowledge {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kb() -> ProviderModelKnowledge {
        ProviderModelKnowledge::new()
    }

    // ── find ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_find_hit() {
        let kb = kb();
        let params = kb.find("minimax", "MiniMax-M2.7");
        assert!(params.is_some());
        let p = params.unwrap();
        assert_eq!(p.context_window, 204_800);
        assert!(p.reasoning);
        assert!(matches!(
            p.reasoning_levels,
            ReasoningLevels::Toggle { on: true }
        ));
    }

    #[test]
    fn test_find_miss_unknown_provider() {
        let kb = kb();
        assert!(kb.find("unknown", "model-x").is_none());
    }

    #[test]
    fn test_find_miss_unknown_model() {
        let kb = kb();
        assert!(kb.find("minimax", "unknown-model").is_none());
    }

    #[test]
    fn test_find_case_insensitive_provider() {
        let kb = kb();
        assert!(kb.find("MINIMAX", "MiniMax-M2").is_some());
        assert!(kb.find("Glm", "glm-5.1").is_some());
    }

    // ── all_models ─────────────────────────────────────────────────────────

    #[test]
    fn test_all_models_hit() {
        let kb = kb();
        let models = kb.all_models("minimax");
        assert!(!models.is_empty());
        assert!(models.contains(&"MiniMax-M2.7"));
    }

    #[test]
    fn test_all_models_miss() {
        let kb = kb();
        let models = kb.all_models("nonexistent");
        assert!(models.is_empty());
    }

    #[test]
    fn test_all_models_all_providers() {
        let kb = kb();
        for p in &["minimax", "glm", "volcengine", "deepseek"] {
            let models = kb.all_models(p);
            assert!(!models.is_empty(), "provider {p} should have models");
        }
    }

    // ── reasoning_levels ──────────────────────────────────────────────────

    #[test]
    fn test_glm_reasoning_levels() {
        let kb = kb();
        let p = kb.find("glm", "glm-5.1").unwrap();
        assert!(matches!(
            p.reasoning_levels,
            ReasoningLevels::Toggle { on: true }
        ));
    }

    #[test]
    fn test_deepseek_reasoning_levels() {
        let kb = kb();
        let p = kb.find("deepseek", "deepseek-v4-flash").unwrap();
        assert!(matches!(
            p.reasoning_levels,
            ReasoningLevels::Levels {
                off: true,
                base: true,
                reasoner: true
            }
        ));
    }

    #[test]
    fn test_minimax_reasoning_levels_toggle() {
        let kb = kb();
        let p = kb.find("minimax", "MiniMax-M2.7").unwrap();
        assert!(matches!(
            p.reasoning_levels,
            ReasoningLevels::Toggle { on: true }
        ));
    }

    // ── JSON loading: model counts ──────────────────────────────────────

    #[test]
    fn test_json_load_deepseek_model_count() {
        let kb = kb();
        let models = kb.all_models("deepseek");
        assert_eq!(models.len(), 5, "deepseek should have 5 models from JSON");
    }

    #[test]
    fn test_json_load_glm_model_count() {
        let kb = kb();
        let models = kb.all_models("glm");
        assert_eq!(models.len(), 9, "glm should have 9 models from JSON");
    }

    #[test]
    fn test_json_load_minimax_model_count() {
        let kb = kb();
        let models = kb.all_models("minimax");
        assert_eq!(models.len(), 7, "minimax should have 7 models from JSON");
    }

    #[test]
    fn test_json_load_volcengine_model_count() {
        let kb = kb();
        let models = kb.all_models("volcengine");
        assert_eq!(models.len(), 2, "volcengine should have 2 models from JSON");
    }

    // ── new models: discoverable via find() ──────────────────────────────

    #[test]
    fn test_find_new_deepseek_v4_pro() {
        let kb = kb();
        let p = kb.find("deepseek", "deepseek-v4-pro");
        assert!(p.is_some(), "deepseek-v4-pro should be in knowledge base");
        let p = p.unwrap();
        assert!(p.reasoning);
        assert_eq!(p.context_window, 1_000_000);
        assert_eq!(p.max_tokens, 384_000);
    }

    #[test]
    fn test_find_new_minimax_m2() {
        let kb = kb();
        let p = kb.find("minimax", "MiniMax-M2");
        assert!(p.is_some(), "MiniMax-M2 should be in knowledge base");
        let p = p.unwrap();
        assert!(p.reasoning);
        assert_eq!(p.context_window, 204_800);
    }

    #[test]
    fn test_find_new_volcengine_models() {
        let kb = kb();
        for model in "doubao-1.5-pro,doubao-1.5-lite".split(',') {
            let p = kb.find("volcengine", model);
            assert!(p.is_some(), "{model} should be in knowledge base");
        }
    }

    #[test]
    fn test_find_new_deepseek_v3_flash() {
        let kb = kb();
        let p = kb.find("deepseek", "deepseek-v3-flash");
        assert!(p.is_some(), "deepseek-v3-flash should be in knowledge base");
        let p = p.unwrap();
        assert_eq!(p.context_window, 64_000);
    }

    // ── updated values ──────────────────────────────────────────────────

    #[test]
    fn test_updated_deepseek_v4_flash_context_window() {
        let kb = kb();
        let p = kb.find("deepseek", "deepseek-v4-flash").unwrap();
        assert_eq!(
            p.context_window, 1_000_000,
            "deepseek-v4-flash context_window should be 1,000,000"
        );
        assert_eq!(
            p.max_tokens, 384_000,
            "deepseek-v4-flash max_tokens should be 384,000"
        );
    }

    // ── recommended_protocol ──────────────────────────────────────────────

    #[test]
    fn test_recommended_protocol_minimax() {
        let kb = kb();
        let proto = kb.recommended_protocol("minimax", "MiniMax-M2.7");
        assert_eq!(proto.as_str(), "anthropic");
    }

    #[test]
    fn test_recommended_protocol_glm() {
        let kb = kb();
        let proto = kb.recommended_protocol("glm", "glm-5.1");
        assert_eq!(proto.as_str(), "openai");
    }

    #[test]
    fn test_recommended_protocol_deepseek() {
        let kb = kb();
        let proto = kb.recommended_protocol("deepseek", "deepseek-v4-flash");
        assert_eq!(proto.as_str(), "anthropic");
    }

    #[test]
    fn test_recommended_protocol_unknown_fallback() {
        let kb = kb();
        let proto = kb.recommended_protocol("unknown", "unknown-model");
        assert_eq!(proto.as_str(), "openai");
    }
}
