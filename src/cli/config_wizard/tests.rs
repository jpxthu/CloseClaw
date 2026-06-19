//! Tests for the config wizard

use super::*;

#[cfg(test)]
mod parse_model_selection_tests {
    use super::parse_model_selection;

    #[test]
    fn single_number() {
        let result = parse_model_selection("3", 10).unwrap();
        assert_eq!(result, vec![2]);
    }

    #[test]
    fn space_separated() {
        let result = parse_model_selection("1 3 5", 10).unwrap();
        assert_eq!(result, vec![0, 2, 4]);
    }

    #[test]
    fn range() {
        let result = parse_model_selection("2-5", 10).unwrap();
        assert_eq!(result, vec![1, 2, 3, 4]);
    }

    #[test]
    fn mixed() {
        let result = parse_model_selection("1-3,5,7", 10).unwrap();
        assert_eq!(result, vec![0, 1, 2, 4, 6]);
    }

    #[test]
    fn all() {
        let result = parse_model_selection("all", 5).unwrap();
        assert_eq!(result, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn all_case_insensitive() {
        let result = parse_model_selection("ALL", 3).unwrap();
        assert_eq!(result, vec![0, 1, 2]);
    }

    #[test]
    fn deduplicates() {
        let result = parse_model_selection("1 1 1", 5).unwrap();
        assert_eq!(result, vec![0]);
    }

    #[test]
    fn sorted() {
        let result = parse_model_selection("5 3 1", 10).unwrap();
        assert_eq!(result, vec![0, 2, 4]);
    }

    #[test]
    fn empty_input_error() {
        let result = parse_model_selection("", 5);
        assert!(result.is_err());
    }

    #[test]
    fn zero_not_allowed() {
        let result = parse_model_selection("0", 5);
        assert!(result.is_err());
    }

    #[test]
    fn out_of_range() {
        let result = parse_model_selection("11", 10);
        assert!(result.is_err());
    }

    #[test]
    fn start_greater_than_end() {
        let result = parse_model_selection("5-2", 10);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_syntax() {
        let result = parse_model_selection("1-2-3", 10);
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod wizard_context_tests {
    use super::*;

    #[test]
    fn context_default_state() {
        let ctx = WizardContext::default();
        assert_eq!(ctx.current_state, WizardState::SelectProvider);
        assert!(ctx.selected_provider.is_none());
        assert!(ctx.credential.is_none());
        assert!(ctx.fetched_models.is_empty());
        assert!(ctx.selected_models.is_empty());
    }

    #[test]
    fn state_equality() {
        assert_eq!(WizardState::SelectProvider, WizardState::SelectProvider);
        assert_eq!(WizardState::InputCredential, WizardState::InputCredential);
        assert_ne!(WizardState::SelectProvider, WizardState::InputCredential);
    }
}

#[cfg(test)]
mod provider_config_protocol_tests {
    use crate::config::providers::models::{ModelDefinition, ProviderConfig};

    /// Test 1: ProviderConfig serializes with protocol field present
    #[test]
    fn test_provider_config_serializes_with_protocol() {
        let config = ProviderConfig {
            base_url: Some("https://api.example.com".to_string()),
            api_key: Some("sk-test".to_string()),
            api: Some("v1".to_string()),
            protocol: Some("openai".to_string()),
            models: vec![ModelDefinition {
                id: "gpt-4".to_string(),
                name: Some("GPT-4".to_string()),
                enabled: Some(true),
            }],
        };
        let json = serde_json::to_string_pretty(&config).unwrap();
        assert!(
            json.contains("protocol"),
            "JSON should contain protocol field: {}",
            json
        );
    }

    /// Test 2: ProviderConfig serializes with protocol field as null when None
    #[test]
    fn test_provider_config_serializes_with_protocol_null() {
        let config = ProviderConfig {
            base_url: None,
            api_key: None,
            api: None,
            protocol: None,
            models: vec![],
        };
        let json = serde_json::to_string_pretty(&config).unwrap();
        assert!(json.contains("\"protocol\": null"));
    }

    /// Test 3: ProviderConfig deserializes with protocol field present
    #[test]
    fn test_provider_config_deserializes_with_protocol() {
        let json = r#"{
            "baseUrl": "https://api.example.com",
            "apiKey": "sk-test",
            "api": "v1",
            "protocol": "anthropic",
            "models": [
                { "id": "gpt-4", "name": "GPT-4", "enabled": true }
            ]
        }"#;
        let config: ProviderConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.protocol.as_deref(), Some("anthropic"));
    }

    /// Test 4: ProviderConfig deserializes with protocol field absent (treated as None)
    #[test]
    fn test_provider_config_deserializes_without_protocol() {
        let json = r#"{
            "baseUrl": "https://api.example.com",
            "apiKey": "sk-test",
            "models": []
        }"#;
        let config: ProviderConfig = serde_json::from_str(json).unwrap();
        assert!(config.protocol.is_none());
    }

    /// Test 5: ProviderConfig roundtrip: serialize -> deserialize preserves protocol
    #[test]
    fn test_provider_config_roundtrip_with_protocol() {
        let original = ProviderConfig {
            base_url: Some("https://api.test.com".to_string()),
            api_key: None,
            api: None,
            protocol: Some("openai".to_string()),
            models: vec![],
        };
        let json = serde_json::to_string(&original).unwrap();
        let roundtripped: ProviderConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped.protocol, original.protocol);
    }
}

#[cfg(test)]
mod write_wizard_config_tests {
    use super::*;
    use tempfile::TempDir;

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

