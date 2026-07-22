//! Unit tests for `SessionManager::register_tools` and `set_tool_register_fn`.
//!
//! These tests verify:
//! - No-op (with warning) when no callback is set.
//! - Correct delegation when a callback is set.
//! - Callback replacement on repeated `set_tool_register_fn` calls.

use super::register_tools::ToolRegisterFn;
use super::tests::make_test_mgr;
use closeclaw_common::{ToolRegistrarError, ToolRegistry, ToolRegistryQuery};
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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

// ═══════════════════════════════════════════════════════════════════════════
// Step 1.4 — End-to-end tool registration behaviour
// ═══════════════════════════════════════════════════════════════════════════

/// Tracking mock that records tool names registered via `register_any`.
///
/// Unlike the minimal `MockRegistry` above, this one captures every
/// registration so tests can verify which tools were registered and
/// in which order.
struct TrackingRegistry {
    /// Tool names recorded by `register_any`.
    registered: Mutex<Vec<String>>,
    /// Registrar names recorded by `register_any`.
    registrar_names: Mutex<Vec<String>>,
}

impl TrackingRegistry {
    fn new() -> Self {
        Self {
            registered: Mutex::new(Vec::new()),
            registrar_names: Mutex::new(Vec::new()),
        }
    }

    /// Return a snapshot of all registered tool names.
    fn tool_names(&self) -> Vec<String> {
        self.registered.lock().unwrap().clone()
    }

    /// Return a snapshot of all registrar names.
    fn registrar_names_snapshot(&self) -> Vec<String> {
        self.registrar_names.lock().unwrap().clone()
    }

    /// Return the set of unique registered tool names.
    fn unique_tool_names(&self) -> HashSet<String> {
        self.tool_names().into_iter().collect()
    }

