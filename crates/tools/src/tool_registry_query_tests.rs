//! Trait-level tests for `get_tool_detail` and `list_tool_names_by_group`
//! on `ToolRegistryQuery`.

use super::*;
use closeclaw_common::tool_registry::ToolRegistryQuery;

/// Helper: create a registry and register a set of dummy tools.
async fn setup_registry_with_tools() -> ToolRegistryImpl {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "Read".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Read file contents".to_string(),
        is_deferred: false,
        is_read_only: true,
        is_destructive: false,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "Write".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Write file contents".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: true,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "Grep".to_string(),
        group: "search".to_string(),
        summary_text: "Search in files".to_string(),
        is_deferred: true,
        is_read_only: true,
        is_destructive: false,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "Search".to_string(),
        group: "search".to_string(),
        summary_text: "Web search".to_string(),
        is_deferred: false,
        is_read_only: true,
        is_destructive: false,
    })
    .await
    .unwrap();
    reg
}

/// Normal path: register tools, query by name via trait → get full
/// `ToolDescriptor` with correct summary.
#[tokio::test]
async fn test_get_tool_detail_returns_correct_descriptor() {
    let reg = setup_registry_with_tools().await;
    let q: &dyn ToolRegistryQuery = &reg;

    let desc = q.get_tool_detail("Read").await.unwrap();
    assert_eq!(desc.name, "Read");
    assert_eq!(desc.group, "file_ops");
    assert_eq!(desc.summary, "Read file contents");
    assert!(desc.flags.is_read_only);
    assert!(!desc.flags.is_destructive);
}

/// Normal path: query by name for a deferred tool also returns full details.
#[tokio::test]
async fn test_get_tool_detail_deferred_tool() {
    let reg = setup_registry_with_tools().await;
    let q: &dyn ToolRegistryQuery = &reg;

    let desc = q.get_tool_detail("Grep").await.unwrap();
    assert_eq!(desc.name, "Grep");
    assert_eq!(desc.group, "search");
    assert_eq!(desc.summary, "Search in files");
    assert!(desc.flags.is_deferred_by_default);
}

/// Boundary: query non-existent tool name returns `None`.
#[tokio::test]
async fn test_get_tool_detail_nonexistent() {
    let reg = setup_registry_with_tools().await;
    let q: &dyn ToolRegistryQuery = &reg;

    let result = q.get_tool_detail("NonExistent").await;
    assert!(result.is_none());
}

/// Boundary: empty registry → query returns `None`.
#[tokio::test]
async fn test_get_tool_detail_empty_registry() {
    let reg = ToolRegistry::new();
    let q: &dyn ToolRegistryQuery = &reg;

    let result = q.get_tool_detail("Read").await;
    assert!(result.is_none());
}

/// Normal path: query by group returns all tool names in that group.
#[tokio::test]
async fn test_list_tool_names_by_group_returns_all() {
    let reg = setup_registry_with_tools().await;
    let q: &dyn ToolRegistryQuery = &reg;

    let mut file_ops = q.list_tool_names_by_group("file_ops").await;
    file_ops.sort();
    assert_eq!(file_ops, vec!["Read", "Write"]);

    let mut search = q.list_tool_names_by_group("search").await;
    search.sort();
    assert_eq!(search, vec!["Grep", "Search"]);
}

/// Boundary: query empty group name returns empty Vec.
#[tokio::test]
async fn test_list_tool_names_by_group_empty_group() {
    let reg = setup_registry_with_tools().await;
    let q: &dyn ToolRegistryQuery = &reg;

    let result = q.list_tool_names_by_group("nonexistent").await;
    assert!(result.is_empty());
}

/// Boundary: empty registry → query returns empty Vec.
#[tokio::test]
async fn test_list_tool_names_by_group_empty_registry() {
    let reg = ToolRegistry::new();
    let q: &dyn ToolRegistryQuery = &reg;

    let result = q.list_tool_names_by_group("file_ops").await;
    assert!(result.is_empty());
}
