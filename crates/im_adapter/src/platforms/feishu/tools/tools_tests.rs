use super::*;
use closeclaw_tools::{Tool, ToolContext, ToolRegistrar, ToolRegistry};

fn make_ctx() -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
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
    crate::ImAdapterToolsRegistrar::new()
        .register(&registry)
        .await
        .unwrap();

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
    crate::ImAdapterToolsRegistrar::new()
        .register(&registry)
        .await
        .unwrap();

    // Registering again should return Conflict error (tool already registered)
    let result = crate::ImAdapterToolsRegistrar::new()
        .register(&registry)
        .await;
    assert!(
        result.is_err(),
        "expected conflict on duplicate registration"
    );
    assert_eq!(registry.len_for_test().await, 7);
}

// ---------------------------------------------------------------------------
// Keywords validation integration tests (Step 1.2)
// Covers differences #2 and #3: keyword quantity rule (3-10) + quality check
// ---------------------------------------------------------------------------

/// Helper: extract keywords string from a detail() output that starts with
/// `[keywords: ...]`.
fn extract_keywords_from_detail(detail: &str) -> Vec<String> {
    assert!(
        detail.starts_with("[keywords:"),
        "detail must start with [keywords: ...], got: {}",
        &detail[..detail.len().min(80)]
    );
    // Find content between `[keywords: ` and `]`
    let after_prefix = &detail["[keywords:".len()..];
    let bracket_end = after_prefix.find(']').unwrap_or_else(|| {
        panic!(
            "missing closing ']' in keywords prefix, got: {}",
            &detail[..detail.len().min(80)]
        )
    });
    let kw_str = after_prefix[..bracket_end].trim();
    kw_str.split_whitespace().map(|s| s.to_string()).collect()
}

/// Helper: create all 7 Feishu tool instances for testing.
fn all_feishu_tools() -> Vec<(String, String)> {
    vec![
        (
            "FeishuCalendar".to_string(),
            FeishuCalendarTool::new().detail(),
        ),
        ("FeishuIm".to_string(), FeishuImTool::new().detail()),
        ("FeishuTask".to_string(), FeishuTaskTool::new().detail()),
        (
            "FeishuBitable".to_string(),
            FeishuBitableTool::new().detail(),
        ),
        ("FeishuSheet".to_string(), FeishuSheetTool::new().detail()),
        ("FeishuDrive".to_string(), FeishuDriveTool::new().detail()),
        ("FeishuDoc".to_string(), FeishuDocTool::new().detail()),
    ]
}

#[test]
fn test_all_feishu_tools_detail_starts_with_keywords_prefix() {
    for (name, detail) in all_feishu_tools() {
        assert!(
            detail.starts_with("[keywords:"),
            "{} detail() must start with [keywords: ...], got: {}",
            name,
            &detail[..detail.len().min(80)]
        );
    }
}

#[test]
fn test_all_feishu_tools_keyword_count_in_range() {
    for (name, detail) in all_feishu_tools() {
        let keywords = extract_keywords_from_detail(&detail);
        assert!(
            keywords.len() >= 3 && keywords.len() <= 10,
            "{}: keyword count {} is outside 3-10 range, keywords: {:?}",
            name,
            keywords.len(),
            keywords
        );
    }
}

#[test]
fn test_all_feishu_tools_keywords_have_no_punctuation() {
    let punctuation: &[char] = &[
        '.', ',', ';', ':', '!', '?', '(', ')', '[', ']', '{', '}', '"', '\'', '/', '\\', '@', '#',
        '$', '%', '&', '*', '+', '=', '|', '~', '`', '<', '>',
    ];
    for (name, detail) in all_feishu_tools() {
        let keywords = extract_keywords_from_detail(&detail);
        for kw in &keywords {
            assert!(
                !kw.chars().any(|c| punctuation.contains(&c)),
                "{}: keyword '{}' contains punctuation",
                name,
                kw
            );
        }
    }
}

#[test]
fn test_all_feishu_tools_keywords_are_lowercase_alphanumeric() {
    for (name, detail) in all_feishu_tools() {
        let keywords = extract_keywords_from_detail(&detail);
        for kw in &keywords {
            assert!(
                kw.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                "{}: keyword '{}' is not lowercase alphanumeric",
                name,
                kw
            );
        }
    }
}
