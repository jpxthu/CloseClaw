//! Dispatch-context test helpers: the inbound-validation sentinel cap and
//! the shared `ChatContext` factory.

use std::sync::Arc;

use closeclaw_gateway::types::GatewayConfig;
use closeclaw_gateway::{Gateway, SessionManager};

use crate::chat_rpc::{ChatContext, RpcTerminalPlugin};

// ── Shared dispatch context factory ───────────────────────────────────────

/// Test cap substituted for the default `max_message_size` = 0 by
/// [`apply_test_sentinels`]; keeps inbound validation from rejecting
/// non-empty test messages.
pub(crate) const TEST_MAX_MESSAGE_SIZE: usize = 64 * 1024;

/// Apply the inbound-validation sentinel to a test [`GatewayConfig`]: at the
/// config default of `max_message_size` = 0 any non-empty text message
/// exceeds the limit and is rejected by `validate_inbound`, so a sentinel 0
/// is replaced by [`TEST_MAX_MESSAGE_SIZE`]; an explicit non-zero cap passes
/// through unchanged.
pub(crate) fn apply_test_sentinels(config: GatewayConfig) -> GatewayConfig {
    if config.max_message_size == 0 {
        GatewayConfig {
            max_message_size: TEST_MAX_MESSAGE_SIZE,
            ..config
        }
    } else {
        config
    }
}

/// Sentinel contract — the default config (`max_message_size` = 0) gets the
/// test cap so `validate_inbound` accepts non-empty text.
#[test]
fn test_apply_test_sentinels_fills_default_sentinel() {
    let config = apply_test_sentinels(GatewayConfig::default());
    assert_eq!(config.max_message_size, TEST_MAX_MESSAGE_SIZE);
}

/// Sentinel contract — an explicit non-zero cap is kept as-is.
#[test]
fn test_apply_test_sentinels_keeps_explicit_cap() {
    let config = GatewayConfig {
        max_message_size: 4096,
        ..Default::default()
    };
    let effective = apply_test_sentinels(config);
    assert_eq!(effective.max_message_size, 4096);
}

/// Shared base harness for the dispatch tests (issue #3067):
/// `SessionManager` + `Gateway` + `RpcTerminalPlugin` assembled into a
/// [`ChatContext`]. Callers may pass `GatewayConfig::default()` and still
/// be safe for inbound validation — [`apply_test_sentinels`] substitutes
/// the cap.
pub(crate) fn make_dispatch_context(config: GatewayConfig) -> ChatContext {
    let config = apply_test_sentinels(config);
    let sessions = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        closeclaw_common::ReasoningLevel::default(),
    ));
    let gateway = Arc::new(Gateway::new_for_tests(config, sessions));
    let rpc_plugin = Arc::new(RpcTerminalPlugin::new());
    ChatContext {
        gateway,
        rpc_plugin,
    }
}
