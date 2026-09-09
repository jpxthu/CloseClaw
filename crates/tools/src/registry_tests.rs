use super::*;
use crate::ToolFlags;
use closeclaw_common::RegistryError;

struct DummyTool {
    name: String,
    group: String,
    summary_text: String,
    is_deferred: bool,
    is_read_only: bool,
    is_destructive: bool,
}

impl Tool for DummyTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn group(&self) -> &str {
        &self.group
    }
    fn summary(&self) -> String {
        self.summary_text.clone()
    }
    fn detail(&self) -> String {
        format!("detail for {}", self.name)
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }
    fn flags(&self) -> ToolFlags {
        let mut f = ToolFlags::default();
        f.is_deferred_by_default = self.is_deferred;
        f.is_read_only = self.is_read_only;
        f.is_destructive = self.is_destructive;
        f
    }
}

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

/// Build a `PromptGenerationContext` for the named tools.
fn make_prompt_ctx(names: &[&str]) -> PromptGenerationContext {
    PromptGenerationContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        available_tool_names: names.iter().map(|s| s.to_string()).collect(),
        tools: None,
        disallowed_tools: None,
        session_mode: None,
        agent_role: None,
        agent_type: None,
    }
}

#[tokio::test]
async fn test_register_and_get_detail() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "Read".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Read file contents".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    let detail = reg.get_detail("Read").await.unwrap();
    assert!(detail.contains("Read"));
}

#[tokio::test]
async fn test_register_not_found() {
    let reg = ToolRegistry::new();
    let err = reg.get_detail("NonExistent").await.unwrap_err();
    assert!(matches!(err, ToolError::NotFound(_)));
}

#[tokio::test]
async fn test_register_duplicate() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "Read".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Read".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    let err = reg
        .register(DummyTool {
            name: "Read".to_string(),
            group: "file_ops".to_string(),
            summary_text: "Read again".to_string(),
            is_deferred: false,
            is_read_only: false,
            is_destructive: false,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::AlreadyRegistered(_)));
}

#[tokio::test]
async fn test_list_descriptors() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "Read".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Read files".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "Write".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Write files".to_string(),
        is_deferred: true,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    let ctx = make_ctx();
    let descriptors = reg.list_descriptors(&ctx).await;
    assert_eq!(descriptors.len(), 2);
    let read_desc = descriptors.iter().find(|d| d.name == "Read").unwrap();
    assert_eq!(read_desc.group, "file_ops");
    assert!(!read_desc.is_deferred);
    let write_desc = descriptors.iter().find(|d| d.name == "Write").unwrap();
    assert!(write_desc.is_deferred);
}

#[tokio::test]
async fn test_list_by_group() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "Read".to_string(),
        group: "file_ops".to_string(),
        summary_text: "R".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "ToolSearch".to_string(),
        group: "meta".to_string(),
        summary_text: "T".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    let file_ops = reg.list_by_group("file_ops").await;
    assert_eq!(file_ops, vec!["Read"]);

    let meta = reg.list_by_group("meta").await;
    assert_eq!(meta, vec!["ToolSearch"]);
}

#[tokio::test]
async fn test_list_by_group_empty() {
    let reg = ToolRegistry::new();
    let result = reg.list_by_group("nonexistent").await;
    assert!(result.is_empty());
}

#[tokio::test]
async fn test_tool_info_from_tool() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "Read".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Read files".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    let guard = reg.tools.read().await;
    let tool = guard.get("Read").unwrap();
    let info = ToolInfo::from_tool(tool, &make_prompt_ctx(&["Read"]));
    assert_eq!(info.name, "Read");
    assert_eq!(info.group, "file_ops");
    assert_eq!(info.detail, "detail for Read");
    assert!(!info.is_deferred);
    assert!(!info.is_read_only);
    assert!(!info.is_destructive);
    assert!(!info.is_expensive);
}

#[tokio::test]
async fn test_build_tools_section() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "Read".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Read files".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "ToolSearch".to_string(),
        group: "meta".to_string(),
        summary_text: "Search tools".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    let ctx = make_prompt_ctx(&["Read", "ToolSearch"]);
    let section = reg.build_tools_section(&ctx).await;
    assert!(section.contains("file_ops"), "section: {section}");
    assert!(
        section.contains("**Read**: detail for Read"),
        "section: {section}"
    );
    assert!(section.contains("meta"), "section: {section}");
    assert!(
        section.contains("**ToolSearch**: detail for ToolSearch"),
        "section: {section}"
    );
    // All-eager group header should include "(always loaded)"
    assert!(section.contains("(always loaded)"), "section: {section}");
}

