//! Tests for the config wizard

use super::*;

use async_trait::async_trait;

use std::sync::atomic::{self, AtomicUsize};
use std::sync::Arc;

// ─────────────────────────────────────────────────────────────────
// Mock LLMProvider for testing fetch_models_with_retry
// ─────────────────────────────────────────────────────────────────

struct MockProvider {
    name: String,
    // What the fetch_model_list call should return
    result: std::sync::Mutex<
        std::sync::Arc<dyn Fn(&str) -> Result<Vec<ModelInfo>, LLMError> + Send + Sync>,
    >,
}

impl MockProvider {
    fn new(
        name: &str,
        f: impl Fn(&str) -> Result<Vec<ModelInfo>, LLMError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.to_string(),
            result: std::sync::Mutex::new(std::sync::Arc::new(f)),
        }
    }

    fn returning(result: impl Into<Result<Vec<ModelInfo>, LLMError>>) -> Self {
        let result = result.into();
        Self::new("mock", move |_| result.clone())
    }
}

#[async_trait]
impl LLMProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(
        &self,
        _request: crate::llm::ChatRequest,
    ) -> Result<crate::llm::ChatResponse, LLMError> {
        Err(LLMError::ApiError("mock not implemented".to_string()))
    }

    async fn fetch_model_list(&self, bearer_token: &str) -> Result<Vec<ModelInfo>, LLMError> {
        let f = self.result.lock().unwrap();
        f(bearer_token)
    }

    fn models(&self) -> Vec<&str> {
        vec![]
    }
}

fn model_infos() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "gpt-4".to_string(),
            name: "GPT-4".to_string(),
            context_window: 8192,
            max_tokens: 8192,
            default_temperature: None,
            reasoning: false,
            input_types: vec![],
        },
        ModelInfo {
            id: "gpt-3.5-turbo".to_string(),
            name: "GPT-3.5 Turbo".to_string(),
            context_window: 4096,
            max_tokens: 4096,
            default_temperature: None,
            reasoning: false,
            input_types: vec![],
        },
    ]
}

fn transient_err(msg: &str) -> LLMError {
    LLMError::ApiError(msg.to_string())
}

fn auth_err(msg: &str) -> LLMError {
    LLMError::AuthFailed(msg.to_string())
}

// ─────────────────────────────────────────────────────────────────
// Tests for fetch_models_with_retry
// ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod fetch_models_with_retry_tests {
    use super::*;

    /// Scenario: Provider returns models successfully on first call — no retry needed.
    #[tokio::test]
    async fn success_no_retry() {
        let provider: Arc<dyn LLMProvider> = Arc::new(MockProvider::returning(Ok(model_infos())));
        let cred = "test-token";
        let result = fetch_models_with_retry(&provider, cred).await;
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id, "gpt-4");
        assert_eq!(result[1].id, "gpt-3.5-turbo");
    }

    /// Scenario: Transient error on first call, succeeds on second call — one retry.
    #[tokio::test]
    async fn transient_retry_succeeds() {
        let attempt = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&attempt);
        let provider: Arc<dyn LLMProvider> = Arc::new(MockProvider::new("minimax", move |_| {
            let n = counter.fetch_add(1, atomic::Ordering::SeqCst);
            if n == 0 {
                Err(transient_err("500 internal error"))
            } else {
                Ok(model_infos())
            }
        }));
        let result = fetch_models_with_retry(&provider, "token").await;
        // Should have retried once and succeeded — 2 total calls
        let attempts = attempt.load(atomic::Ordering::SeqCst);
        assert_eq!(
            attempts, 2,
            "Expected 2 attempts (fail then success), got {}",
            attempts
        );
        assert_eq!(result.len(), 2);
    }

    /// Scenario: All attempts return transient errors — falls back to knowledge base.
    #[tokio::test]
    async fn transient_exhausted_fallback() {
        let attempt = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&attempt);
        let provider: Arc<dyn LLMProvider> = Arc::new(MockProvider::new("minimax", move |_| {
            counter.fetch_add(1, atomic::Ordering::SeqCst);
            Err(transient_err("500 internal error"))
        }));
        let result = fetch_models_with_retry(&provider, "token").await;
        // All 3 attempts exhausted, fallback to knowledge base — result comes from minimax KB
        let attempts = attempt.load(atomic::Ordering::SeqCst);
        assert_eq!(attempts, 3, "Expected 3 attempts, got {}", attempts);
        assert!(!result.is_empty());
    }

    /// Scenario: Auth error — immediately falls back to knowledge base (no retry).
    #[tokio::test]
    async fn auth_error_no_retry_immediate_fallback() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&call_count);
        let provider: Arc<dyn LLMProvider> = Arc::new(MockProvider::new("minimax", move |_| {
            counter.fetch_add(1, atomic::Ordering::SeqCst);
            Err(auth_err("invalid api key"))
        }));
        let result = fetch_models_with_retry(&provider, "token").await;
        let count = call_count.load(atomic::Ordering::SeqCst);
        assert_eq!(
            count, 1,
            "Auth error should not retry — only 1 call expected, got {}",
            count
        );
        assert!(!result.is_empty());
    }
}

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
    use super::*;
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
    use std::sync::Once;

    // Ensure HOME is set for tests that call config_dir()
    static SETUP: Once = Once::new();
    fn setup() {
        SETUP.call_once(|| {
            std::env::set_var("HOME", "/tmp");
        });
    }

    /// Helper: create a temporary directory and return its path.
    fn with_temp_dir(f: impl FnOnce(std::path::PathBuf)) {
        let tmp = std::env::temp_dir();
        let dir = tmp.join(format!("closeclaw_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        f(dir.clone());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Test: write_wizard_config outputs protocol field in ProviderConfig
    ///
    /// We intercept the config_dir() by patching HOME to a temp dir so
    /// write_wizard_config writes to a controlled location.
    #[test]
    fn test_write_wizard_config_includes_protocol() {
        setup();
        with_temp_dir(|dir| {
            // Set HOME so config_dir() resolves to our temp dir
            std::env::set_var("HOME", dir.to_str().unwrap());

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

            write_wizard_config(&output).expect("write_wizard_config should succeed");

            let models_path = dir.join(".closeclaw").join("config").join("models.json");
            let content =
                std::fs::read_to_string(&models_path).expect("models.json should be written");

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
        });
    }

    /// Test: write_wizard_config handles empty selected_models gracefully
    #[test]
    fn test_write_wizard_config_with_no_selected_models() {
        setup();
        with_temp_dir(|dir| {
            std::env::set_var("HOME", dir.to_str().unwrap());

            let output = WizardOutput {
                provider_id: "glm".to_string(),
                credential: "test-key".to_string(),
                selected_models: vec![],
            };

            // Should not panic — empty models is valid
            let result = write_wizard_config(&output);
            assert!(
                result.is_ok(),
                "empty selected_models should not cause error: {:?}",
                result
            );
        });
    }
}