    /// Test: write_wizard_config_to overwrites existing config (not appends)
    #[test]
    fn test_write_wizard_config_to_overwrites_existing() {
        let tmp = TempDir::new().unwrap();

        // First write: 1 model
        let output1 = WizardOutput {
            provider_id: "minimax".to_string(),
            credential: "test-api-key".to_string(),
            selected_models: vec![ModelInfo {
                id: "MiniMax-M2.7".to_string(),
                name: "MiniMax M2.7".to_string(),
                context_window: 1000,
                max_tokens: 1000,
                default_temperature: None,
                reasoning: false,
                input_types: vec![],
            }],
        };
        write_wizard_config_to(&output1, tmp.path()).unwrap();

        // Second write: 2 different models
        let output2 = WizardOutput {
            provider_id: "minimax".to_string(),
            credential: "test-api-key-2".to_string(),
            selected_models: vec![
                ModelInfo {
                    id: "abab6.5-chat".to_string(),
                    name: "ABAB6.5 Chat".to_string(),
                    context_window: 2000,
                    max_tokens: 2000,
                    default_temperature: None,
                    reasoning: false,
                    input_types: vec![],
                },
                ModelInfo {
                    id: "abab6.5-chat-pro".to_string(),
                    name: "ABAB6.5 Chat Pro".to_string(),
                    context_window: 4000,
                    max_tokens: 4000,
                    default_temperature: None,
                    reasoning: false,
                    input_types: vec![],
                },
            ],
        };
        write_wizard_config_to(&output2, tmp.path()).unwrap();

        // Verify: minimax provider should have exactly 2 models (overwrite, not append)
        let models_path = tmp.path().join("models.json");
        let content = std::fs::read_to_string(&models_path).unwrap();
        let parsed: ModelsConfigData = serde_json::from_str(&content).unwrap();
        let provider = parsed.providers.get("minimax").unwrap();
        assert_eq!(
            provider.models.len(),
            2,
            "should have exactly 2 models after overwrite, got: {}",
            provider.models.len()
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
}

#[cfg(test)]
mod ensure_master_agent_tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create isolated config_dir and agents_dir inside a TempDir.
    /// Returns (config_dir, agents_dir).
    fn isolated_dirs(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
        let config_dir = tmp.path().join("config");
        let agents_dir = tmp.path().join("agents");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&agents_dir).unwrap();
        (config_dir, agents_dir)
    }

    /// Helper: assert master agent config.json has expected defaults.
    fn assert_master_config(config_path: &std::path::Path) {
        assert!(config_path.exists(), "master config.json should exist");
        let content = std::fs::read_to_string(config_path).unwrap();
        let config: AgentConfig =
            serde_json::from_str(&content).expect("master config.json should be valid JSON");
        assert_eq!(config.id, "master");
        assert!(config.name.is_none());
        assert!(config.parent_id.is_none());
        assert!(config.model.is_none());
        assert!(config.workspace.is_none());
        assert!(config.agent_dir.is_none());
        assert_eq!(
            config.bootstrap_mode,
            crate::session::bootstrap::BootstrapMode::Full
        );
        assert_eq!(config.skills, vec!["*"]);
        assert_eq!(config.tools, vec!["*"]);
        assert!(config.disallowed_tools.is_empty());
    }

