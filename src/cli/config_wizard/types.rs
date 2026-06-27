//! Types for the Config Wizard

use std::collections::HashMap;

use crate::llm::model_info::ModelInfo;

use serde::{Deserialize, Serialize};

/// Represents the current state in the wizard state machine
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WizardState {
    SelectProvider,
    InputCredential,
    FetchModels,
    SelectModels,
    Confirm,
    WriteConfig,
}

/// Information about a provider
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub id: &'static str,
    pub display_name: &'static str,
    pub provider_type: ProviderType,
    pub description: &'static str,
}

/// Provider type enumeration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderType {
    Minimax,
    Glm,
    Volcengine,
    Deepseek,
}

/// All available providers
pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        id: "minimax",
        display_name: "MiniMax",
        provider_type: ProviderType::Minimax,
        description: "多模态大模型，推荐 Anthropic 协议",
    },
    ProviderInfo {
        id: "glm",
        display_name: "GLM",
        provider_type: ProviderType::Glm,
        description: "智谱 AI 大模型，推荐 OpenAI 协议",
    },
    ProviderInfo {
        id: "volcengine",
        display_name: "Volcengine",
        provider_type: ProviderType::Volcengine,
        description: "火山引擎大模型",
    },
    ProviderInfo {
        id: "deepseek",
        display_name: "DeepSeek",
        provider_type: ProviderType::Deepseek,
        description: "深度求索推理模型，推荐 Anthropic 协议",
    },
];

/// Context carried through the wizard
#[derive(Clone)]
pub struct WizardContext {
    pub current_state: WizardState,
    pub selected_provider: Option<ProviderInfo>,
    pub credential: Option<String>,
    pub fetched_models: Vec<ModelInfo>,
    pub selected_models: Vec<ModelInfo>,
    pub existing_config: HashMap<String, serde_json::Value>,
}

impl Default for WizardContext {
    fn default() -> Self {
        Self {
            current_state: WizardState::SelectProvider,
            selected_provider: None,
            credential: None,
            fetched_models: Vec::new(),
            selected_models: Vec::new(),
            existing_config: HashMap::new(),
        }
    }
}

/// Output of the wizard, passed to config writer
#[derive(Debug, Clone)]
pub struct WizardOutput {
    pub provider_id: String,
    pub credential: String,
    pub selected_models: Vec<ModelInfo>,
}
