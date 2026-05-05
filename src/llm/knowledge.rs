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
    pub fn new() -> Self {
        let mut inner = HashMap::new();

        // ──────────────────────────────────────────────────────────────────────
        // MiniMax
        // MiniMax 官方文档 + 实测 API 响应
        // ──────────────────────────────────────────────────────────────────────
        let mut minimax_models = HashMap::new();
        minimax_models.insert(
            "MiniMax-M2",
            ModelRecommendParams {
                context_window: 32_768,
                max_tokens: 8_192,
                default_temperature: 0.7,
                reasoning: false,
                reasoning_levels: ReasoningLevels::None,
                input_types: vec![InputType::Text],
            },
        );
        minimax_models.insert(
            "MiniMax-M2.1",
            ModelRecommendParams {
                context_window: 32_768,
                max_tokens: 8_192,
                default_temperature: 0.7,
                reasoning: false,
                reasoning_levels: ReasoningLevels::None,
                input_types: vec![InputType::Text],
            },
        );
        minimax_models.insert(
            "MiniMax-M2.5",
            ModelRecommendParams {
                context_window: 32_768,
                max_tokens: 8_192,
                default_temperature: 0.7,
                reasoning: true,
                reasoning_levels: ReasoningLevels::None,
                input_types: vec![InputType::Text],
            },
        );
        minimax_models.insert(
            "MiniMax-M2.7",
            ModelRecommendParams {
                context_window: 32_768,
                max_tokens: 8_192,
                default_temperature: 0.7,
                reasoning: true,
                reasoning_levels: ReasoningLevels::None,
                input_types: vec![InputType::Text],
            },
        );
        inner.insert("minimax".to_string(), minimax_models);

        // ──────────────────────────────────────────────────────────────────────
        // GLM
        // 实测 API 响应 + 官方文档 (bigmodel.cn)
        // reasoning_content 字段表示支持思考内容
        // ──────────────────────────────────────────────────────────────────────
        let mut glm_models = HashMap::new();
        glm_models.insert(
            "GLM-4.5-Air",
            ModelRecommendParams {
                context_window: 128_000,
                max_tokens: 8_192,
                default_temperature: 0.7,
                reasoning: false,
                reasoning_levels: ReasoningLevels::None,
                input_types: vec![InputType::Text],
            },
        );
        glm_models.insert(
            "GLM-4.5-AirX",
            ModelRecommendParams {
                context_window: 128_000,
                max_tokens: 8_192,
                default_temperature: 0.7,
                reasoning: false,
                reasoning_levels: ReasoningLevels::None,
                input_types: vec![InputType::Text, InputType::Image],
            },
        );
        glm_models.insert(
            "glm-4.5-air",
            ModelRecommendParams {
                context_window: 128_000,
                max_tokens: 8_192,
                default_temperature: 0.7,
                reasoning: false,
                reasoning_levels: ReasoningLevels::None,
                input_types: vec![InputType::Text],
            },
        );
        glm_models.insert(
            "glm-4.5-airx",
            ModelRecommendParams {
                context_window: 128_000,
                max_tokens: 8_192,
                default_temperature: 0.7,
                reasoning: false,
                reasoning_levels: ReasoningLevels::None,
                input_types: vec![InputType::Text, InputType::Image],
            },
        );
        glm_models.insert(
            "glm-4.7",
            ModelRecommendParams {
                context_window: 128_000,
                max_tokens: 8_192,
                default_temperature: 0.7,
                reasoning: true,
                reasoning_levels: ReasoningLevels::Toggle { on: false },
                input_types: vec![InputType::Text],
            },
        );
        glm_models.insert(
            "glm-5.1",
            ModelRecommendParams {
                context_window: 128_000,
                max_tokens: 8_192,
                default_temperature: 0.7,
                reasoning: true,
                reasoning_levels: ReasoningLevels::Toggle { on: false },
                input_types: vec![InputType::Text],
            },
        );
        inner.insert("glm".to_string(), glm_models);

        // ──────────────────────────────────────────────────────────────────────
        // VolcEngine (火山引擎 ARTVC)
        // 火山引擎 ARTVC API 文档
        // ──────────────────────────────────────────────────────────────────────
        let mut volcengine_models = HashMap::new();
        volcengine_models.insert(
            "doubao-1.5-pro",
            ModelRecommendParams {
                context_window: 32_768,
                max_tokens: 8_192,
                default_temperature: 0.7,
                reasoning: false,
                reasoning_levels: ReasoningLevels::None,
                input_types: vec![InputType::Text],
            },
        );
        volcengine_models.insert(
            "doubao-1.5-lite",
            ModelRecommendParams {
                context_window: 32_768,
                max_tokens: 8_192,
                default_temperature: 0.7,
                reasoning: false,
                reasoning_levels: ReasoningLevels::None,
                input_types: vec![InputType::Text],
            },
        );
        inner.insert("volcengine".to_string(), volcengine_models);

        // ──────────────────────────────────────────────────────────────────────
        // DeepSeek
        // 实测 API 响应 + 官方文档 (deepseek.com)
        // reasoning_levels 支持 off/base/reasoner 三档
        // ──────────────────────────────────────────────────────────────────────
        let mut deepseek_models = HashMap::new();
        deepseek_models.insert(
            "deepseek-v3-flash",
            ModelRecommendParams {
                context_window: 64_000,
                max_tokens: 8_192,
                default_temperature: 0.7,
                reasoning: false,
                reasoning_levels: ReasoningLevels::Levels {
                    off: false,
                    base: true,
                    reasoner: true,
                },
                input_types: vec![InputType::Text],
            },
        );
        deepseek_models.insert(
            "deepseek-v3",
            ModelRecommendParams {
                context_window: 64_000,
                max_tokens: 8_192,
                default_temperature: 0.7,
                reasoning: false,
                reasoning_levels: ReasoningLevels::Levels {
                    off: false,
                    base: true,
                    reasoner: true,
                },
                input_types: vec![InputType::Text],
            },
        );
        deepseek_models.insert(
            "deepseek-v4-flash",
            ModelRecommendParams {
                context_window: 64_000,
                max_tokens: 8_192,
                default_temperature: 0.7,
                reasoning: true,
                reasoning_levels: ReasoningLevels::Levels {
                    off: false,
                    base: true,
                    reasoner: true,
                },
                input_types: vec![InputType::Text],
            },
        );
        deepseek_models.insert(
            "deepseek-v4",
            ModelRecommendParams {
                context_window: 64_000,
                max_tokens: 8_192,
                default_temperature: 0.7,
                reasoning: true,
                reasoning_levels: ReasoningLevels::Levels {
                    off: false,
                    base: true,
                    reasoner: true,
                },
                input_types: vec![InputType::Text],
            },
        );
        inner.insert("deepseek".to_string(), deepseek_models);

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
        assert_eq!(p.context_window, 32_768);
        assert!(p.reasoning);
        assert!(matches!(p.reasoning_levels, ReasoningLevels::None));
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
            ReasoningLevels::Toggle { on: false }
        ));
    }

    #[test]
    fn test_deepseek_reasoning_levels() {
        let kb = kb();
        let p = kb.find("deepseek", "deepseek-v4-flash").unwrap();
        assert!(matches!(
            p.reasoning_levels,
            ReasoningLevels::Levels {
                off: false,
                base: true,
                reasoner: true
            }
        ));
    }

    #[test]
    fn test_minimax_reasoning_levels_none() {
        let kb = kb();
        let p = kb.find("minimax", "MiniMax-M2.7").unwrap();
        assert!(matches!(p.reasoning_levels, ReasoningLevels::None));
    }
}
