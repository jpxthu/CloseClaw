use super::*;
use closeclaw_llm::ReasoningLevels;
use tempfile::TempDir;

fn write_initial_config(
    tmp: &TempDir,
    providers: std::collections::HashMap<String, ProviderConfig>,
) {
    let models_path = tmp.path().join("models.json");
    let initial = ModelsConfigData {
        mode: "merge".to_string(),
        providers,
    };
    std::fs::write(
        &models_path,
        serde_json::to_string_pretty(&initial).unwrap(),
    )
    .unwrap();
}

fn make_wizard_output(provider_id: &str, credential: &str, models: Vec<ModelInfo>) -> WizardOutput {
    WizardOutput {
        provider_id: provider_id.to_string(),
        credential: credential.to_string(),
        selected_models: models,
    }
}

fn make_model(id: &str, name: &str, context_window: u32, max_tokens: u32) -> ModelInfo {
    ModelInfo {
        id: id.to_string(),
        name: name.to_string(),
        context_window,
        max_tokens,
        default_temperature: None,
        reasoning: false,
        reasoning_levels: ReasoningLevels::None,
        input_types: vec![],
    }
}

fn make_model_def(id: &str, name: &str, enabled: bool) -> ModelDefinition {
    ModelDefinition {
        id: id.to_string(),
        name: Some(name.to_string()),
        enabled: Some(enabled),
    }
}

fn read_parsed_config(tmp: &TempDir) -> ModelsConfigData {
    let models_path = tmp.path().join("models.json");
    let content = std::fs::read_to_string(&models_path).unwrap();
    serde_json::from_str(&content).unwrap()
}

/// Test: write_wizard_config outputs protocol field in ProviderConfig
#[test]
fn test_write_wizard_config_includes_protocol() {
    let tmp = TempDir::new().unwrap();
    let output = WizardOutput {
        provider_id: "minimax".to_string(),
        credential: "test-api-key".to_string(),
        selected_models: vec![ModelInfo {
            id: "MiniMax-M2.7".to_string(),
            name: "MiniMax M2.7".to_string(),
            context_window: 1000,
            max_tokens: 1000,
            default_temperature: None,
            reasoning: false,
            reasoning_levels: ReasoningLevels::None,
            input_types: vec![],
        }],
    };

    write_wizard_config_to(&output, tmp.path()).expect("write_wizard_config_to should succeed");

    let models_path = tmp.path().join("models.json");
    let content = std::fs::read_to_string(&models_path).expect("models.json should be written");

    let parsed: ModelsConfigData =
        serde_json::from_str(&content).expect("models.json should be valid JSON");

    let provider = parsed
        .providers
        .get("minimax")
        .expect("minimax provider should exist");
    // MiniMax-M2.7 -> recommended_protocol should be "anthropic"
    assert!(
        provider.protocol.is_some(),
        "protocol field should be present, got: {:?}",
        provider.protocol
    );
    assert_eq!(
        provider.protocol.as_deref(),
        Some("anthropic"),
        "MiniMax-M2.7 should have protocol 'anthropic', got: {:?}",
        provider.protocol
    );
}

/// Test: write_wizard_config_to creates correct file structure in specified path
#[test]
fn test_write_wizard_config_to_creates_files() {
    let tmp = TempDir::new().unwrap();
    let output = WizardOutput {
        provider_id: "minimax".to_string(),
        credential: "test-api-key".to_string(),
        selected_models: vec![ModelInfo {
            id: "MiniMax-M2.7".to_string(),
            name: "MiniMax M2.7".to_string(),
            context_window: 1000,
            max_tokens: 1000,
            default_temperature: None,
            reasoning: false,
            reasoning_levels: ReasoningLevels::None,
            input_types: vec![],
        }],
    };

    write_wizard_config_to(&output, tmp.path()).unwrap();

    // models.json exists and is valid
    let models_path = tmp.path().join("models.json");
    assert!(models_path.exists(), "models.json should exist");
    let content = std::fs::read_to_string(&models_path).unwrap();
    let parsed: ModelsConfigData = serde_json::from_str(&content).unwrap();
    assert!(parsed.providers.contains_key("minimax"));

    // credentials file exists
    let cred_path = tmp.path().join("credentials").join("minimax.json");
    assert!(cred_path.exists(), "credentials file should exist");
}

