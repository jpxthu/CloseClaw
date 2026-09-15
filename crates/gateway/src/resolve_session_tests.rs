//! Unit tests for `resolve_session` agent-id resolution priority.
//!
//! Test dimensions (explicit request agent_id routing, Step 1.3b):
//! 1. Normal: explicit `agent_id` in metadata wins over bot→Agent bindings
//! 2. Boundary: empty explicit `agent_id` falls back to bindings resolution
//! 3. Regression: no explicit agent_id → bindings hit resolves to bound agent
//! 4. Regression: no explicit agent_id, no binding → peer_id fallback

use crate::Gateway;
use closeclaw_common::processor::ProcessedMessage;
use std::collections::HashMap;

fn make_processed(agent_id: Option<&str>) -> ProcessedMessage {
    let mut metadata = HashMap::new();
    metadata.insert("peer_id".to_string(), "cli".to_string());
    metadata.insert("sender_id".to_string(), "user-1".to_string());
    if let Some(id) = agent_id {
        metadata.insert("agent_id".to_string(), id.to_string());
    }
    ProcessedMessage {
        content_blocks: vec![],
        metadata,
    }
}

fn bindings() -> HashMap<String, String> {
    let mut bindings = HashMap::new();
    bindings.insert("cli".to_string(), "bound-agent".to_string());
    bindings
}

/// Explicit request agent_id (terminal chat `--agent-id`) must win over
/// the bot→Agent binding on peer_id.
#[test]
fn test_explicit_agent_id_overrides_bindings() {
    let processed = make_processed(Some("master"));
    let message = Gateway::build_resolve_message(&processed, "terminal", &bindings());
    assert_eq!(message.to, "master");
    assert_eq!(message.from, "user-1");
    assert_eq!(message.channel, "terminal");
}

/// Empty explicit agent_id falls back to bindings resolution
/// (no routing to a nonexistent empty-named agent).
#[test]
fn test_empty_explicit_agent_id_falls_back_to_bindings() {
    let processed = make_processed(Some(""));
    let message = Gateway::build_resolve_message(&processed, "terminal", &bindings());
    assert_eq!(message.to, "bound-agent");
}

/// Without explicit agent_id, existing behavior is preserved:
/// bindings hit resolves to the bound agent.
#[test]
fn test_bindings_hit_without_explicit_agent_id() {
    let processed = make_processed(None);
    let message = Gateway::build_resolve_message(&processed, "terminal", &bindings());
    assert_eq!(message.to, "bound-agent");
}

/// Without explicit agent_id and without a binding, the peer_id fallback
/// (backward compatible) still applies.
#[test]
fn test_peer_id_fallback_without_explicit_agent_id() {
    let processed = make_processed(None);
    let message = Gateway::build_resolve_message(&processed, "feishu", &HashMap::new());
    assert_eq!(message.to, "cli");
}
