use super::*;
use crate::tools::{Tool, ToolContext, ToolRegistry};

fn make_ctx() -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
    }
}

// ---------------------------------------------------------------------------
// Individual tool struct tests
// ---------------------------------------------------------------------------

#[test]
fn test_feishu_im_tool_name() {
    let tool = FeishuImTool::new();
    assert_eq!(tool.name(), "FeishuIm");
}

#[test]
fn test_feishu_im_tool_group() {
    let tool = FeishuImTool::new();
    assert_eq!(tool.group(), "feishu_im");
}

#[test]
fn test_feishu_im_tool_flags() {
    let tool = FeishuImTool::new();
    let flags = tool.flags();
    assert!(flags.is_deferred_by_default);
}

#[test]
fn test_feishu_calendar_tool_name() {
    let tool = FeishuCalendarTool::new();
    assert_eq!(tool.name(), "FeishuCalendar");
}

#[test]
fn test_feishu_calendar_tool_group() {
    let tool = FeishuCalendarTool::new();
    assert_eq!(tool.group(), "feishu_calendar");
}

#[test]
fn test_feishu_calendar_tool_flags() {
    let tool = FeishuCalendarTool::new();
    let flags = tool.flags();
    assert!(flags.is_deferred_by_default);
}

#[test]
fn test_feishu_task_tool_name() {
    let tool = FeishuTaskTool::new();
    assert_eq!(tool.name(), "FeishuTask");
}

#[test]
fn test_feishu_task_tool_group() {
    let tool = FeishuTaskTool::new();
    assert_eq!(tool.group(), "feishu_task");
}

#[test]
fn test_feishu_task_tool_flags() {
    let tool = FeishuTaskTool::new();
    let flags = tool.flags();
    assert!(flags.is_deferred_by_default);
}

#[test]
fn test_feishu_bitable_tool_name() {
    let tool = FeishuBitableTool::new();
    assert_eq!(tool.name(), "FeishuBitable");
}

#[test]
fn test_feishu_bitable_tool_group() {
    let tool = FeishuBitableTool::new();
    assert_eq!(tool.group(), "feishu_bitable");
}

#[test]
fn test_feishu_bitable_tool_flags() {
    let tool = FeishuBitableTool::new();
    let flags = tool.flags();
    assert!(flags.is_deferred_by_default);
}

#[test]
fn test_feishu_doc_tool_name() {
    let tool = FeishuDocTool::new();
    assert_eq!(tool.name(), "FeishuDoc");
}

#[test]
fn test_feishu_doc_tool_group() {
    let tool = FeishuDocTool::new();
    assert_eq!(tool.group(), "feishu_doc");
}

#[test]
fn test_feishu_doc_tool_flags() {
    let tool = FeishuDocTool::new();
    let flags = tool.flags();
    assert!(flags.is_deferred_by_default);
}

#[test]
fn test_feishu_drive_tool_name() {
    let tool = FeishuDriveTool::new();
    assert_eq!(tool.name(), "FeishuDrive");
}

#[test]
fn test_feishu_drive_tool_group() {
    let tool = FeishuDriveTool::new();
    assert_eq!(tool.group(), "feishu_drive");
}

#[test]
fn test_feishu_drive_tool_flags() {
    let tool = FeishuDriveTool::new();
    let flags = tool.flags();
    assert!(flags.is_deferred_by_default);
}

#[test]
fn test_feishu_sheet_tool_name() {
    let tool = FeishuSheetTool::new();
    assert_eq!(tool.name(), "FeishuSheet");
}

#[test]
fn test_feishu_sheet_tool_group() {
    let tool = FeishuSheetTool::new();
    assert_eq!(tool.group(), "feishu_sheet");
}

#[test]
fn test_feishu_sheet_tool_flags() {
    let tool = FeishuSheetTool::new();
    let flags = tool.flags();
    assert!(flags.is_deferred_by_default);
}

// ---------------------------------------------------------------------------
// register_tools() integration test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_register_tools_populates_registry() {
    let registry = ToolRegistry::new();
    register_tools(&registry).await;

    // Registry should contain exactly 7 tools
    let ctx = make_ctx();
    let descriptors = registry.list_descriptors(&ctx).await;
    assert_eq!(descriptors.len(), 7, "expected 7 feishu tools registered");

    // Every tool should be deferred
    for desc in &descriptors {
        assert!(desc.is_deferred, "tool '{}' should be deferred", desc.name);
    }

    // Verify all group names present
    let groups: Vec<&str> = descriptors.iter().map(|d| d.group.as_str()).collect();
    for expected_group in &[
        "feishu_im",
        "feishu_calendar",
        "feishu_task",
        "feishu_bitable",
        "feishu_doc",
        "feishu_drive",
        "feishu_sheet",
    ] {
        assert!(
            groups.contains(expected_group),
            "group '{}' not found in registered tools",
            expected_group
        );
    }
}

#[tokio::test]
async fn test_register_tools_no_duplicates() {
    let registry = ToolRegistry::new();
    register_tools(&registry).await;

    // Calling register_tools again should hit AlreadyRegistered errors
    // (silently ignored via .ok()) but the count should stay at 7
    register_tools(&registry).await;
    assert_eq!(registry.len_for_test().await, 7);
}