/// Test: write_wizard_config_to auto-creates nested directories
#[test]
fn test_write_wizard_config_to_creates_nested_dirs() {
    let tmp = TempDir::new().unwrap();
    let nested_path = tmp.path().join("deep").join("nested");
    let output = WizardOutput {
        provider_id: "minimax".to_string(),
        credential: "test-api-key".to_string(),
        selected_models: vec![ModelInfo {
            id: "MiniMax-M2.7".to_string(),
            name: "MiniMax M2.7".to_string(),
            context_window: 1000,
            max_tokens: 1000,
            default_temperature: None,
            reasoning: false,
            reasoning_levels: ReasoningLevels::None,
            input_types: vec![],
        }],
    };

    write_wizard_config_to(&output, &nested_path).unwrap();

    let models_path = nested_path.join("models.json");
    assert!(
        models_path.exists(),
        "models.json should exist in nested path"
    );
    let content = std::fs::read_to_string(&models_path).unwrap();
    let parsed: ModelsConfigData = serde_json::from_str(&content).unwrap();
    assert!(parsed.providers.contains_key("minimax"));

    let cred_path = nested_path.join("credentials").join("minimax.json");
    assert!(
        cred_path.exists(),
        "credentials file should exist in nested path"
    );
}

/// Test: re-running wizard preserves existing models not in selected_models
#[test]
fn test_write_wizard_config_to_preserves_existing_models() {
    let tmp = TempDir::new().unwrap();
    // First write: 1 model
    let output1 = make_wizard_output(
        "minimax",
        "test-api-key",
        vec![make_model("MiniMax-M2.7", "MiniMax M2.7", 1000, 1000)],
    );
    write_wizard_config_to(&output1, tmp.path()).unwrap();
    // Second write: 2 different models (MiniMax-M2.7 NOT selected)
    let output2 = make_wizard_output(
        "minimax",
        "test-api-key-2",
        vec![
            make_model("abab6.5-chat", "ABAB6.5 Chat", 2000, 2000),
            make_model("abab6.5-chat-pro", "ABAB6.5 Chat Pro", 4000, 4000),
        ],
    );
    write_wizard_config_to(&output2, tmp.path()).unwrap();
    // Verify: first model preserved alongside 2 new ones = 3 total
    let parsed = read_parsed_config(&tmp);
    let provider = parsed.providers.get("minimax").unwrap();
    assert_eq!(
        provider.models.len(),
        3,
        "should have 3 models (1 existing + 2 new), got: {}",
        provider.models.len()
    );
    let ids: Vec<&str> = provider.models.iter().map(|m| m.id.as_str()).collect();
    assert!(
        ids.contains(&"MiniMax-M2.7"),
        "first write's model MiniMax-M2.7 should be preserved, got: {:?}",
        ids
    );
}

/// Test: re-running wizard with same model replaces it entirely
#[test]
fn test_write_wizard_config_to_replaces_existing_model_by_id() {
    let tmp = TempDir::new().unwrap();

    // First write: MiniMax-M2.7 with old name
    let output1 = WizardOutput {
        provider_id: "minimax".to_string(),
        credential: "test-api-key".to_string(),
        selected_models: vec![ModelInfo {
            id: "MiniMax-M2.7".to_string(),
            name: "Old Name".to_string(),
            context_window: 1000,
            max_tokens: 1000,
            default_temperature: None,
            reasoning: false,
            reasoning_levels: ReasoningLevels::None,
            input_types: vec![],
        }],
    };
    write_wizard_config_to(&output1, tmp.path()).unwrap();

    // Second write: same id, new name
    let output2 = WizardOutput {
        provider_id: "minimax".to_string(),
        credential: "test-api-key".to_string(),
        selected_models: vec![ModelInfo {
            id: "MiniMax-M2.7".to_string(),
            name: "New Name".to_string(),
            context_window: 2000,
            max_tokens: 2000,
            default_temperature: None,
            reasoning: false,
            reasoning_levels: ReasoningLevels::None,
            input_types: vec![],
        }],
    };
    write_wizard_config_to(&output2, tmp.path()).unwrap();

    // Verify: still 1 model, name updated to New Name
    let models_path = tmp.path().join("models.json");
    let content = std::fs::read_to_string(&models_path).unwrap();
    let parsed: ModelsConfigData = serde_json::from_str(&content).unwrap();
    let provider = parsed.providers.get("minimax").unwrap();
    assert_eq!(provider.models.len(), 1);
    assert_eq!(provider.models[0].name.as_deref(), Some("New Name"));
    assert_eq!(provider.models[0].enabled, Some(true));
}

