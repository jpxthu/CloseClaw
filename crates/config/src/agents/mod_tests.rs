//! Tests for agents module (moved out of mod.rs)

use super::*;

#[test]
fn test_valid_config() {
    let json = r#"{
        "agents": ["orchestrator", "coder", "tester"]
    }"#;
    let provider = AgentsConfigProvider::from_json_str(json).unwrap();
    provider.validate().unwrap();
    assert!(provider.lookup().contains_key("orchestrator"));
    assert!(provider.lookup().contains_key("coder"));
    assert!(provider.lookup().contains_key("tester"));
}

#[test]
fn test_duplicate_id_rejected() {
    let json = r#"{
        "agents": ["agent", "agent"]
    }"#;
    let provider = AgentsConfigProvider::from_json_str(json).unwrap();
    assert!(provider.validate().is_err());
}

#[test]
fn test_empty_id_rejected() {
    let json = r#"{
        "agents": [""]
    }"#;
    let provider = AgentsConfigProvider::from_json_str(json).unwrap();
    assert!(provider.validate().is_err());
}
