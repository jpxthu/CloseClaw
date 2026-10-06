//! Tests for Gateway construction and processor registry wiring.
//!
//! Verifies that `Gateway::new` (injected chain), `Gateway::new_for_tests`
//! (chain-less convenience) and `Gateway::with_processor_registry` behave
//! correctly. The default-chain assembly itself lives in the composition
//! root (`closeclaw-daemon`) and is covered there.

use crate::{GatewayConfig, SessionManager};
use closeclaw_common::middleware::MiddlewareContext;
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;

// ── Gateway::new ────────────────────────────────────────────────────────────

/// Gateway::new must store the chain injected by the composition root.
#[test]
fn test_gateway_new_stores_injected_processor_chain() {
    use closeclaw_processor_chain::registry::ProcessorRegistry;
    use closeclaw_processor_chain::session_router::SessionRouter;

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
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(SessionRouter::new()));
    let (expected_inbound, expected_outbound) = (registry.inbound_len(), registry.outbound_len());
    let gw = crate::Gateway::new(config, sm, Arc::new(registry));
    assert_eq!(
        gw.processor_registry_len(),
        (expected_inbound, expected_outbound),
        "Gateway::new must keep the injected chain untouched"
    );
}

/// Gateway::new_for_tests (chain-less convenience) must take the bypass
/// path — no processor chain installed.
#[test]
fn test_gateway_new_for_tests_has_no_processor_chain() {
    let config = GatewayConfig {
        name: "test-no-chain-gw".to_string(),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new_for_tests(config, sm);
    assert_eq!(
        gw.processor_registry_len(),
        (0, 0),
        "chain-less convenience constructor installs no chain"
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

// ── populated registry reflection ───────────────────────────────────────────

/// Gateway::with_processor_registry with a populated registry must reflect
/// the registry's chain counts.
#[test]
fn test_gateway_with_processor_registry_reflects_chain_counts() {
    use closeclaw_processor_chain::registry::ProcessorRegistry;
    use closeclaw_processor_chain::session_router::SessionRouter;
    use closeclaw_processor_chain::verbosity_filter::VerbosityFilter;

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
    let mut registry = ProcessorRegistry::new();
    registry.register(Arc::new(SessionRouter::new()));
    registry.register(Arc::new(VerbosityFilter));
    let expected_inbound = registry.inbound_len();
    let expected_outbound = registry.outbound_len();
    let gw = crate::Gateway::with_processor_registry(config, sm, Arc::new(registry));
    let (inbound, outbound) = gw.processor_registry_len();
    assert_eq!(
        inbound, expected_inbound,
        "Gateway should reflect the injected registry inbound count"
    );
    assert_eq!(
        outbound, expected_outbound,
        "Gateway should reflect the injected registry outbound count"
    );
}

// ── Default outbound middleware registration ────────────────────────────────

/// Gateway::new must register the built-in audit and rate-limit middlewares.
#[tokio::test]
async fn test_gateway_new_registers_default_middlewares() {
    let config = GatewayConfig {
        name: "test-default-mw".to_string(),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new_for_tests(config, sm);
    let mws = gw.get_outbound_middlewares().await;
    assert_eq!(mws.len(), 2, "expected 2 default middlewares");
    assert_eq!(mws[0].name(), "audit");
    assert_eq!(mws[1].name(), "rate_limit");
}

/// Gateway::with_processor_registry must also register default middlewares.
#[tokio::test]
async fn test_gateway_with_processor_registry_registers_default_middlewares() {
    let config = GatewayConfig {
        name: "test-default-mw-registry".to_string(),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let registry = closeclaw_processor_chain::ProcessorRegistry::new();
    let gw = crate::Gateway::with_processor_registry(config, sm, Arc::new(registry));
    let mws = gw.get_outbound_middlewares().await;
    assert_eq!(mws.len(), 2, "expected 2 default middlewares");
    assert_eq!(mws[0].name(), "audit");
    assert_eq!(mws[1].name(), "rate_limit");
}

// ── Rate limit configuration passthrough (Step 1.3) ────────────────────────

fn rate_limit_ctx(session_id: &str) -> MiddlewareContext {
    MiddlewareContext {
        session_id: session_id.into(),
        channel: "feishu".into(),
        chat_id: "c1".into(),
    }
}

fn rate_limit_rendered() -> closeclaw_common::im_plugin::RenderedOutput {
    closeclaw_common::im_plugin::RenderedOutput {
        msg_type: "text".into(),
        payload: serde_json::json!({"content": {"text": "hi"}}),
    }
}

/// rate_limit_per_minute: 60 → middleware allows up to 60 messages.
#[tokio::test]
async fn test_rate_limit_config_custom_value_passthrough() {
    let config = GatewayConfig {
        name: "test-rate-limit-60".to_string(),
        rate_limit_per_minute: 60,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new_for_tests(config, sm);
    let mws = gw.get_outbound_middlewares().await;
    let rate_mw = mws.iter().find(|m| m.name() == "rate_limit").unwrap();

    let ctx = rate_limit_ctx("s1");
    let rendered = rate_limit_rendered();

    // Should allow up to 60 messages.
    for i in 0..60 {
        assert!(
            rate_mw.process(&ctx, &rendered).await.is_ok(),
            "message {i} should be allowed within limit of 60"
        );
    }
    // 61st message should be rejected.
    assert!(rate_mw.process(&ctx, &rendered).await.is_err());
}

/// rate_limit_per_minute: 0 → middleware uses default (30).
#[tokio::test]
async fn test_rate_limit_config_zero_fallback_to_default() {
    let config = GatewayConfig {
        name: "test-rate-limit-0".to_string(),
        rate_limit_per_minute: 0,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new_for_tests(config, sm);
    let mws = gw.get_outbound_middlewares().await;
    let rate_mw = mws.iter().find(|m| m.name() == "rate_limit").unwrap();

    let ctx = rate_limit_ctx("s1");
    let rendered = rate_limit_rendered();

    // Should allow up to 30 (default).
    for i in 0..30 {
        assert!(
            rate_mw.process(&ctx, &rendered).await.is_ok(),
            "message {i} should be allowed within default limit of 30"
        );
    }
    // 31st message should be rejected.
    assert!(rate_mw.process(&ctx, &rendered).await.is_err());
}

/// Default GatewayConfig (rate_limit_per_minute not set) → default 30.
#[tokio::test]
async fn test_rate_limit_config_default_unset_uses_30() {
    let config = GatewayConfig {
        name: "test-rate-limit-default".to_string(),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new_for_tests(config, sm);
    let mws = gw.get_outbound_middlewares().await;
    let rate_mw = mws.iter().find(|m| m.name() == "rate_limit").unwrap();

    let ctx = rate_limit_ctx("s1");
    let rendered = rate_limit_rendered();

    // Should allow up to 30 (default).
    for i in 0..30 {
        assert!(
            rate_mw.process(&ctx, &rendered).await.is_ok(),
            "message {i} should be allowed within default limit of 30"
        );
    }
    assert!(rate_mw.process(&ctx, &rendered).await.is_err());
}
