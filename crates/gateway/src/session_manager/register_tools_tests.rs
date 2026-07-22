//! Unit tests for `SessionManager::register_tools` and `set_tool_register_fn`.
//!
//! These tests verify:
//! - No-op (with warning) when no callback is set.
//! - Correct delegation when a callback is set.
//! - Callback replacement on repeated `set_tool_register_fn` calls.

use super::register_tools::ToolRegisterFn;
use super::tests::make_test_mgr;
use closeclaw_common::{ToolRegistrarError, ToolRegistry, ToolRegistryQuery};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ── Mock ToolRegistry ──────────────────────────────────────────────────────

/// Minimal mock implementing the [`ToolRegistry`] super-trait surface
/// required by `register_tools`.  Only the trait-object methods needed
/// for compilation are stubbed; the mock itself does not store tools.
struct MockRegistry;

#[async_trait::async_trait]
impl ToolRegistryQuery for MockRegistry {
    async fn list_tool_names(&self) -> Vec<String> {
        vec![]
    }
    async fn get_tool_descriptors(
        &self,
        _agent_id: Option<&str>,
        _agent_tools: Option<&[String]>,
        _agent_disallowed_tools: Option<&[String]>,
    ) -> Vec<closeclaw_common::ToolDescriptor> {
        vec![]
    }
    async fn has_tool(&self, _name: &str) -> bool {
        false
    }
    async fn get_tool_schema(&self, _name: &str) -> Option<serde_json::Value> {
        None
    }
}

#[async_trait::async_trait]
impl ToolRegistry for MockRegistry {
    async fn register_any(
        &self,
        _tool: Box<dyn std::any::Any + Send + Sync>,
        _registrar_name: &str,
    ) -> Result<(), closeclaw_common::RegistryError> {
        Ok(())
    }
    fn freeze(&self) {}
    fn is_frozen(&self) -> bool {
        false
    }
    async fn build_index(&self) -> String {
        String::new()
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Create a callback that increments `call_count` and returns `Ok(())`.
fn ok_callback(cc: Arc<AtomicUsize>) -> ToolRegisterFn {
    Arc::new(move |_registry: &dyn ToolRegistry| {
        cc.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    })
}

/// Create a callback that returns an error.
fn err_callback(msg: &str) -> ToolRegisterFn {
    let msg = msg.to_string();
    Arc::new(move |_registry: &dyn ToolRegistry| {
        let msg = msg.clone();
        Box::pin(async move { Err(ToolRegistrarError::Internal(msg)) })
    })
}

// ── Tests ──────────────────────────────────────────────────────────────────

/// When no callback is set, `register_tools` returns `Ok(())` (no-op).
#[tokio::test]
async fn test_register_tools_no_callback_returns_ok() {
    let mgr = make_test_mgr(None);
    let registry = MockRegistry;
    let result = mgr.register_tools(&registry).await;
    assert!(
        result.is_ok(),
        "register_tools should return Ok when no callback is set"
    );
}

/// When a callback is set, `register_tools` invokes it and propagates its result.
#[tokio::test]
async fn test_register_tools_delegates_to_callback() {
    let mgr = make_test_mgr(None);
    let call_count = Arc::new(AtomicUsize::new(0));
    let func = ok_callback(Arc::clone(&call_count));

    mgr.set_tool_register_fn(func).await;
    let registry = MockRegistry;
    let result = mgr.register_tools(&registry).await;

    assert!(result.is_ok(), "register_tools should return Ok");
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "callback should be called exactly once"
    );
}

/// When a callback returns an error, `register_tools` propagates it.
#[tokio::test]
async fn test_register_tools_propagates_callback_error() {
    let mgr = make_test_mgr(None);
    let func = err_callback("test error");

    mgr.set_tool_register_fn(func).await;
    let registry = MockRegistry;
    let result = mgr.register_tools(&registry).await;

    assert!(
        result.is_err(),
        "register_tools should propagate callback error"
    );
    match result.unwrap_err() {
        ToolRegistrarError::Internal(msg) => assert_eq!(msg, "test error"),
        other => panic!("unexpected error variant: {:?}", other),
    }
}

/// Setting the callback twice replaces the previous one.
#[tokio::test]
async fn test_set_tool_register_fn_replaces_previous() {
    let mgr = make_test_mgr(None);
    let call_count = Arc::new(AtomicUsize::new(0));

    // First callback — should NOT be called after replacement.
    let func1: ToolRegisterFn = {
        let cc = Arc::clone(&call_count);
        Arc::new(move |_registry: &dyn ToolRegistry| {
            cc.fetch_add(100, Ordering::SeqCst); // would add 100 if called
            Box::pin(async { Ok(()) })
        })
    };

    // Second callback — should be called.
    let func2 = ok_callback(Arc::clone(&call_count));

    mgr.set_tool_register_fn(func1).await;
    mgr.set_tool_register_fn(func2).await;

    let registry = MockRegistry;
    let result = mgr.register_tools(&registry).await;
    assert!(result.is_ok());

    // Only the second callback should have been invoked.
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "only the second callback should be called"
    );
}

/// Calling `register_tools` without setting a callback first is safe
/// (no panic, returns Ok, and a warning is logged).
#[tokio::test]
async fn test_register_tools_before_set_is_noop() {
    let mgr = make_test_mgr(None);
    let registry = MockRegistry;
    // Should not panic and should return Ok.
    let result = mgr.register_tools(&registry).await;
    assert!(result.is_ok());
}
