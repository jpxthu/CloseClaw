//! Tests for Gateway construction and processor registry wiring.
//!
//! Verifies that `Gateway::new` and `Gateway::with_processor_registry`
//! behave correctly, and that `build_processor_registry` produces valid
//! processor chains.

use crate::{GatewayConfig, SessionManager};
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;

// ── Gateway::new ────────────────────────────────────────────────────────────

/// Gateway::new must initialize with no processor registry.
#[test]
fn test_gateway_new_has_no_processor_registry() {
    let config = GatewayConfig {
        name: "test-new-gw".to_string(),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, sm);
    let (inbound, outbound) = gw.processor_registry_len();
    assert_eq!(inbound, 0, "Gateway::new should have 0 inbound processors");
    assert_eq!(
        outbound, 0,
        "Gateway::new should have 0 outbound processors"
    );
}

// ── Gateway::with_processor_registry ────────────────────────────────────────

/// Gateway::with_processor_registry must store the registry and expose
/// correct inbound/outbound counts.
#[test]
fn test_gateway_with_processor_registry_stores_it() {
    use closeclaw_processor_chain::ProcessorRegistry;

    let config = GatewayConfig {
        name: "test-with-registry".to_string(),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let registry = ProcessorRegistry::new();
    let gw = crate::Gateway::with_processor_registry(config, sm, Arc::new(registry));
    let (inbound, outbound) = gw.processor_registry_len();
    // Empty registry: 0 inbound, 0 outbound
    assert_eq!(inbound, 0, "empty registry should have 0 inbound");
    assert_eq!(outbound, 0, "empty registry should have 0 outbound");
}

// ── build_processor_registry ────────────────────────────────────────────────

/// build_processor_registry must return a registry with non-zero inbound
/// and outbound chains for default config.
#[test]
fn test_build_processor_registry_returns_valid_registry() {
    let config = GatewayConfig {
        name: "test-build-registry".to_string(),
        ..Default::default()
    };
    let registry = crate::build_processor_registry(&config);
    assert!(
        registry.inbound_len() > 0,
        "build_processor_registry should produce inbound processors"
    );
    assert!(
        registry.outbound_len() > 0,
        "build_processor_registry should produce outbound processors"
    );
}

/// Gateway::with_processor_registry with a populated registry must reflect
/// the registry's chain counts.
#[test]
fn test_gateway_with_processor_registry_reflects_chain_counts() {
    let config = GatewayConfig {
        name: "test-populated-registry".to_string(),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let registry = crate::build_processor_registry(&config);
    let expected_inbound = registry.inbound_len();
    let expected_outbound = registry.outbound_len();
    let gw = crate::Gateway::with_processor_registry(config, sm, Arc::new(registry));
    let (inbound, outbound) = gw.processor_registry_len();
    assert_eq!(
        inbound, expected_inbound,
        "Gateway should reflect build_processor_registry inbound count"
    );
    assert_eq!(
        outbound, expected_outbound,
        "Gateway should reflect build_processor_registry outbound count"
    );
}
