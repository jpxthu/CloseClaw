//! Shared test constructors for slash permission tests.
//!
//! Provides [`make_gateway`] and [`deny_engine`] so that
//! `tests_slash_permission.rs`, `slash_permission_routing_tests.rs`,
//! and `slash_permission_outbound_tests.rs` avoid duplicating
//! identical helper functions.

use std::sync::Arc;

use crate::{Gateway, GatewayConfig, SessionManager};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::{
    Action, Defaults, Effect, Rule, RuleSet, Subject,
};
use closeclaw_session::persistence::ReasoningLevel;

/// Create a minimal `Gateway` for unit tests.
///
/// Uses a zeroed rate-limit config and no persistence, suitable for
/// isolated permission-routing tests.
pub(crate) fn make_gateway() -> Arc<Gateway> {
    let config = GatewayConfig {
        name: "test".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    Arc::new(Gateway::new(config, sm))
}

/// Create a [`PermissionEngine`] that denies every action for every agent.
///
/// Useful for testing permission-denial paths without relying on
/// specific policy configuration.
pub(crate) fn deny_engine() -> Arc<tokio::sync::RwLock<PermissionEngine>> {
    let rules = RuleSet {
        rules: vec![Rule {
            name: "deny-all".to_owned(),
            subject: Subject::AgentOnly {
                agent: "*".to_owned(),
                match_type: Default::default(),
            },
            effect: Effect::Deny,
            actions: vec![Action::All],
            template: None,
            priority: 100,
        }],
        defaults: Defaults::default(),
        template_includes: vec![],
        ..Default::default()
    };
    Arc::new(tokio::sync::RwLock::new(
        PermissionEngine::new_with_default_data_root(rules),
    ))
}
