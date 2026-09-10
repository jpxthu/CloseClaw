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
// Helper: all 22 tool instances
// ---------------------------------------------------------------------------

fn all_feishu_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(FeishuImUserMessageTool::new()),
        Box::new(FeishuImUserGetMessagesTool::new()),
        Box::new(FeishuImUserGetThreadMessagesTool::new()),
        Box::new(FeishuSearchUserTool::new()),
        Box::new(FeishuCalendarEventTool::new()),
        Box::new(FeishuCalendarEventAttendeeTool::new()),
        Box::new(FeishuCalendarFreebusyTool::new()),
        Box::new(FeishuCalendarCalendarTool::new()),
        Box::new(FeishuTaskTaskTool::new()),
        Box::new(FeishuTaskTasklistTool::new()),
        Box::new(FeishuTaskCommentTool::new()),
        Box::new(FeishuTaskSubtaskTool::new()),
        Box::new(FeishuBitableAppTool::new()),
        Box::new(FeishuBitableAppTableTool::new()),
        Box::new(FeishuBitableAppTableRecordTool::new()),
        Box::new(FeishuBitableAppTableFieldTool::new()),
        Box::new(FeishuBitableAppTableViewTool::new()),
        Box::new(FeishuDocCommentsTool::new()),
        Box::new(FeishuDocMediaTool::new()),
        Box::new(FeishuSearchDocWikiTool::new()),
        Box::new(FeishuDriveFileTool::new()),
        Box::new(FeishuSheetTool::new()),
    ]
}

// Expected (name, group) pairs from the tools README table.
fn expected_tools() -> Vec<(&'static str, &'static str)> {
    vec![
        ("feishu_im_user_message", "feishu_im"),
        ("feishu_im_user_get_messages", "feishu_im"),
        ("feishu_im_user_get_thread_messages", "feishu_im"),
        ("feishu_search_user", "feishu_im"),
        ("feishu_calendar_event", "feishu_calendar"),
        ("feishu_calendar_event_attendee", "feishu_calendar"),
        ("feishu_calendar_freebusy", "feishu_calendar"),
        ("feishu_calendar_calendar", "feishu_calendar"),
        ("feishu_task_task", "feishu_task"),
        ("feishu_task_tasklist", "feishu_task"),
        ("feishu_task_comment", "feishu_task"),
        ("feishu_task_subtask", "feishu_task"),
        ("feishu_bitable_app", "feishu_bitable"),
        ("feishu_bitable_app_table", "feishu_bitable"),
        ("feishu_bitable_app_table_record", "feishu_bitable"),
        ("feishu_bitable_app_table_field", "feishu_bitable"),
        ("feishu_bitable_app_table_view", "feishu_bitable"),
        ("feishu_doc_comments", "feishu_doc"),
        ("feishu_doc_media", "feishu_doc"),
        ("feishu_search_doc_wiki", "feishu_doc"),
        ("feishu_drive_file", "feishu_drive"),
        ("feishu_sheet", "feishu_sheet"),
    ]
}

// ---------------------------------------------------------------------------
// Individual tool struct tests
// ---------------------------------------------------------------------------

#[test]
fn test_all_tools_count() {
    assert_eq!(all_feishu_tools().len(), 22);
}

#[test]
fn test_all_tools_name_and_group_match_doc() {
    let tools = all_feishu_tools();
    let expected = expected_tools();
    assert_eq!(tools.len(), expected.len());
    for (tool, &(exp_name, exp_group)) in tools.iter().zip(expected.iter()) {
        assert_eq!(tool.name(), exp_name, "tool name mismatch");
        assert_eq!(tool.group(), exp_group, "tool group mismatch");
    }
}

#[test]
fn test_all_tools_deferred() {
    for tool in all_feishu_tools() {
        let flags = tool.flags();
        assert!(
            flags.is_deferred_by_default,
            "tool '{}' should be deferred",
            tool.name()
        );
    }
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

    let ctx = make_ctx();
    let descriptors = registry.list_descriptors(&ctx).await;
    assert_eq!(descriptors.len(), 22, "expected 22 feishu tools registered");

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

    let result = crate::ImAdapterToolsRegistrar::new()
        .register(&registry)
        .await;
    assert!(
        result.is_err(),
        "expected conflict on duplicate registration"
    );
    assert_eq!(registry.len_for_test().await, 22);
}

// ---------------------------------------------------------------------------
// Keywords validation integration tests
// ---------------------------------------------------------------------------

fn extract_keywords_from_detail(detail: &str) -> Vec<String> {
    assert!(
        detail.starts_with("[keywords:"),
        "detail must start with [keywords: ...], got: {}",
        &detail[..detail.len().min(80)]
    );
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

#[test]
fn test_all_feishu_tools_detail_starts_with_keywords_prefix() {
    for tool in all_feishu_tools() {
        let detail = tool.detail();
        assert!(
            detail.starts_with("[keywords:"),
            "{} detail() must start with [keywords: ...], got: {}",
            tool.name(),
            &detail[..detail.len().min(80)]
        );
    }
}

#[test]
fn test_all_feishu_tools_keyword_count_in_range() {
    for tool in all_feishu_tools() {
        let detail = tool.detail();
        let keywords = extract_keywords_from_detail(&detail);
        assert!(
            keywords.len() >= 3 && keywords.len() <= 10,
            "{}: keyword count {} is outside 3-10 range, keywords: {:?}",
            tool.name(),
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
    for tool in all_feishu_tools() {
        let detail = tool.detail();
        let keywords = extract_keywords_from_detail(&detail);
        for kw in &keywords {
            assert!(
                !kw.chars().any(|c| punctuation.contains(&c)),
                "{}: keyword '{}' contains punctuation",
                tool.name(),
                kw
            );
        }
    }
}

#[test]
fn test_all_feishu_tools_keywords_are_lowercase_alphanumeric() {
    for tool in all_feishu_tools() {
        let detail = tool.detail();
        let keywords = extract_keywords_from_detail(&detail);
        for kw in &keywords {
            assert!(
                kw.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                "{}: keyword '{}' is not lowercase alphanumeric",
                tool.name(),
                kw
            );
        }
    }
}
