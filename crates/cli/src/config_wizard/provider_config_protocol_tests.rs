use closeclaw_config::providers::models::{ModelDefinition, ProviderConfig};

#[test]
fn test_provider_config_serializes_with_protocol() {
    let config = ProviderConfig {
        base_url: Some("https://api.example.com".to_string()),
        api_key: Some("sk-test".to_string()),
        api: Some("v1".to_string()),
        protocol: Some("openai".to_string()),
        credential_path: None,
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

#[test]
fn test_provider_config_serializes_with_protocol_null() {
    let config = ProviderConfig {
        base_url: None,
        api_key: None,
        api: None,
        protocol: None,
        credential_path: None,
        models: vec![],
    };
    let json = serde_json::to_string_pretty(&config).unwrap();
    assert!(json.contains("\"protocol\": null"));
}

/// Test: ProviderConfig deserializes with protocol field present
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

/// Test: ProviderConfig deserializes with protocol field absent (treated as None)
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

/// Test: ProviderConfig roundtrip: serialize -> deserialize preserves protocol
#[test]
fn test_provider_config_roundtrip_with_protocol() {
    let original = ProviderConfig {
        base_url: Some("https://api.test.com".to_string()),
        api_key: None,
        api: None,
        protocol: Some("openai".to_string()),
        credential_path: None,
        models: vec![],
    };
    let json = serde_json::to_string(&original).unwrap();
    let roundtripped: ProviderConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(roundtripped.protocol, original.protocol);
}
