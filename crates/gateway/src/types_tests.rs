//! Unit tests for gateway types.
//!
//! Verifies that `default_inbound_queue_capacity` and `GatewayConfig::default`
//! align with the design doc default of 256.

use crate::GatewayConfig;

// ── default_inbound_queue_capacity ─────────────────────────────────────────

/// The inbound queue capacity helper must return 256 (design doc default).
#[test]
fn test_default_inbound_queue_capacity_is_256() {
    assert_eq!(super::default_inbound_queue_capacity(), 256);
}

// ── GatewayConfig::default ─────────────────────────────────────────────────

/// GatewayConfig::default() must set inbound_queue_capacity to 256.
#[test]
fn test_gateway_config_default_inbound_queue_capacity() {
    let config = GatewayConfig::default();
    assert_eq!(
        config.inbound_queue_capacity, 256,
        "GatewayConfig::default() inbound_queue_capacity must be 256"
    );
}

// ── serde deserialization boundary ─────────────────────────────────────────

/// Deserializing JSON without `inbound_queue_capacity` must default to 256.
#[test]
fn test_serde_deserialize_without_inbound_queue_capacity_defaults_to_256() {
    let json = r#"{
        "name": "test",
        "rate_limit_per_minute": 10,
        "max_message_size": 4096
    }"#;
    let config: GatewayConfig = serde_json::from_str(json).expect("deserialization should succeed");
    assert_eq!(
        config.inbound_queue_capacity, 256,
        "missing inbound_queue_capacity must default to 256"
    );
}

// ── bot_agent_bindings ─────────────────────────────────────────────────────

/// GatewayConfig::default() must set bot_agent_bindings to an empty map.
#[test]
fn test_gateway_config_default_bot_agent_bindings_empty() {
    let config = GatewayConfig::default();
    assert!(
        config.bot_agent_bindings.is_empty(),
        "GatewayConfig::default() bot_agent_bindings must be empty"
    );
}

/// Deserializing JSON with bot_agent_bindings must populate the map.
#[test]
fn test_serde_deserialize_with_bot_agent_bindings() {
    let json = r#"{
        "name": "test",
        "bot_agent_bindings": {
            "bot_x": "agent-a",
            "bot_y": "agent-b"
        }
    }"#;
    let config: GatewayConfig = serde_json::from_str(json).expect("deserialization should succeed");
    assert_eq!(config.bot_agent_bindings.len(), 2);
    assert_eq!(config.bot_agent_bindings["bot_x"], "agent-a");
    assert_eq!(config.bot_agent_bindings["bot_y"], "agent-b");
}

/// Deserializing JSON without bot_agent_bindings must default to empty map.
#[test]
fn test_serde_deserialize_without_bot_agent_bindings_defaults_to_empty() {
    let json = r#"{
        "name": "test"
    }"#;
    let config: GatewayConfig = serde_json::from_str(json).expect("deserialization should succeed");
    assert!(
        config.bot_agent_bindings.is_empty(),
        "missing bot_agent_bindings must default to empty map"
    );
}

/// Binding lookup: peer_id present in bindings returns the bound agent_id.
#[test]
fn test_bot_agent_binding_lookup_hit() {
    let mut bindings = std::collections::HashMap::new();
    bindings.insert("bot_x".to_string(), "agent-a".to_string());
    bindings.insert("bot_y".to_string(), "agent-b".to_string());

    let result = bindings
        .get("bot_x")
        .cloned()
        .unwrap_or_else(|| "bot_x".to_string());
    assert_eq!(
        result, "agent-a",
        "binding lookup must return bound agent_id"
    );
}

/// Binding lookup: peer_id not in bindings falls back to peer_id.
#[test]
fn test_bot_agent_binding_lookup_miss_fallback() {
    let mut bindings = std::collections::HashMap::new();
    bindings.insert("bot_x".to_string(), "agent-a".to_string());

    let peer_id = "unknown_bot";
    let result = bindings
        .get(peer_id)
        .cloned()
        .unwrap_or_else(|| peer_id.to_string());
    assert_eq!(
        result, "unknown_bot",
        "binding miss must fallback to peer_id"
    );
}

/// Empty bindings map: all lookups fall back to peer_id.
#[test]
fn test_bot_agent_binding_empty_map_fallback() {
    let bindings = std::collections::HashMap::<String, String>::new();

    let peer_id = "any_bot";
    let result = bindings
        .get(peer_id)
        .cloned()
        .unwrap_or_else(|| peer_id.to_string());
    assert_eq!(result, "any_bot", "empty bindings must fallback to peer_id");
}
