//! Tests for ImAdapterToolsRegistrar registration behavior.
//!
//! Verifies that the registrar registers exactly 22 Feishu sub-tools
//! with the correct names, groups, and deferred flags.

use closeclaw_tools::{ToolContext, ToolRegistrar, ToolRegistry};

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

#[tokio::test]
async fn test_im_adapter_registrar_registers_twenty_two_tools() {
    let registry = ToolRegistry::new();
    crate::ImAdapterToolsRegistrar::new()
        .register(&registry)
        .await
        .unwrap();

    let ctx = make_ctx();
    let descriptors = registry.list_descriptors(&ctx).await;
    assert_eq!(descriptors.len(), 22, "expected 22 feishu tools");
}

#[tokio::test]
async fn test_im_adapter_registrar_tool_names() {
    let registry = ToolRegistry::new();
    crate::ImAdapterToolsRegistrar::new()
        .register(&registry)
        .await
        .unwrap();

    let ctx = make_ctx();
    let descriptors = registry.list_descriptors(&ctx).await;
    let names: Vec<&str> = descriptors.iter().map(|d| d.name.as_str()).collect();

    let expected_names = [
        "feishu_im_user_message",
        "feishu_im_user_get_messages",
        "feishu_im_user_get_thread_messages",
        "feishu_search_user",
        "feishu_calendar_event",
        "feishu_calendar_event_attendee",
        "feishu_calendar_freebusy",
        "feishu_calendar_calendar",
        "feishu_task_task",
        "feishu_task_tasklist",
        "feishu_task_comment",
        "feishu_task_subtask",
        "feishu_bitable_app",
        "feishu_bitable_app_table",
        "feishu_bitable_app_table_record",
        "feishu_bitable_app_table_field",
        "feishu_bitable_app_table_view",
        "feishu_doc_comments",
        "feishu_doc_media",
        "feishu_search_doc_wiki",
        "feishu_drive_file",
        "feishu_sheet",
    ];

    for expected in &expected_names {
        assert!(
            names.contains(expected),
            "tool '{}' not found in {:?}",
            expected,
            names
        );
    }
}

#[tokio::test]
async fn test_im_adapter_registrar_tool_groups() {
    let registry = ToolRegistry::new();
    crate::ImAdapterToolsRegistrar::new()
        .register(&registry)
        .await
        .unwrap();

    let ctx = make_ctx();
    let descriptors = registry.list_descriptors(&ctx).await;
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
            "group '{}' not found in {:?}",
            expected_group,
            groups
        );
    }
}

#[tokio::test]
async fn test_im_adapter_registrar_all_deferred() {
    let registry = ToolRegistry::new();
    crate::ImAdapterToolsRegistrar::new()
        .register(&registry)
        .await
        .unwrap();

    let ctx = make_ctx();
    let descriptors = registry.list_descriptors(&ctx).await;

    for desc in &descriptors {
        assert!(desc.is_deferred, "tool '{}' should be deferred", desc.name);
    }
}

#[tokio::test]
async fn test_im_adapter_registrar_name_and_priority() {
    let registrar = crate::ImAdapterToolsRegistrar::new();
    assert_eq!(registrar.name(), "ImAdapterToolsRegistrar");
    assert_eq!(registrar.priority(), 4);
}

#[tokio::test]
async fn test_im_adapter_registrar_idempotent_via_conflict() {
    let registry = ToolRegistry::new();

    crate::ImAdapterToolsRegistrar::new()
        .register(&registry)
        .await
        .unwrap();

    let result = crate::ImAdapterToolsRegistrar::new()
        .register(&registry)
        .await;
    assert!(result.is_err());

    assert_eq!(registry.len_for_test().await, 22);
}

#[tokio::test]
async fn test_im_adapter_registrar_group_counts() {
    let registry = ToolRegistry::new();
    crate::ImAdapterToolsRegistrar::new()
        .register(&registry)
        .await
        .unwrap();

    let ctx = make_ctx();
    let descriptors = registry.list_descriptors(&ctx).await;

    let count = |group: &str| -> usize { descriptors.iter().filter(|d| d.group == group).count() };

    assert_eq!(count("feishu_im"), 4);
    assert_eq!(count("feishu_calendar"), 4);
    assert_eq!(count("feishu_task"), 4);
    assert_eq!(count("feishu_bitable"), 5);
    assert_eq!(count("feishu_doc"), 3);
    assert_eq!(count("feishu_drive"), 1);
    assert_eq!(count("feishu_sheet"), 1);
}
