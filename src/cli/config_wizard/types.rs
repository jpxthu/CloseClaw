//! Types for the Config Wizard

use std::collections::HashMap;

use crate::llm::{model_info::ModelInfo, LLMProvider};
use std::sync::Arc;

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
    },
    ProviderInfo {
        id: "glm",
        display_name: "GLM",
        provider_type: ProviderType::Glm,
    },
    ProviderInfo {
        id: "volcengine",
        display_name: "Volcengine",
        provider_type: ProviderType::Volcengine,
    },
    ProviderInfo {
        id: "deepseek",
        display_name: "DeepSeek",
        provider_type: ProviderType::Deepseek,
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
    pub provider: Option<Arc<dyn LLMProvider>>,
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
            provider: None,
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