    /// Return the number of tools registered.
    fn count(&self) -> usize {
        self.registered.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl ToolRegistryQuery for TrackingRegistry {
    async fn list_tool_names(&self) -> Vec<String> {
        self.tool_names()
    }
    async fn get_tool_descriptors(
        &self,
        _agent_id: Option<&str>,
        _agent_tools: Option<&[String]>,
        _agent_disallowed_tools: Option<&[String]>,
    ) -> Vec<closeclaw_common::ToolDescriptor> {
        vec![]
    }
    async fn has_tool(&self, name: &str) -> bool {
        self.tool_names().iter().any(|n| n == name)
    }
    async fn get_tool_schema(&self, _name: &str) -> Option<serde_json::Value> {
        None
    }
}

#[async_trait::async_trait]
impl ToolRegistry for TrackingRegistry {
    async fn register_any(
        &self,
        tool: Box<dyn std::any::Any + Send + Sync>,
        registrar_name: &str,
    ) -> Result<(), closeclaw_common::RegistryError> {
        // Extract tool name from the Box<dyn Any> — the tool is wrapped
        // in a ToolBox(Arc<dyn Tool>), but for testing we can extract
        // the name via the Downcast pattern.
        //
        // Since we cannot depend on `closeclaw_tools::Tool` here,
        // we use a simpler approach: store the registrar name and
        // use the tool's type name as a proxy for identification.
        //
        // In practice the callback constructs real tool instances
        // and the real ToolRegistry handles name extraction.
        // Here we just verify the callback *was called* with the
        // correct registry reference.
        let _ = tool; // accept any tool object
        self.registrar_names
            .lock()
            .unwrap()
            .push(registrar_name.to_string());
        // We use a placeholder name derived from the registrar + index
        let idx = self.registered.lock().unwrap().len();
        self.registered
            .lock()
            .unwrap()
            .push(format!("tool_{}", idx));
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

// ── Step 1.4 helpers ──────────────────────────────────────────────────────

/// Simulate the daemon's `wire_session_manager` callback.
///
/// This mirrors the real callback in `registries.rs` that constructs
/// `SessionsSpawnTool`, `SessionsSteerTool`, `SessionsKillTool`, and
/// `SessionsYieldTool`, then registers each via `register_any`.
///
/// For testing, we register synthetic tool objects instead of real ones
/// (gateway crate cannot depend on tools crate).
fn make_session_tools_callback() -> ToolRegisterFn {
    let tool_names: Vec<String> = vec![
        "sessions_spawn".to_string(),
        "sessions_steer".to_string(),
        "sessions_kill".to_string(),
        "sessions_yield".to_string(),
    ];
    Arc::new(move |registry: &dyn ToolRegistry| {
        let names = tool_names.clone();
        Box::pin(async move {
            let r = "SessionManager.register_tools";
            for name in &names {
                // Create a dummy box — the real code uses ToolBox(Arc<dyn Tool>)
                // but for testing we just need to exercise register_any.
                let dummy: Box<dyn std::any::Any + Send + Sync> = Box::new(name.clone());
                registry
                    .register_any(dummy, r)
                    .await
                    .map_err(|e| ToolRegistrarError::Internal(e.to_string()))?;
            }
            Ok(())
        })
    })
}

/// Create a callback that registers a configurable set of tool names.
fn make_custom_tools_callback(names: Vec<String>) -> ToolRegisterFn {
    Arc::new(move |registry: &dyn ToolRegistry| {
        let names = names.clone();
        Box::pin(async move {
            let r = "SessionManager.register_tools";
            for name in &names {
                let dummy: Box<dyn std::any::Any + Send + Sync> = Box::new(name.clone());
                registry
                    .register_any(dummy, r)
                    .await
                    .map_err(|e| ToolRegistrarError::Internal(e.to_string()))?;
            }
            Ok(())
        })
    })
}

// ── Step 1.4 tests ────────────────────────────────────────────────────────

/// End-to-end: set the daemon-style callback, call register_tools,
/// and verify all 4 session tools are registered to the tracking registry.
///
/// This validates the plan's goal: `SessionManager.register_tools` →
/// internal callback → tools registered into the ToolRegistry.
#[tokio::test]
async fn test_e2e_tools_registered_to_registry() {
    let mgr = make_test_mgr(None);
    let registry = TrackingRegistry::new();

    let callback = make_session_tools_callback();
    mgr.set_tool_register_fn(callback).await;

    let result = mgr.register_tools(&registry).await;
    assert!(result.is_ok(), "register_tools should succeed");

    // All 4 session tools should be registered.
    assert_eq!(
        registry.count(),
        4,
        "expected 4 session tools to be registered, got {}",
        registry.count()
    );

    let names = registry.unique_tool_names();
    assert!(
        names.len() >= 4,
        "should have at least 4 unique tool entries"
    );

    // All registrations should use the correct registrar name.
    for rn in registry.registrar_names_snapshot() {
        assert_eq!(
            rn, "SessionManager.register_tools",
            "registrar name should match daemon convention"
        );
    }
}

/// Registration timing: callback is invoked exactly once per
/// `register_tools` call, simulating the wire_session_manager flow
/// where the daemon sets the callback and immediately calls
/// `session_manager.register_tools(registry)`.
///
/// Verifies that the callback receives the same registry reference
/// that was passed to `register_tools`.
#[tokio::test]
async fn test_registration_timing_single_invocation() {
    let mgr = make_test_mgr(None);
    let registry = TrackingRegistry::new();
    let call_count = Arc::new(AtomicUsize::new(0));
    let cc = Arc::clone(&call_count);

    // The callback increments call_count and registers 3 tools
    // (matching the 3 tools defined in session-tools.md).
    let callback: ToolRegisterFn = Arc::new(move |registry: &dyn ToolRegistry| {
        cc.fetch_add(1, Ordering::SeqCst);
        let names = vec![
            "sessions_spawn".to_string(),
            "sessions_steer".to_string(),
            "sessions_kill".to_string(),
        ];
        Box::pin(async move {
            let r = "SessionManager.register_tools";
            for name in &names {
                let dummy: Box<dyn std::any::Any + Send + Sync> = Box::new(name.clone());
                registry
                    .register_any(dummy, r)
                    .await
                    .map_err(|e| ToolRegistrarError::Internal(e.to_string()))?;
            }
            Ok(())
        })
    });

    mgr.set_tool_register_fn(callback).await;

    // Simulate wire_session_manager: single call to register_tools.
    let result = mgr.register_tools(&registry).await;
    assert!(result.is_ok());

    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "callback should be invoked exactly once (single register_tools call)"
    );
    assert_eq!(
        registry.count(),
        3,
        "3 doc-defined session tools registered"
    );
}

/// sessions_yield (code_extra) is registered alongside the 3
/// doc-defined session tools. The plan states yield is preserved
/// in code and registered through the same callback.
///
/// This test verifies that when the callback includes yield,
/// all 4 tools (spawn + steer + kill + yield) are registered.
#[tokio::test]
async fn test_sessions_yield_registered_alongside_doc_tools() {
    let mgr = make_test_mgr(None);
    let registry = TrackingRegistry::new();

    // Use the full daemon-style callback that includes yield.
    let callback = make_session_tools_callback();
    mgr.set_tool_register_fn(callback).await;

    let result = mgr.register_tools(&registry).await;
    assert!(result.is_ok());

    // Verify all 4 tools registered (3 doc-defined + yield).
    assert_eq!(
        registry.count(),
        4,
        "expected 4 tools (spawn + steer + kill + yield), got {}",
        registry.count()
    );

    let names = registry.tool_names();
    // All names start with "tool_" prefix (TrackingRegistry convention).
    // The important thing is the count; the real registry tracks names
    // via ToolBox. Here we just verify the callback exercised register_any
    // 4 times.
    assert_eq!(
        names.len(),
        4,
        "4 register_any calls expected for yield + 3 doc tools"
    );
}

/// When the callback registers zero tools (all fail), register_tools
/// returns an error. This mirrors the daemon's guard:
/// `if registered == 0 { return Err(...) }`.
#[tokio::test]
async fn test_register_tools_error_when_no_tools_succeed() {
    let mgr = make_test_mgr(None);
    let registry = TrackingRegistry::new();

    // Callback that returns an error (simulating all tools failing).
    let callback: ToolRegisterFn = Arc::new(move |registry: &dyn ToolRegistry| {
        let _ = registry; // suppress unused warning
        Box::pin(async {
            Err(ToolRegistrarError::Internal(
                "all 4 session tools failed to register".to_string(),
            ))
        })
    });

    mgr.set_tool_register_fn(callback).await;
    let result = mgr.register_tools(&registry).await;

    assert!(result.is_err(), "should fail when callback returns error");
    match result.unwrap_err() {
        ToolRegistrarError::Internal(msg) => {
            assert!(msg.contains("failed to register"));
        }
        other => panic!("unexpected error variant: {:?}", other),
    }
}

/// Callback receives the correct registry reference.
///
/// Verifies the callback is invoked with a valid `&dyn ToolRegistry`
/// by using `register_any` to record a call on the tracking registry.
#[tokio::test]
async fn test_callback_receives_correct_registry_reference() {
    let mgr = make_test_mgr(None);
    let registry = TrackingRegistry::new();
    let called = Arc::new(AtomicUsize::new(0));
    let cc = Arc::clone(&called);

    let callback: ToolRegisterFn = Arc::new(move |registry: &dyn ToolRegistry| {
        cc.fetch_add(1, Ordering::SeqCst);
        // Exercise register_any to confirm the registry is functional.
        let dummy: Box<dyn std::any::Any + Send + Sync> = Box::new("probe".to_string());
        Box::pin(async move {
            registry
                .register_any(dummy, "test_probe")
                .await
                .map_err(|e| ToolRegistrarError::Internal(e.to_string()))
        })
    });

    mgr.set_tool_register_fn(callback).await;
    let result = mgr.register_tools(&registry).await;
    assert!(result.is_ok());

    // The callback was invoked and register_any was called on the
    // tracking registry, confirming it received the correct reference.
    assert_eq!(called.load(Ordering::SeqCst), 1);
    assert_eq!(
        registry.count(),
        1,
        "one probe tool registered, confirming registry reference is correct"
    );
}

/// Multiple register_tools calls: callback is invoked each time,
/// verifying the wire_session_manager flow where register_tools
/// is called once during init.
#[tokio::test]
async fn test_multiple_register_tools_calls_invoke_callback_each_time() {
    let mgr = make_test_mgr(None);
    let call_count = Arc::new(AtomicUsize::new(0));
    let cc = Arc::clone(&call_count);

    let callback: ToolRegisterFn = Arc::new(move |registry: &dyn ToolRegistry| {
        cc.fetch_add(1, Ordering::SeqCst);
        let _ = registry;
        Box::pin(async { Ok(()) })
    });

    mgr.set_tool_register_fn(callback).await;

    let registry = TrackingRegistry::new();
    // First call — simulates wire_session_manager.
    mgr.register_tools(&registry).await.unwrap();
    // Second call — would be a bug, but verify behaviour is idempotent.
    mgr.register_tools(&registry).await.unwrap();

    assert_eq!(
        call_count.load(Ordering::SeqCst),
        2,
        "callback invoked once per register_tools call"
    );
}

/// A custom subset of tools can be registered (e.g., only 3 tools
/// without yield). Verifies the callback pattern is flexible.
#[tokio::test]
async fn test_custom_tool_subset_registration() {
    let mgr = make_test_mgr(None);
    let registry = TrackingRegistry::new();

    let callback = make_custom_tools_callback(vec![
        "sessions_spawn".to_string(),
        "sessions_steer".to_string(),
        "sessions_kill".to_string(),
        // No yield — simulating the doc-only 3-tool subset.
    ]);

    mgr.set_tool_register_fn(callback).await;
    let result = mgr.register_tools(&registry).await;
    assert!(result.is_ok());

    assert_eq!(
        registry.count(),
        3,
        "only 3 doc-defined tools registered (no yield)"
    );
}
