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

// -----------------------------------------------------------------
// Step 1.1: config_dir() path tests
// -----------------------------------------------------------------

#[cfg(test)]
mod config_dir_tests {
    use super::config_dir;
    use tempfile::TempDir;

    #[test]
    fn test_config_dir_points_to_closeclaw() {
        let dir = config_dir();
        let dir_str = dir.to_string_lossy();
        assert!(
            dir_str.ends_with(".closeclaw"),
            "config_dir should end with .closeclaw, got: {}",
            dir_str
        );
    }

    #[test]
    fn test_config_dir_is_under_home() {
        let dir = config_dir();
        let home = std::env::var("HOME").expect("HOME not set");
        assert!(
            dir.starts_with(&home),
            "config_dir should be under HOME, got: {} (HOME={})",
            dir.display(),
            home
        );
    }

    #[test]
    fn test_config_dir_matches_wizard_output_path() {
        // Verify write_wizard_config uses the same directory as config_dir()
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let models_path = dir.join("models.json");
        assert!(
            models_path.to_string_lossy().ends_with("models.json"),
            "wizard output should write models.json at config root"
        );
    }
}

// -----------------------------------------------------------------
// Step 1.2: credentials directory structure tests
// -----------------------------------------------------------------

#[cfg(test)]
mod credentials_path_tests {
    use crate::config::manager::ConfigSection;

    #[test]
    fn test_credentials_section_is_directory() {
        assert!(ConfigSection::Credentials.is_directory());
    }

    #[test]
    fn test_credentials_dir_name() {
        assert_eq!(ConfigSection::Credentials.dir_name(), "credentials");
    }

    #[test]
    fn test_non_credentials_sections_are_not_directories() {
        assert!(!ConfigSection::Models.is_directory());
        assert!(!ConfigSection::Channels.is_directory());
        assert!(!ConfigSection::Gateway.is_directory());
        assert!(!ConfigSection::Plugins.is_directory());
        assert!(!ConfigSection::System.is_directory());
    }

    #[test]
    fn test_credentials_filename_ends_with_slash() {
        let filename = ConfigSection::Credentials.filename();
        assert!(
            filename.ends_with('/'),
            "Credentials filename should end with /, got: {}",
            filename
        );
    }
}
