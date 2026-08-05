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
