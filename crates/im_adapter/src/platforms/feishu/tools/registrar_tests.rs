//! Tests for ImAdapterToolsRegistrar registration behavior.
//!
//! Verifies that the registrar registers exactly 7 Feishu tools with the
//! correct names, groups, and deferred flags.

use closeclaw_tools::{ToolContext, ToolRegistrar, ToolRegistry};

fn make_ctx() -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
        session_mode: None,
    }
}

#[tokio::test]
async fn test_im_adapter_registrar_registers_seven_tools() {
    let registry = ToolRegistry::new();
    crate::ImAdapterToolsRegistrar::new()
        .register(&registry)
        .await
        .unwrap();

    let ctx = make_ctx();
    let descriptors = registry.list_descriptors(&ctx).await;
    assert_eq!(descriptors.len(), 7, "expected 7 feishu tools");
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

    for expected in &[
        "FeishuIm",
        "FeishuCalendar",
        "FeishuTask",
        "FeishuBitable",
        "FeishuDoc",
        "FeishuDrive",
        "FeishuSheet",
    ] {
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

    // First registration succeeds.
    crate::ImAdapterToolsRegistrar::new()
        .register(&registry)
        .await
        .unwrap();

    // Second registration should fail with Conflict.
    let result = crate::ImAdapterToolsRegistrar::new()
        .register(&registry)
        .await;
    assert!(result.is_err());

    // Count should still be 7.
    assert_eq!(registry.len_for_test().await, 7);
}
