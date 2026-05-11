//! Tests for LegacyChatSession::build_fallback_chain

#[cfg(test)]
mod fallback_chain_tests {
    use crate::chat::session::LegacyChatSession;
    use tempfile::TempDir;

    #[test]
    fn test_build_fallback_chain_models_json_preferred() {
        // Arrange: create a temp dir with config/models.json containing enabled models
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path().to_str().unwrap();
        let config_subdir = tmp.path().join("config");
        std::fs::create_dir_all(&config_subdir).unwrap();
        let models_path = config_subdir.join("models.json");
        let models_json = serde_json::json!({
            "mode": "merge",
            "providers": {
                "openai": {
                    "baseUrl": "https://api.openai.com",
                    "models": [
                        { "id": "gpt-4", "enabled": true },
                        { "id": "gpt-3.5", "enabled": false }
                    ]
                },
                "anthropic": {
                    "models": [
                        { "id": "claude-3", "enabled": true },
                        { "id": "claude-2", "enabled": true }
                    ]
                }
            }
        });
        std::fs::write(&models_path, models_json.to_string()).unwrap();

        // Act
        let chain = LegacyChatSession::build_fallback_chain(Some(config_dir));

        // Assert: only enabled models, in "provider/model" format
        assert_eq!(chain.len(), 3);
        assert!(chain.contains(&"openai/gpt-4".to_string()));
        assert!(!chain.contains(&"openai/gpt-3.5".to_string()));
        assert!(chain.contains(&"anthropic/claude-3".to_string()));
        assert!(chain.contains(&"anthropic/claude-2".to_string()));
    }

    #[test]
    fn test_build_fallback_chain_env_fallback() {
        // Arrange: no config dir (None) and LLM_FALLBACK_CHAIN env var set
        std::env::set_var("LLM_FALLBACK_CHAIN", "openai/gpt-4, anthropic/claude-3");

        // Act
        let chain = LegacyChatSession::build_fallback_chain(None);

        // Assert
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0], "openai/gpt-4");
        assert_eq!(chain[1], "anthropic/claude-3");

        std::env::remove_var("LLM_FALLBACK_CHAIN");
    }

    #[test]
    fn test_build_fallback_chain_empty_models_fallback_to_env() {
        // Arrange: config dir with empty models.json, fall back to LLM_PROVIDER/LLM_MODEL
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path().to_str().unwrap();
        let config_subdir = tmp.path().join("config");
        std::fs::create_dir_all(&config_subdir).unwrap();
        let models_path = config_subdir.join("models.json");
        // Write a valid but empty models config
        std::fs::write(&models_path, r#"{"mode":"merge","providers":{}}"#).unwrap();

        // LLM_FALLBACK_CHAIN not set, LLM_PROVIDER/LLM_MODEL defaults
        std::env::remove_var("LLM_FALLBACK_CHAIN");
        std::env::set_var("LLM_PROVIDER", "minimax");
        std::env::set_var("LLM_MODEL", "MiniMax-M2.5");

        // Act
        let chain = LegacyChatSession::build_fallback_chain(Some(config_dir));

        // Assert
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0], "minimax/MiniMax-M2.5");

        std::env::remove_var("LLM_FALLBACK_CHAIN");
        std::env::remove_var("LLM_PROVIDER");
        std::env::remove_var("LLM_MODEL");
    }

    #[test]
    fn test_build_fallback_chain_models_json_missing_fallback_to_env() {
        // Arrange: config dir exists but has no models.json file
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path().to_str().unwrap();
        // Do NOT create config/models.json

        std::env::set_var("LLM_PROVIDER", "openai");
        std::env::set_var("LLM_MODEL", "gpt-4o");

        // Act
        let chain = LegacyChatSession::build_fallback_chain(Some(config_dir));

        // Assert: falls back to LLM_PROVIDER/LLM_MODEL
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0], "openai/gpt-4o");

        std::env::remove_var("LLM_PROVIDER");
        std::env::remove_var("LLM_MODEL");
    }
}
