//! Tests for tool_registry injection through the gateway session build paths.
//!
//! Verifies that `SessionManager` propagates the tool_registry reference
//! into each `ConversationSession` during resolve, satisfying the
//! design doc §注入链路的参数契约 (must_fix #1).

use super::tests::{clear_global_prompt_state, make_test_mgr};
use closeclaw_common::tool_registry::ToolRegistryQuery;
use std::sync::Arc;

/// Helper: build a minimal Message for gateway tests.
fn test_message() -> crate::Message {
    crate::Message {
        id: "msg-tr".to_string(),
        from: "user".to_string(),
        to: "agent".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: std::collections::HashMap::new(),
        thread_id: None,
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    }
}

/// Fake ToolRegistryQuery for gateway tests.
struct FakeToolRegistryQuery;

#[async_trait::async_trait]
impl ToolRegistryQuery for FakeToolRegistryQuery {
    async fn list_tool_names(&self) -> Vec<String> {
        Vec::new()
    }
    async fn get_tool_descriptors(
        &self,
        _agent_id: Option<&str>,
        _agent_tools: Option<&[String]>,
        _agent_disallowed_tools: Option<&[String]>,
    ) -> Vec<closeclaw_common::ToolDescriptor> {
        Vec::new()
    }
    async fn has_tool(&self, _name: &str) -> bool {
        false
    }
    async fn get_tool_schema(&self, _name: &str) -> Option<serde_json::Value> {
        None
    }
    async fn get_tool_detail(&self, _name: &str) -> Option<closeclaw_common::ToolDescriptor> {
        None
    }
    async fn list_tool_names_by_group(&self, _group: &str) -> Vec<String> {
        Vec::new()
    }
}

/// After find_or_create, the session's tool_registry is set from SessionManager.
///
/// This verifies the gateway injects the registry into the session during
/// the resolve path (new session creation).
#[tokio::test]
async fn test_new_session_receives_tool_registry() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    let registry: Arc<dyn ToolRegistryQuery> = Arc::new(FakeToolRegistryQuery);
    let registry_ptr = Arc::as_ptr(&registry);
    mgr.set_tool_registry(registry).await;

    let session_id = mgr
        .find_or_create("feishu", &test_message(), None)
        .await
        .unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();
    let conv_guard = conv.read().await;

    // The session should have tool_registry set. We can verify by
    // rebuilding the system prompt (which passes registry through
    // InjectionParams) and checking the builder received it.
    // A simpler check: the session's tool_registry field is set.
    // Since tool_registry is private, we verify indirectly through
    // the system prompt builder mock that receives InjectionParams.

    // Use a capturing builder to verify the registry reaches the session.
    drop(conv_guard);

    // Set a capturing builder on the session.
    let captured = Arc::new(std::sync::Mutex::new(
        None::<closeclaw_common::injection_params::InjectionParams>,
    ));
    let capturing = ParamsCapturingBuilder {
        captured: captured.clone(),
    };
    {
        let mut conv_guard = conv.write().await;
        conv_guard.set_system_prompt_builder(Arc::new(capturing));
    }

    // Trigger rebuild — this constructs InjectionParams with the
    // session's tool_registry and passes it to the builder.
    {
        let mut conv_guard = conv.write().await;
        conv_guard
            .rebuild_system_prompt(&session_id, "test-agent", None)
            .await;
    }

    let params = captured.lock().unwrap();
    let p = params
        .as_ref()
        .expect("build_prompt_with_params should have been called");
    let reg = p
        .tool_registry
        .as_ref()
        .expect("tool_registry should be Some");
    assert_eq!(
        Arc::as_ptr(reg),
        registry_ptr,
        "registry in InjectionParams must be the same Arc as SessionManager's"
    );
}

/// After find_or_create, a session without tool_registry set on
/// SessionManager gets None in InjectionParams (no panic).
#[tokio::test]
async fn test_new_session_no_registry_no_panic() {
    clear_global_prompt_state();

    let mgr = make_test_mgr(None);
    // No set_tool_registry call — registry stays None.

    let session_id = mgr
        .find_or_create("feishu", &test_message(), None)
        .await
        .unwrap();
    let conv = mgr.get_conversation_session(&session_id).await.unwrap();

    let captured = Arc::new(std::sync::Mutex::new(
        None::<closeclaw_common::injection_params::InjectionParams>,
    ));
    {
        let mut conv_guard = conv.write().await;
        conv_guard.set_system_prompt_builder(Arc::new(ParamsCapturingBuilder {
            captured: captured.clone(),
        }));
    }

    {
        let mut conv_guard = conv.write().await;
        let prompt = conv_guard
            .rebuild_system_prompt(&session_id, "test-agent", None)
            .await;
        // Should not panic, and prompt should be returned.
        assert!(!prompt.is_empty());
    }

    let params = captured.lock().unwrap();
    let p = params.as_ref().unwrap();
    assert!(
        p.tool_registry.is_none(),
        "tool_registry should be None when SessionManager has no registry"
    );
}

// ── Test doubles ────────────────────────────────────────────────────────

use closeclaw_common::{
    injection_params::InjectionParams, BootstrapMode, PromptOverrides, SessionRole,
    SystemPromptBuilder,
};

struct ParamsCapturingBuilder {
    captured: Arc<std::sync::Mutex<Option<InjectionParams>>>,
}

#[async_trait::async_trait]
impl SystemPromptBuilder for ParamsCapturingBuilder {
    async fn build_prompt(
        &self,
        _session_id: &str,
        _agent_id: &str,
        _overrides: Option<&PromptOverrides>,
        _bootstrap_mode_override: Option<BootstrapMode>,
        _session_role: SessionRole,
    ) -> String {
        "default".to_string()
    }

    async fn build_prompt_with_params(&self, params: &InjectionParams) -> String {
        *self.captured.lock().unwrap() = Some(params.clone());
        "captured".to_string()
    }

    async fn invalidate_cache(&self) {}
}
