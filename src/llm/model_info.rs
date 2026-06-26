//! Model metadata types for LLM Model Discovery & Auto-Config Wizard.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::knowledge::ProviderModelKnowledge;

/// Supported input types for a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputType {
    /// Text-only input.
    Text,
    /// Image + text multimodal input.
    Image,
}

/// Model metadata returned by the discovery wizard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Provider-specific model identifier (e.g. "deepseek-v3-flash").
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Context window size in tokens.
    pub context_window: u32,
    /// Maximum output tokens the model can produce.
    pub max_tokens: u32,
    /// Default sampling temperature, if known.
    pub default_temperature: Option<f32>,
    /// Whether the model supports structured reasoning / tool use.
    pub reasoning: bool,
    /// List of input modalities supported by this model.
    pub input_types: Vec<InputType>,
}

impl ModelInfo {
    /// Build a ModelInfo from a "provider/model_id" string by looking up
    /// `ProviderModelKnowledge`. Fields not in the knowledge base get safe
    /// defaults.
    fn from_provider_model_str(s: &str) -> Result<Self, ParseModelInfoError> {
        let (provider, model_id) = s
            .split_once('/')
            .ok_or(ParseModelInfoError(s.to_string()))?;

        let kb = ProviderModelKnowledge::new();
        let params = kb.find(provider, model_id);

        let (context_window, max_tokens, default_temperature, reasoning, input_types) = match params
        {
            Some(p) => (
                p.context_window,
                p.max_tokens,
                Some(p.default_temperature),
                p.reasoning,
                p.input_types.clone(),
            ),
            None => (32_768, 4_096, None, false, vec![InputType::Text]),
        };

        Ok(Self {
            id: model_id.to_string(),
            name: format_name(provider, model_id),
            context_window,
            max_tokens,
            default_temperature,
            reasoning,
            input_types,
        })
    }
}

/// Parse error for ModelInfo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseModelInfoError(String);

impl std::fmt::Display for ParseModelInfoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid ModelInfo format: {}", self.0)
    }
}

impl std::error::Error for ParseModelInfoError {}

impl FromStr for ModelInfo {
    type Err = ParseModelInfoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_provider_model_str(s)
    }
}

/// Construct a human-readable display name from provider and model id.
fn format_name(provider: &str, model_id: &str) -> String {
    let lower = provider.to_lowercase();
    let p = match lower.as_str() {
        "minimax" => "MiniMax",
        "glm" => "GLM",
        "deepseek" => "DeepSeek",
        "volcengine" => "VolcEngine",
        other => other,
    };
    let capitalized = model_id
        .split('-')
        .map(word_capitalize)
        .collect::<Vec<_>>()
        .join(" ");
    format!("{p} {capitalized}")
}

fn word_capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().chain(chars).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_input_type_serde() {
        for ty in &[InputType::Text, InputType::Image] {
            let json = serde_json::to_string(ty).unwrap();
            let parsed: InputType = serde_json::from_str(&json).unwrap();
            assert_eq!(*ty, parsed);
        }
    }

    #[test]
    fn test_model_info_serde() {
        let model = ModelInfo {
            id: "deepseek-v3-flash".to_string(),
            name: "DeepSeek V3 Flash".to_string(),
            context_window: 64_000,
            max_tokens: 8_192,
            default_temperature: Some(0.7),
            reasoning: false,
            input_types: vec![InputType::Text],
        };
        let json = serde_json::to_string(&model).unwrap();
        let parsed: ModelInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "deepseek-v3-flash");
        assert_eq!(parsed.context_window, 64_000);
    }

    #[test]
    fn test_from_str_known_model() {
        let model = ModelInfo::from_str("deepseek/deepseek-v4-flash").unwrap();
        assert_eq!(model.id, "deepseek-v4-flash");
        assert!(model.reasoning);
        assert_eq!(model.context_window, 1_000_000);
        assert_eq!(model.max_tokens, 384_000);
        assert_eq!(model.default_temperature, Some(1.0));
    }

    #[test]
    fn test_from_str_unknown_model() {
        let model = ModelInfo::from_str("unknown/model").unwrap();
        assert_eq!(model.id, "model");
        assert!(!model.reasoning);
        assert_eq!(model.context_window, 32_768);
        assert_eq!(model.max_tokens, 4_096);
        assert_eq!(model.default_temperature, None);
        assert_eq!(model.input_types, vec![InputType::Text]);
    }

    #[test]
    fn test_from_str_bad_format() {
        let err = ModelInfo::from_str("no-slash-at-all").unwrap_err();
        assert!(err.to_string().contains("invalid ModelInfo format"));
    }
}