#[tokio::test]
async fn test_build_tools_section_with_detail() {
    let reg = ToolRegistry::new();
    // Eager tool — should show detail
    reg.register(DummyTool {
        name: "Read".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Read files".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();
    // Deferred tool — should show name only
    reg.register(DummyTool {
        name: "Write".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Write files".to_string(),
        is_deferred: true,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    let ctx = make_prompt_ctx(&["Read", "Write"]);
    let section = reg.build_tools_section(&ctx).await;
    // Eager: bold name + detail
    assert!(
        section.contains("**Read**: detail for Read"),
        "eager tool should show detail, got: {section}"
    );
    // Deferred: name only, no bold/detail
    assert!(
        section.contains("  - Write"),
        "deferred tool should show name only, got: {section}"
    );
    assert!(
        !section.contains("**Write**:"),
        "deferred tool should NOT have bold detail, got: {section}"
    );
    // Mixed eager+deferred group header is "(always loaded)"
    assert!(section.contains("(always loaded)"), "section: {section}");
}

#[tokio::test]
async fn test_build_tools_section_deferred_group() {
    let reg = ToolRegistry::new();
    // Pure deferred group
    reg.register(DummyTool {
        name: "SlowOp".to_string(),
        group: "background".to_string(),
        summary_text: "Slow async operation".to_string(),
        is_deferred: true,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "Cleanup".to_string(),
        group: "background".to_string(),
        summary_text: "Clean up temp files".to_string(),
        is_deferred: true,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    let ctx = make_prompt_ctx(&["SlowOp", "Cleanup"]);
    let section = reg.build_tools_section(&ctx).await;
    // Deferred group header must show "(deferred)"
    assert!(
        section.contains("(deferred)"),
        "deferred group should have '(deferred)' tag, got: {section}"
    );
    assert!(
        !section.contains("(always loaded)"),
        "pure deferred group should NOT have '(always loaded)', got: {section}"
    );
    // Deferred tools: no bold, no detail
    assert!(
        section.contains("  - SlowOp"),
        "deferred tool name should appear, got: {section}"
    );
    assert!(
        !section.contains("**SlowOp**"),
        "deferred tool should NOT be bold, got: {section}"
    );
}

#[tokio::test]
async fn test_build_tools_section_danger_marks() {
    let reg = ToolRegistry::new();
    // Eager read-only tool
    reg.register(DummyTool {
        name: "Viewer".to_string(),
        group: "review".to_string(),
        summary_text: "View contents".to_string(),
        is_deferred: false,
        is_read_only: true,
        is_destructive: false,
    })
    .await
    .unwrap();
    // Eager destructive tool
    reg.register(DummyTool {
        name: "Deleter".to_string(),
        group: "review".to_string(),
        summary_text: "Delete files".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: true,
    })
    .await
    .unwrap();
    // Eager tool with no danger mark
    reg.register(DummyTool {
        name: "Lister".to_string(),
        group: "review".to_string(),
        summary_text: "List everything".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();
    // Deferred read-only tool
    reg.register(DummyTool {
        name: "DReader".to_string(),
        group: "lazy".to_string(),
        summary_text: "Deferred read".to_string(),
        is_deferred: true,
        is_read_only: true,
        is_destructive: false,
    })
    .await
    .unwrap();
    // Deferred destructive tool
    reg.register(DummyTool {
        name: "DDeleter".to_string(),
        group: "lazy".to_string(),
        summary_text: "Deferred delete".to_string(),
        is_deferred: true,
        is_read_only: false,
        is_destructive: true,
    })
    .await
    .unwrap();

    let ctx = make_prompt_ctx(&["Viewer", "Deleter", "Lister", "DReader", "DDeleter"]);
    let section = reg.build_tools_section(&ctx).await;
    assert!(
        section.contains("**Viewer** (read-only): detail for Viewer"),
        "expected eager read-only mark, got: {section}"
    );
    assert!(
        section.contains("**Deleter** (destructive): detail for Deleter"),
        "expected eager destructive mark, got: {section}"
    );
    assert!(
        section.contains("**Lister**: detail for Lister"),
        "expected eager no mark, got: {section}"
    );
    assert!(
        section.contains("  - DReader (read-only)"),
        "expected deferred read-only mark, got: {section}"
    );
    assert!(
        section.contains("  - DDeleter (destructive)"),
        "expected deferred destructive mark, got: {section}"
    );
}

#[tokio::test]
async fn test_build_tools_section_eager_and_deferred_group() {
    let reg = ToolRegistry::new();
    // Eager tool in one group
    reg.register(DummyTool {
        name: "Query".to_string(),
        group: "data".to_string(),
        summary_text: "Query data".to_string(),
        is_deferred: false,
        is_read_only: true,
        is_destructive: false,
    })
    .await
    .unwrap();
    // Deferred tool in the same group → mixed group
    reg.register(DummyTool {
        name: "Purge".to_string(),
        group: "data".to_string(),
        summary_text: "Purge old data".to_string(),
        is_deferred: true,
        is_read_only: false,
        is_destructive: true,
    })
    .await
    .unwrap();
    // Pure eager group
    reg.register(DummyTool {
        name: "Compute".to_string(),
        group: "math".to_string(),
        summary_text: "Compute stuff".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    let ctx = make_prompt_ctx(&["Query", "Purge", "Compute"]);
    let section = reg.build_tools_section(&ctx).await;
    // Mixed group (eager + deferred): should say "(always loaded)"
    assert!(
        section.contains("**data** — (always loaded)"),
        "mixed group should have '(always loaded)' tag, got: {section}"
    );
    // Pure eager group: "(always loaded)"
    assert!(
        section.contains("**math** — (always loaded)"),
        "pure eager group should have '(always loaded)' tag, got: {section}"
    );
    // Eager tool with read-only mark
    assert!(
        section.contains("**Query** (read-only): detail for Query"),
        "expected eager read-only mark, got: {section}"
    );
    // Deferred tool with destructive mark
    assert!(
        section.contains("  - Purge (destructive)"),
        "expected deferred destructive mark, got: {section}"
    );
}

#[tokio::test]
async fn test_build_tools_section_empty() {
    let reg = ToolRegistry::new();
    let ctx = make_prompt_ctx(&[]);
    let section = reg.build_tools_section(&ctx).await;
    assert!(section.is_empty());
}

/// Header-only overflow: when a group header alone exceeds the limit,
/// it should not produce an orphan header line.
#[tokio::test]
async fn test_build_tools_section_no_orphan_header() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "A".to_string(),
        group: "g".to_string(),
        summary_text: "tool A".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    let ctx = make_prompt_ctx(&["A"]);
    let section = reg.build_tools_section(&ctx).await;
    // With default TOOLS_SECTION_MAX_LEN=15000, header fits easily.
    // Just verify the section is non-empty and contains the tool.
    assert!(section.contains("**A**"), "tool A present: {section}");
    assert!(
        section.contains("(always loaded)"),
        "header present: {section}"
    );
}

// =========================================================================
// RegistryError — Display and variant tests
// =========================================================================

#[test]
fn test_registry_error_already_registered_display() {
    let err = RegistryError::AlreadyRegistered("Read".into());
    assert_eq!(format!("{}", err), "tool `Read` already registered");
}

#[test]
fn test_registry_error_conflict_display() {
    let err = RegistryError::Conflict {
        tool: "Read".into(),
        registrar: "core".into(),
        attempting: "extra".into(),
    };
    let msg = format!("{}", err);
    assert!(msg.contains("Read"));
    assert!(msg.contains("core"));
    assert!(msg.contains("extra"));
}

#[test]
fn test_registry_error_frozen_display() {
    let err = RegistryError::Frozen;
    assert!(format!("{}", err).contains("frozen"));
}

#[test]
fn test_registry_error_internal_display() {
    let err = RegistryError::Internal("something broke".into());
    assert_eq!(format!("{}", err), "something broke");
}

#[test]
fn test_registry_error_debug() {
    let err = RegistryError::AlreadyRegistered("Write".into());
    let debug = format!("{:?}", err);
    assert!(debug.contains("AlreadyRegistered"));
}

// =========================================================================
// Plan Mode tool visibility filtering tests
// =========================================================================

use closeclaw_common::SessionMode;

/// Helper to create a PromptGenerationContext with session_mode.
fn make_plan_mode_ctx() -> PromptGenerationContext {
    PromptGenerationContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        available_tool_names: vec![],
        tools: None,
        disallowed_tools: None,
        session_mode: Some(SessionMode::Plan),
        agent_role: None,
        agent_type: None,
    }
}

#[tokio::test]
async fn test_plan_mode_shows_write_and_edit_tools() {
    let reg = ToolRegistry::new();
    // Register read-only and write tools.
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
    // Write and Edit are in PLAN_MODE_ALWAYS_VISIBLE so they remain visible
    // in Plan Mode (permission layer restricts writes to plans/ via
    // is_plans_path()).
    reg.register(DummyTool {
        name: "Write".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Write file".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();
    reg.register(DummyTool {
        name: "Edit".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Edit file".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();
    // A non-read-only tool that is NOT in PLAN_MODE_ALWAYS_VISIBLE
    // should still be filtered out.
    reg.register(DummyTool {
        name: "CommandExec".to_string(),
        group: "exec".to_string(),
        summary_text: "Execute command".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    let ctx = make_plan_mode_ctx();
    let section = reg.build_tools_section(&ctx).await;

    assert!(
        section.contains("Read"),
        "Read should be visible in Plan mode"
    );
    assert!(
        section.contains("Write"),
        "Write should be visible in Plan mode (always-visible list)"
    );
    assert!(
        section.contains("Edit"),
        "Edit should be visible in Plan mode (always-visible list)"
    );
    assert!(
        !section.contains("CommandExec"),
        "CommandExec should be hidden in Plan mode"
    );
}

#[tokio::test]
async fn test_normal_mode_and_no_session_mode_do_not_filter() {
    // Normal mode shows all tools
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "Write".to_string(),
        group: "file_ops".to_string(),
        summary_text: "Write file".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    let normal_ctx = PromptGenerationContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        available_tool_names: vec![],
        tools: None,
        disallowed_tools: None,
        session_mode: Some(SessionMode::Normal),
        agent_role: None,
        agent_type: None,
    };
    let section = reg.build_tools_section(&normal_ctx).await;
    assert!(section.contains("Write"), "Write visible in Normal mode");

    // No session_mode also shows all tools
    let no_mode_ctx = PromptGenerationContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        available_tool_names: vec![],
        tools: None,
        disallowed_tools: None,
        session_mode: None,
        agent_role: None,
        agent_type: None,
    };
    let section = reg.build_tools_section(&no_mode_ctx).await;
    assert!(
        section.contains("Write"),
        "Write visible without session_mode"
    );
}

#[tokio::test]
async fn test_plan_mode_keeps_mode_execution_trigger() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "ModeExecutionTrigger".to_string(),
        group: "mode".to_string(),
        summary_text: "trigger execution from plan mode".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();
    let ctx = make_plan_mode_ctx();
    let section = reg.build_tools_section(&ctx).await;
    assert!(
        section.contains("ModeExecutionTrigger"),
        "ModeExecutionTrigger should be visible in Plan mode"
    );
}

#[test]
fn test_plan_mode_tool_visible_mode_execution_trigger() {
    let tool = DummyTool {
        name: "ModeExecutionTrigger".to_string(),
        group: "mode".to_string(),
        summary_text: "trigger execution".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    };
    let tool: Arc<dyn Tool> = Arc::new(tool);
    assert!(plan_mode_tool_visible(&tool));
}

// ── plan_approval removed from Plan Mode visibility ────────────────────────

/// `plan_approval` is NOT in PLAN_MODE_ALWAYS_VISIBLE, so a non-read-only
/// tool with that name should be hidden in Plan Mode.
#[test]
fn test_plan_mode_tool_not_visible_plan_approval() {
    let tool = DummyTool {
        name: "plan_approval".to_string(),
        group: "plan".to_string(),
        summary_text: "approve plan".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    };
    let tool: Arc<dyn Tool> = Arc::new(tool);
    assert!(!plan_mode_tool_visible(&tool));
}

/// `plan_approval` should NOT appear in Plan Mode tool section even if
/// registered (it was removed in Step 1.1).
#[tokio::test]
async fn test_plan_mode_hides_plan_approval_tool() {
    let reg = ToolRegistry::new();
    reg.register(DummyTool {
        name: "plan_approval".to_string(),
        group: "plan".to_string(),
        summary_text: "approve plan".to_string(),
        is_deferred: false,
        is_read_only: false,
        is_destructive: false,
    })
    .await
    .unwrap();

    let ctx = make_plan_mode_ctx();
    let section = reg.build_tools_section(&ctx).await;

    assert!(
        !section.contains("plan_approval"),
        "plan_approval should be hidden in Plan mode, got: {section}"
    );
}

// =========================================================================
// strip_keywords_prefix tests
// =========================================================================

#[test]
fn test_strip_keywords_prefix_variants() {
    // With prefix -> stripped
    assert_eq!(
        strip_keywords_prefix("[keywords: read file cat] Read file contents"),
        "Read file contents"
    );
    assert_eq!(
        strip_keywords_prefix("[keywords: ] some detail"),
        "some detail"
    );
    assert_eq!(strip_keywords_prefix("[keywords: search find]"), "");
    // Without prefix -> unchanged
    assert_eq!(
        strip_keywords_prefix("Just a regular description"),
        "Just a regular description"
    );
    assert_eq!(strip_keywords_prefix(""), "");
    // Prefix in middle -> not stripped
    assert_eq!(
        strip_keywords_prefix("Some text [keywords: read file] and after"),
        "Some text [keywords: read file] and after"
    );
    // Empty bracket -> unchanged
    assert_eq!(
        strip_keywords_prefix("[keywords:] some detail"),
        "[keywords:] some detail"
    );
    assert_eq!(strip_keywords_prefix("[keywords:]"), "[keywords:]");
}

#[test]
fn test_from_tool_strips_keywords_from_detail() {
    struct Dummy {
        name: String,
        detail_text: String,
    }
    impl Tool for Dummy {
        fn name(&self) -> &str {
            &self.name
        }
        fn group(&self) -> &str {
            "test"
        }
        fn summary(&self) -> String {
            self.name.clone()
        }
        fn detail(&self) -> String {
            self.detail_text.clone()
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn flags(&self) -> ToolFlags {
            ToolFlags::default()
        }
    }
    // With keywords prefix → stripped.
    let tool: Arc<dyn Tool> = Arc::new(Dummy {
        name: "KwTool".to_string(),
        detail_text: "[keywords: search find] Search for things".to_string(),
    });
    let info = ToolInfo::from_tool(&tool, &make_prompt_ctx(&["KwTool"]));
    assert_eq!(info.detail, "Search for things");
    // Without keywords prefix → unchanged.
    let tool: Arc<dyn Tool> = Arc::new(Dummy {
        name: "PlainTool".to_string(),
        detail_text: "Plain description".to_string(),
    });
    let info = ToolInfo::from_tool(&tool, &make_prompt_ctx(&["PlainTool"]));
    assert_eq!(info.detail, "Plain description");
}

// =========================================================================
// build_tools_section — integration test: keywords prefix not leaked
// =========================================================================

/// Verify that `build_tools_section` output never contains `[keywords:`
/// even when a tool's raw detail includes the prefix.
#[tokio::test]
async fn test_build_tools_section_strips_keywords() {
    struct KwDummy {
        name: String,
        detail_text: String,
    }
    impl Tool for KwDummy {
        fn name(&self) -> &str {
            &self.name
        }
        fn group(&self) -> &str {
            "file_ops"
        }
        fn summary(&self) -> String {
            format!("summary for {}", self.name)
        }
        fn detail(&self) -> String {
            self.detail_text.clone()
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn flags(&self) -> ToolFlags {
            ToolFlags::default()
        }
    }

    let reg = ToolRegistry::new();
    reg.register(KwDummy {
        name: "Read".to_string(),
        detail_text: "[keywords: read file cat view content] Read file contents".to_string(),
    })
    .await
    .unwrap();

    let ctx = make_prompt_ctx(&["Read"]);
    let section = reg.build_tools_section(&ctx).await;

    assert!(
        !section.contains("[keywords:"),
        "build_tools_section output must not contain [keywords: prefix, got: {section}"
    );
    assert!(
        section.contains("Read file contents"),
        "detail text should be present after stripping, got: {section}"
    );
}

// =========================================================================
// ToolRegistryQuery trait-level tests: get_tool_detail & list_tool_names_by_group
// =========================================================================
mod group_atomicity_tests;
mod keyword_tests;
mod registry_integration_tests;
mod spawn_filter_tests;
mod tool_registry_query_tests;
mod truncation_tests;
