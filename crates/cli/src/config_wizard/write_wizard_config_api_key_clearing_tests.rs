use super::*;
use closeclaw_llm::ReasoningLevels;
use tempfile::TempDir;

/// Test: write_wizard_config always sets api_key to None in models.json
#[test]
fn test_write_wizard_config_clears_api_key() {
    let tmp = TempDir::new().unwrap();
    let output = WizardOutput {
        provider_id: "glm".to_string(),
        credential: "my-secret-api-key".to_string(),
        selected_models: vec![ModelInfo {
            id: "glm-4".to_string(),
            name: "GLM 4".to_string(),
            context_window: 128_000,
            max_tokens: 4_096,
            default_temperature: Some(0.7),
            reasoning: false,
            reasoning_levels: ReasoningLevels::None,
            input_types: vec![],
        }],
    };

    write_wizard_config_to(&output, tmp.path()).unwrap();

    let models_path = tmp.path().join("models.json");
    let content = std::fs::read_to_string(&models_path).unwrap();
    let parsed: ModelsConfigData = serde_json::from_str(&content).unwrap();
    let provider = parsed.providers.get("glm").unwrap();

    assert!(
        provider.api_key.is_none(),
        "api_key should be None (cleared), got: {:?}",
        provider.api_key
    );
    assert_eq!(
        provider.credential_path.as_deref(),
        Some("credentials/glm.json"),
        "credential_path should reference the credentials file"
    );
}

/// Test: api_key is None even when existing config had api_key set
#[test]
fn test_write_wizard_config_clears_existing_api_key() {
    let tmp = TempDir::new().unwrap();
    let mut providers = std::collections::HashMap::new();
    providers.insert(
        "deepseek".to_string(),
        ProviderConfig {
            base_url: Some("https://api.deepseek.com".to_string()),
            api_key: Some("old-secret-key".to_string()),
            api: None,
            protocol: None,
            credential_path: Some("credentials/deepseek.json".to_string()),
            models: vec![],
        },
    );
    let initial = ModelsConfigData {
        mode: "merge".to_string(),
        providers,
    };
    std::fs::write(
        tmp.path().join("models.json"),
        serde_json::to_string_pretty(&initial).unwrap(),
    )
    .unwrap();

    let output = WizardOutput {
        provider_id: "deepseek".to_string(),
        credential: "new-key".to_string(),
        selected_models: vec![ModelInfo {
            id: "deepseek-v3".to_string(),
            name: "DeepSeek V3".to_string(),
            context_window: 64_000,
            max_tokens: 8_192,
            default_temperature: None,
            reasoning: true,
            reasoning_levels: ReasoningLevels::None,
            input_types: vec![],
        }],
    };

    write_wizard_config_to(&output, tmp.path()).unwrap();

    let parsed: ModelsConfigData =
        serde_json::from_str(&std::fs::read_to_string(tmp.path().join("models.json")).unwrap())
            .unwrap();
    let provider = parsed.providers.get("deepseek").unwrap();

    assert!(
        provider.api_key.is_none(),
        "api_key should be None after wizard rewrite, got: {:?}",
        provider.api_key
    );
    assert_eq!(
        provider.base_url.as_deref(),
        Some("https://api.deepseek.com")
    );
}
