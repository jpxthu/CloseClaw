use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::json;

use crate::lazy_tool::{LazyTool, ToolMeta};
use crate::tool_registry::ToolFlags;
use crate::tool_trait::{Tool, ToolCallError, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// Stub tool for testing
// ---------------------------------------------------------------------------

/// Minimal tool stub — returns a configurable result.
struct StubTool {
    name: String,
    result: Result<ToolResult, ToolCallError>,
}

impl StubTool {
    fn ok(name: &str) -> Self {
        Self {
            name: name.to_string(),
            result: Ok(ToolResult {
                data: json!({"ok": true}),
                new_messages: vec![],
                context_modifier: None,
            }),
        }
    }

    fn not_implemented(name: &str) -> Self {
        Self {
            name: name.to_string(),
            result: Err(ToolCallError::NotImplemented),
        }
    }
}

#[async_trait::async_trait]
impl Tool for StubTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn group(&self) -> &str {
        "test_group"
    }
    fn summary(&self) -> String {
        format!("summary of {}", self.name)
    }
    fn detail(&self) -> String {
        format!("detail of {}", self.name)
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({})
    }
    async fn call(
        &self,
        _args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolCallError> {
        self.result.clone()
    }
    fn flags(&self) -> ToolFlags {
        ToolFlags::default()
    }
}

fn default_flags() -> ToolFlags {
    ToolFlags {
        is_deferred_by_default: true,
        ..ToolFlags::default()
    }
}

fn make_ctx() -> ToolContext {
    ToolContext {
        agent_id: "test".into(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    }
}

fn make_meta(name: &str) -> ToolMeta {
    ToolMeta {
        name: name.to_string(),
        group: "test_group".to_string(),
        summary: format!("summary of {}", name),
        detail: format!("detail of {}", name),
        input_schema: json!({"type": "object"}),
        flags: default_flags(),
    }
}

fn make_lazy(name: &str) -> LazyTool {
    let n = name.to_string();
    LazyTool::new(
        Box::new(move || Box::new(StubTool::ok(&n))),
        make_meta(name),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_first_call_triggers_factory() {
    let lazy = make_lazy("my_tool");
    let result = lazy.call(json!({}), &make_ctx()).await.unwrap();
    assert_eq!(result.data, json!({"ok": true}));
}

#[tokio::test]
async fn test_cached_after_first_call() {
    let count = Arc::new(AtomicUsize::new(0));
    let count_clone = count.clone();

    let lazy = LazyTool::new(
        Box::new(move || {
            count_clone.fetch_add(1, Ordering::SeqCst);
            Box::new(StubTool::ok("cached_tool"))
        }),
        ToolMeta {
            name: "cached_tool".to_string(),
            group: "grp".to_string(),
            summary: "sum".to_string(),
            detail: "det".to_string(),
            input_schema: json!({}),
            flags: ToolFlags::default(),
        },
    );

    // Call 3 times — factory should run only once.
    for _ in 0..3 {
        lazy.call(json!({}), &make_ctx()).await.unwrap();
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_metadata_returns_pre_stored_values() {
    let lazy = LazyTool::new(
        Box::new(|| Box::new(StubTool::ok("t"))),
        ToolMeta {
            name: "my_name".to_string(),
            group: "my_group".to_string(),
            summary: "my_summary".to_string(),
            detail: "my_detail".to_string(),
            input_schema: json!({"type": "string"}),
            flags: default_flags(),
        },
    );

    assert_eq!(lazy.name(), "my_name");
    assert_eq!(lazy.group(), "my_group");
    assert_eq!(lazy.summary(), "my_summary");
    assert_eq!(lazy.detail(), "my_detail");
    assert_eq!(lazy.input_schema(), json!({"type": "string"}));
    assert!(lazy.flags().is_deferred_by_default);
}

#[tokio::test]
async fn test_concurrent_calls_single_init() {
    let count = Arc::new(AtomicUsize::new(0));
    let count_clone = count.clone();

    let lazy = Arc::new(LazyTool::new(
        Box::new(move || {
            count_clone.fetch_add(1, Ordering::SeqCst);
            Box::new(StubTool::ok("concurrent_tool"))
        }),
        ToolMeta {
            name: "concurrent_tool".to_string(),
            group: "grp".to_string(),
            summary: "sum".to_string(),
            detail: "det".to_string(),
            input_schema: json!({}),
            flags: ToolFlags::default(),
        },
    ));

    let mut handles = Vec::new();
    for _ in 0..10 {
        let lazy = Arc::clone(&lazy);
        handles.push(tokio::spawn(async move {
            lazy.call(json!({}), &make_ctx()).await.unwrap();
        }));
    }

    for h in handles {
        h.await.unwrap();
    }
    // Factory should have run exactly once.
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_propagates_not_implemented() {
    let lazy = LazyTool::new(
        Box::new(|| Box::new(StubTool::not_implemented("ni_tool"))),
        ToolMeta {
            name: "ni_tool".to_string(),
            group: "grp".to_string(),
            summary: "sum".to_string(),
            detail: "det".to_string(),
            input_schema: json!({}),
            flags: ToolFlags::default(),
        },
    );

    let err = lazy.call(json!({}), &make_ctx()).await.unwrap_err();
    assert!(matches!(err, ToolCallError::NotImplemented));
}