/// Test: existing provider's base_url/api_key/api are preserved after rewrite
#[test]
fn test_write_wizard_config_to_preserves_provider_base_fields() {
    let tmp = TempDir::new().unwrap();
    let mut providers = std::collections::HashMap::new();
    providers.insert(
        "minimax".to_string(),
        ProviderConfig {
            base_url: Some("https://custom.api.com/v1".to_string()),
            api_key: Some("sk-existing-key".to_string()),
            api: Some("v2".to_string()),
            protocol: Some("openai".to_string()),
            credential_path: Some("credentials/minimax.json".to_string()),
            models: vec![ModelDefinition {
                id: "MiniMax-M2.7".to_string(),
                name: Some("MiniMax M2.7".to_string()),
                enabled: Some(true),
            }],
        },
    );
    write_initial_config(&tmp, providers);

    let output = make_wizard_output(
        "minimax",
        "new-api-key",
        vec![make_model("abab6.5-chat", "ABAB6.5 Chat", 2000, 2000)],
    );
    write_wizard_config_to(&output, tmp.path()).unwrap();

    let parsed = read_parsed_config(&tmp);
    let provider = parsed.providers.get("minimax").unwrap();
    assert_eq!(
        provider.base_url.as_deref(),
        Some("https://custom.api.com/v1")
    );
    assert_eq!(provider.api_key, None);
    assert_eq!(provider.api.as_deref(), Some("v2"));
}

/// Test: existing unselected models are preserved after rewrite
#[test]
fn test_write_wizard_config_to_preserves_unselected_models() {
    let tmp = TempDir::new().unwrap();
    let mut providers = std::collections::HashMap::new();
    providers.insert(
        "minimax".to_string(),
        ProviderConfig {
            base_url: None,
            api_key: None,
            api: None,
            protocol: None,
            credential_path: Some("credentials/minimax.json".to_string()),
            models: vec![
                make_model_def("model-a", "Model A", true),
                make_model_def("model-b", "Model B", true),
                make_model_def("model-c", "Model C", false),
            ],
        },
    );
    write_initial_config(&tmp, providers);

    let output = make_wizard_output(
        "minimax",
        "test-key",
        vec![
            make_model("model-a", "Model A Updated", 1000, 1000),
            make_model("model-d", "Model D", 3000, 3000),
        ],
    );
    write_wizard_config_to(&output, tmp.path()).unwrap();

    let parsed = read_parsed_config(&tmp);
    let provider = parsed.providers.get("minimax").unwrap();
    assert_eq!(provider.models.len(), 4);
    let ids: Vec<&str> = provider.models.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.contains(&"model-b"));
    assert!(ids.contains(&"model-c"));
    assert!(ids.contains(&"model-d"));
    let model_a = provider.models.iter().find(|m| m.id == "model-a").unwrap();
    assert_eq!(model_a.name.as_deref(), Some("Model A Updated"));
    assert_eq!(model_a.enabled, Some(true));
}

/// Test: write_wizard_config includes credentialPath in ProviderConfig
#[test]
fn test_write_wizard_config_includes_credential_path() {
    let tmp = TempDir::new().unwrap();
    let output = WizardOutput {
        provider_id: "minimax".to_string(),
        credential: "test-api-key".to_string(),
        selected_models: vec![ModelInfo {
            id: "MiniMax-M2.7".to_string(),
            name: "MiniMax M2.7".to_string(),
            context_window: 1000,
            max_tokens: 1000,
            default_temperature: None,
            reasoning: false,
            reasoning_levels: ReasoningLevels::None,
            input_types: vec![],
        }],
    };

    write_wizard_config_to(&output, tmp.path()).expect("write_wizard_config_to should succeed");

    let models_path = tmp.path().join("models.json");
    let content = std::fs::read_to_string(&models_path).expect("models.json should be written");

    let parsed: ModelsConfigData =
        serde_json::from_str(&content).expect("models.json should be valid JSON");

    let provider = parsed
        .providers
        .get("minimax")
        .expect("minimax provider should exist");
    assert_eq!(
        provider.credential_path.as_deref(),
        Some("credentials/minimax.json"),
        "credentialPath should be set to credentials/<provider_id>.json"
    );
}

/// Test: write_wizard_config handles empty selected_models gracefully
#[test]
fn test_write_wizard_config_with_no_selected_models() {
    let tmp = TempDir::new().unwrap();
    let output = WizardOutput {
        provider_id: "glm".to_string(),
        credential: "test-key".to_string(),
        selected_models: vec![],
    };

    // Should not panic — empty models is valid
    let result = write_wizard_config_to(&output, tmp.path());
    assert!(
        result.is_ok(),
        "empty selected_models should not cause error: {:?}",
        result
    );
}
