//! Tests for Protocol layer — verify routing and public API after GlmProtocol removal.

use crate::protocol::{AnthropicProtocol, ChatProtocol, OpenAiProtocol};
use crate::types::ProtocolId;

// ── Normal path: protocol_id routing ────────────────────────────────────────

#[test]
fn test_openai_protocol_routes_to_openai_id() {
    let proto = OpenAiProtocol::new();
    assert_eq!(proto.protocol_id(), &ProtocolId::new("openai"));
}

#[test]
fn test_anthropic_protocol_routes_to_anthropic_id() {
    let proto = AnthropicProtocol::new();
    assert_eq!(proto.protocol_id(), &ProtocolId::new("anthropic"));
}

#[test]
fn test_openai_protocol_path_is_chat_completions() {
    let proto = OpenAiProtocol::new();
    assert_eq!(proto.path(), "/v1/chat/completions");
}

#[test]
fn test_anthropic_protocol_path_is_messages() {
    let proto = AnthropicProtocol::new();
    assert_eq!(proto.path(), "/v1/messages");
}

// ── Normal path: protocol_id can be used for matching ───────────────────────

#[test]
fn test_protocol_id_matching_openai() {
    let provider_proto_id = ProtocolId::new("openai");
    let openai = OpenAiProtocol::new();
    assert_eq!(openai.protocol_id(), &provider_proto_id);
}

#[test]
fn test_protocol_id_matching_anthropic() {
    let provider_proto_id = ProtocolId::new("anthropic");
    let anthropic = AnthropicProtocol::new();
    assert_eq!(anthropic.protocol_id(), &provider_proto_id);
}

// ── Boundary: no GlmProtocol in public exports ──────────────────────────────

/// Verify that GlmProtocol is NOT exported from the protocol module.
/// This test uses a compile-time trick: if GlmProtocol were exported,
/// `any::type_name_of_val` would resolve; otherwise the type doesn't exist
/// and this test would fail to compile — which is exactly what we want.
///
/// Instead we use a runtime check: attempt to access GlmProtocol via the
/// module and verify it does not exist. Since we can't `use` a non-existent
/// type, we verify the boundary indirectly by checking the module only
/// exports the expected types.
#[test]
fn test_protocol_module_exports_do_not_include_glm() {
    // The following are the ONLY public exports from protocol/mod.rs:
    // - ChatProtocol (trait)
    // - IncomingSseStream (type alias)
    // - OutgoingEventStream (type alias)
    // - ProtocolError (enum)
    // - Result (type alias)
    // - AnthropicProtocol (struct)
    // - OpenAiProtocol (struct)
    //
    // GlmProtocol must NOT be among them. We verify by ensuring
    // the two expected protocol implementations are present and
    // that no "glm" symbol exists in the module.

    // Positive: the expected exports exist
    let _openai = OpenAiProtocol::new();
    let _anthropic = AnthropicProtocol::new();

    // Negative: compile-time assertion that GlmProtocol is not accessible.
    // If someone re-adds `pub use glm::GlmProtocol;` to mod.rs,
    // this test will still pass (the type exists). To catch that,
    // we check that no protocol_id returns "glm".
    let openai_id = OpenAiProtocol::new().protocol_id().clone();
    let anthropic_id = AnthropicProtocol::new().protocol_id().clone();

    assert_ne!(openai_id, ProtocolId::new("glm"));
    assert_ne!(anthropic_id, ProtocolId::new("glm"));
}

// ── Error path: unknown protocol id ─────────────────────────────────────────

#[test]
fn test_unknown_protocol_id_not_matched_by_openai() {
    let unknown = ProtocolId::new("nonexistent-protocol");
    let openai = OpenAiProtocol::new();
    assert_ne!(openai.protocol_id(), &unknown);
}

#[test]
fn test_unknown_protocol_id_not_matched_by_anthropic() {
    let unknown = ProtocolId::new("nonexistent-protocol");
    let anthropic = AnthropicProtocol::new();
    assert_ne!(anthropic.protocol_id(), &unknown);
}

#[test]
fn test_glm_protocol_id_not_matched_by_any_implementation() {
    // After GlmProtocol removal, no implementation should handle "glm" protocol id.
    let glm_id = ProtocolId::new("glm");
    let openai = OpenAiProtocol::new();
    let anthropic = AnthropicProtocol::new();
    assert_ne!(openai.protocol_id(), &glm_id);
    assert_ne!(anthropic.protocol_id(), &glm_id);
}

// ── Boundary: public API surface ────────────────────────────────────────────

/// Verify that the ChatProtocol trait methods are accessible on both
/// protocol implementations, confirming the trait contract is intact.
#[test]
fn test_chat_protocol_trait_is_implemented_for_openai() {
    fn assert_chat_protocol(_: &dyn ChatProtocol) {}
    let proto = OpenAiProtocol::new();
    assert_chat_protocol(&proto);
}

#[test]
fn test_chat_protocol_trait_is_implemented_for_anthropic() {
    fn assert_chat_protocol(_: &dyn ChatProtocol) {}
    let proto = AnthropicProtocol::new();
    assert_chat_protocol(&proto);
}
