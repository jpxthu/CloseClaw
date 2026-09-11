//! Secondary detail expensive annotation tests (Step 1.3).
//!
//! Verifies that `ToolSearchTool::try_exact_match` appends `(expensive)`
//! to the detail text when the matched tool has `is_expensive == true`.

use super::*;

/// Helper: make a ToolDescriptor with configurable flags.
fn sec_desc(
    name: &str,
    group: &str,
    is_expensive: bool,
    is_deferred: bool,
) -> closeclaw_common::tool_registry::ToolDescriptor {
    closeclaw_common::tool_registry::ToolDescriptor {
        name: name.to_string(),
        group: group.to_string(),
        summary: format!("summary for {}", name),
        detail: format!("{} detail text", name),
        input_schema: serde_json::json!({}),
        flags: closeclaw_common::tool_registry::ToolFlags {
            is_concurrency_safe: true,
            is_read_only: false,
            is_destructive: false,
            is_expensive,
            is_deferred_by_default: is_deferred,
        },
    }
}

/// Helper: make a minimal mock registry for secondary detail tests.
struct MockReg {
    tools: tokio::sync::RwLock<
        std::collections::HashMap<String, closeclaw_common::tool_registry::ToolDescriptor>,
    >,
}

impl MockReg {
    fn new() -> Self {
        Self {
            tools: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    async fn insert(&self, desc: closeclaw_common::tool_registry::ToolDescriptor) {
        self.tools.write().await.insert(desc.name.clone(), desc);
    }
}

#[async_trait::async_trait]
impl closeclaw_common::tool_registry::ToolRegistryQuery for MockReg {
    async fn list_tool_names(&self) -> Vec<String> {
        self.tools.read().await.keys().cloned().collect()
    }

    async fn get_tool_descriptors(
        &self,
        _agent_id: Option<&str>,
        _agent_tools: Option<&[String]>,
        _agent_disallowed_tools: Option<&[String]>,
    ) -> Vec<closeclaw_common::tool_registry::ToolDescriptor> {
        self.tools.read().await.values().cloned().collect()
    }

    async fn has_tool(&self, name: &str) -> bool {
        self.tools.read().await.contains_key(name)
    }

    async fn get_tool_schema(&self, _name: &str) -> Option<serde_json::Value> {
        None
    }

    async fn get_tool_detail(
        &self,
        name: &str,
    ) -> Option<closeclaw_common::tool_registry::ToolDescriptor> {
        self.tools.read().await.get(name).cloned()
    }

    async fn list_tool_names_by_group(&self, _group: &str) -> Vec<String> {
        vec![]
    }

    async fn get_tool_concurrency_safe(&self, _name: &str) -> Option<bool> {
        None
    }

    async fn call_tool(
        &self,
        _name: &str,
        _args: serde_json::Value,
        _ctx: &closeclaw_common::ToolContext,
    ) -> Result<closeclaw_common::ToolResult, closeclaw_common::tool_trait::ToolCallError> {
        Err(closeclaw_common::tool_trait::ToolCallError::NotFound(
            "not implemented".into(),
        ))
    }
}

fn sec_ctx() -> crate::ToolContext {
    crate::ToolContext {
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

// ── Expensive eager tool: exact mode → (expensive) in secondary detail ───────

/// Expensive eager tool searched via ToolSearch exact mode
/// should have `(expensive)` appended to detail.
#[tokio::test]
async fn test_sec_detail_expensive_eager_appends_tag() {
    let reg = Arc::new(MockReg::new());
    reg.insert(sec_desc("Bash", "exec", true, false)).await;
    let tool = crate::builtin::search::ToolSearchTool::new(Arc::clone(&reg) as _);
    let result = tool
        .call(serde_json::json!({"query": "Bash"}), &sec_ctx())
        .await
        .unwrap();
    let detail = result.data["detail"].as_str().unwrap();
    assert!(
        detail.contains("(expensive)"),
        "expensive eager tool should have (expensive) in secondary detail, got: {detail}"
    );
    assert!(
        detail.contains("Bash detail text"),
        "detail text should be preserved, got: {detail}"
    );
}

// ── Expensive deferred tool: exact mode → (expensive) in secondary detail ────

/// Expensive deferred tool searched via ToolSearch exact mode
/// should have `(expensive)` appended to secondary detail.
#[tokio::test]
async fn test_sec_detail_expensive_deferred_appends_tag() {
    let reg = Arc::new(MockReg::new());
    reg.insert(sec_desc("GitStatus", "vcs", true, true)).await;
    let tool = crate::builtin::search::ToolSearchTool::new(Arc::clone(&reg) as _);
    let result = tool
        .call(serde_json::json!({"query": "GitStatus"}), &sec_ctx())
        .await
        .unwrap();
    let detail = result.data["detail"].as_str().unwrap();
    assert!(
        detail.contains("(expensive)"),
        "expensive deferred tool should have (expensive) in secondary detail, got: {detail}"
    );
}

// ── Non-expensive tool: exact mode → no (expensive) ──────────────────────────

/// Non-expensive tool searched via ToolSearch exact mode
/// should NOT have `(expensive)` in detail.
#[tokio::test]
async fn test_sec_detail_not_expensive_no_tag() {
    let reg = Arc::new(MockReg::new());
    reg.insert(sec_desc("Read", "file_ops", false, false)).await;
    let tool = crate::builtin::search::ToolSearchTool::new(Arc::clone(&reg) as _);
    let result = tool
        .call(serde_json::json!({"query": "Read"}), &sec_ctx())
        .await
        .unwrap();
    let detail = result.data["detail"].as_str().unwrap();
    assert!(
        !detail.contains("(expensive)"),
        "non-expensive tool should NOT have (expensive), got: {detail}"
    );
}

// ── new_messages content should match detail ─────────────────────────────────

/// The `new_messages` content from ToolSearch exact mode should also
/// contain the `(expensive)` annotation for expensive tools.
#[tokio::test]
async fn test_sec_detail_new_messages_contains_expensive() {
    let reg = Arc::new(MockReg::new());
    reg.insert(sec_desc("ToolSearch", "meta", true, false))
        .await;
    let tool = crate::builtin::search::ToolSearchTool::new(Arc::clone(&reg) as _);
    let result = tool
        .call(serde_json::json!({"query": "ToolSearch"}), &sec_ctx())
        .await
        .unwrap();
    assert!(!result.new_messages.is_empty());
    let msg_content = &result.new_messages[0].content;
    assert!(
        msg_content.contains("(expensive)"),
        "new_messages content should contain (expensive), got: {msg_content}"
    );
    // Verify data.detail matches new_messages content
    assert_eq!(
        result.data["detail"].as_str().unwrap(),
        msg_content.as_str(),
        "data.detail and new_messages content should match"
    );
}