    /// Helper: assert agents.json contains exactly the given agent IDs.
    fn assert_agents_json(agents_json_path: &std::path::Path, expected: &[&str]) {
        assert!(agents_json_path.exists(), "agents.json should exist");
        let content = std::fs::read_to_string(agents_json_path).unwrap();
        let config: AgentsConfig =
            serde_json::from_str(&content).expect("agents.json should be valid JSON");
        let expected_owned: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        assert_eq!(config.agents, expected_owned);
    }

    /// Test 1: empty directory → both files created with correct content.
    #[test]
    fn test_creates_on_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let (config_dir, agents_dir) = isolated_dirs(&tmp);
        ensure_master_agent(&config_dir, &agents_dir).unwrap();

        let master_config_path = agents_dir.join("master").join("config.json");
        let agents_json_path = config_dir.join("agents.json");

        assert_master_config(&master_config_path);
        assert_agents_json(&agents_json_path, &["master"]);
    }

    /// Test 2: idempotent — calling twice does not modify existing files.
    #[test]
    fn test_idempotent() {
        let tmp = TempDir::new().unwrap();
        let (config_dir, agents_dir) = isolated_dirs(&tmp);
        ensure_master_agent(&config_dir, &agents_dir).unwrap();

        let master_config_path = agents_dir.join("master").join("config.json");
        let agents_json_path = config_dir.join("agents.json");

        let config_before = std::fs::read_to_string(&master_config_path).unwrap();
        let agents_before = std::fs::read_to_string(&agents_json_path).unwrap();

        // Second call should not change anything
        ensure_master_agent(&config_dir, &agents_dir).unwrap();

        let config_after = std::fs::read_to_string(&master_config_path).unwrap();
        let agents_after = std::fs::read_to_string(&agents_json_path).unwrap();

        assert_eq!(
            config_before, config_after,
            "config.json should not be modified"
        );
        assert_eq!(
            agents_before, agents_after,
            "agents.json should not be modified"
        );
    }

    /// Test 3: agents.json already has other agents → master is appended.
    #[test]
    fn test_appends_master_to_existing_agents_json() {
        let tmp = TempDir::new().unwrap();
        let (config_dir, agents_dir) = isolated_dirs(&tmp);

        let agents_json_path = config_dir.join("agents.json");
        std::fs::write(
            &agents_json_path,
            serde_json::to_string_pretty(&AgentsConfig {
                agents: vec!["helper".to_string(), "researcher".to_string()],
            })
            .unwrap(),
        )
        .unwrap();

        ensure_master_agent(&config_dir, &agents_dir).unwrap();

        // agents.json should now contain helper, researcher, master
        assert_agents_json(&agents_json_path, &["helper", "researcher", "master"]);

        // master config.json should also exist
        let master_config_path = agents_dir.join("master").join("config.json");
        assert_master_config(&master_config_path);
    }

    /// Test 4: config.json exists but agents.json missing master → master appended.
    #[test]
    fn test_agents_json_missing_master_only() {
        let tmp = TempDir::new().unwrap();
        let (config_dir, agents_dir) = isolated_dirs(&tmp);

        // Pre-create master config.json (already exists)
        std::fs::create_dir_all(agents_dir.join("master")).unwrap();
        let existing_config = AgentConfig {
            id: "master".to_string(),
            ..Default::default()
        };
        std::fs::write(
            agents_dir.join("master").join("config.json"),
            serde_json::to_string_pretty(&existing_config).unwrap(),
        )
        .unwrap();

        // Pre-create agents.json WITHOUT master
        let agents_json_path = config_dir.join("agents.json");
        std::fs::write(
            &agents_json_path,
            serde_json::to_string_pretty(&AgentsConfig {
                agents: vec!["helper".to_string()],
            })
            .unwrap(),
        )
        .unwrap();

        ensure_master_agent(&config_dir, &agents_dir).unwrap();

        // config.json should remain unchanged (not overwritten)
        let config_content =
            std::fs::read_to_string(agents_dir.join("master").join("config.json")).unwrap();
        let config: AgentConfig = serde_json::from_str(&config_content).unwrap();
        assert_eq!(config.id, "master");

        // agents.json should now include master
        assert_agents_json(&agents_json_path, &["helper", "master"]);
    }
}
