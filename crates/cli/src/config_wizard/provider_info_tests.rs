use super::types::{ProviderInfo, ProviderType};
use super::PROVIDERS;

#[test]
fn test_provider_info_serialize_with_description() {
    let info = ProviderInfo {
        id: "test",
        display_name: "Test Provider",
        provider_type: ProviderType::Minimax,
        description: "A test provider",
    };
    let json = serde_json::to_string(&info).unwrap();
    assert!(
        json.contains("description"),
        "JSON should contain description: {}",
        json
    );
    assert!(json.contains("A test provider"));
}

#[test]
fn test_provider_info_deserialize_with_description() {
    let json = r#"{
            "id": "test",
            "display_name": "Test Provider",
            "provider_type": "Minimax",
            "description": "A test provider"
        }"#;
    let info: ProviderInfo = serde_json::from_str(json).unwrap();
    assert_eq!(info.id, "test");
    assert_eq!(info.display_name, "Test Provider");
    assert_eq!(info.description, "A test provider");
}

#[test]
fn test_provider_info_roundtrip_with_description() {
    // &'static str fields require static source for deserialization
    let json = "{\"id\":\"test\",\"display_name\":\"Test\",
             \"provider_type\":\"Deepseek\",\"description\":\"test description\"}";
    let roundtripped: ProviderInfo = serde_json::from_str(json).unwrap();
    assert_eq!(roundtripped.id, "test");
    assert_eq!(roundtripped.display_name, "Test");
    assert_eq!(roundtripped.description, "test description");
    assert_eq!(roundtripped.provider_type, ProviderType::Deepseek);
}

#[test]
fn test_providers_table_all_entries_have_description() {
    for provider in PROVIDERS {
        assert!(
            !provider.description.is_empty(),
            "Provider '{}' should have a non-empty description",
            provider.id
        );
    }
}
