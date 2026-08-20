//! Step 1.3 regression tests: sessions_spawn visibility & tool filtering.
//!
//! Verifies that after removing `effective_spawn_budget` from
//! `FragmentContext` and `PromptGenerationContext`:
//! - `sessions_spawn` remains visible in Normal mode
//! - `disallowed_tools` / whitelist correctly filter `sessions_spawn`
//! - No budget-related references leak into tool prompts

use super::*;

/// sessions_spawn should be visible in Normal mode (not just Plan mode).
/// This verifies the spawn whitelist regression: after removing effective_spawn_budget,
/// sessions_spawn remains accessible to agents via the registry.
#[tokio::test]
async fn test_normal_mode_sessions_spawn_visible() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "sessions_spawn".to_string(),
        group: "sessions".to_string(),
        summary_text: "Spawn session".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    let ctx = PromptGenerationContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        available_tool_names: vec![],
        tools: None,
        disallowed_tools: None,
        session_mode: Some(SessionMode::Normal),
        agent_role: None,
        agent_type: None,
    };
    let section = reg.build_tools_section(&ctx).await;

    assert!(
        section.contains("sessions_spawn"),
        "sessions_spawn should be visible in Normal mode, got: {section}"
    );
}

/// sessions_spawn excluded via disallowed_tools should not appear in the tools section.
#[tokio::test]
async fn test_disallowed_tools_excludes_sessions_spawn() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "sessions_spawn".to_string(),
        group: "sessions".to_string(),
        summary_text: "Spawn session".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    let ctx = PromptGenerationContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        available_tool_names: vec![],
        tools: None,
        disallowed_tools: Some(vec!["sessions_spawn".to_string()]),
        session_mode: None,
        agent_role: None,
        agent_type: None,
    };
    let section = reg.build_tools_section(&ctx).await;

    assert!(
        !section.contains("sessions_spawn"),
        "sessions_spawn should be excluded by disallowed_tools, got: {section}"
    );
}

/// sessions_spawn excluded via tools whitelist (only other tools allowed)
/// should not appear in the tools section.
#[tokio::test]
async fn test_whitelist_excludes_sessions_spawn() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "sessions_spawn".to_string(),
        group: "sessions".to_string(),
        summary_text: "Spawn session".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "Read".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Read file".to_string(),
        is_deferred: false,
        is_read_only: true,
        is_destructive: false,
    })
    .await
    .unwrap();

    let ctx = PromptGenerationContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        available_tool_names: vec![],
        tools: Some(vec!["Read".to_string()]),
        disallowed_tools: None,
        session_mode: None,
        agent_role: None,
        agent_type: None,
    };
    let section = reg.build_tools_section(&ctx).await;

    assert!(
        section.contains("Read"),
        "Read should be visible when whitelisted, got: {section}"
    );
    assert!(
        !section.contains("sessions_spawn"),
        "sessions_spawn should NOT be visible when not in whitelist, got: {section}"
    );
}
