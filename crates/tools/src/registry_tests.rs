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
    // Eager read-only: bold name + "(read-only)" + detail
    assert!(
        section.contains("**Viewer** (read-only): detail for Viewer"),
        "expected eager read-only mark, got: {section}"
    );
    // Eager destructive: bold name + "(destructive)" + detail
    assert!(
        section.contains("**Deleter** (destructive): detail for Deleter"),
        "expected eager destructive mark, got: {section}"
    );
    // Eager no mark: no suffix after bold name
    assert!(
        section.contains("**Lister**: detail for Lister"),
        "expected eager no mark, got: {section}"
    );
    // Deferred read-only: name + "(read-only)"
    assert!(
        section.contains("  - DReader (read-only)"),
        "expected deferred read-only mark, got: {section}"
    );
    // Deferred destructive: name + "(destructive)"
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

// ---- AgentToolsConfigQuery path tests ----
// Tests for ToolRegistry's ability to query agent tools configuration.
// These tests require a mock AgentToolsConfigQuery implementation.
// For now, marked #[ignore] until moved to integration tests.

use closeclaw_common::bootstrap::BootstrapMode;
use closeclaw_config::agents::{ConfigSource, MemoryConfig, ResolvedAgentConfig};

#[allow(dead_code)]
fn make_agent_config(
    id: &str,
    tools: Vec<String>,
    disallowed_tools: Vec<String>,
) -> ResolvedAgentConfig {
    ResolvedAgentConfig {
        id: id.to_string(),
        name: id.to_string(),
        parent_id: None,
        model: None,
        workspace: None,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills: vec![],
        tools,
        disallowed_tools,
        subagents: Default::default(),
        memory: MemoryConfig::default(),
        source: ConfigSource::User,
    }
}

#[tokio::test]
#[ignore = "requires mock AgentToolsConfigQuery — move to integration tests"]
async fn test_query_agent_tools_config_not_set() {
    let reg = ToolRegistry::new();
    // No query set — should return (None, None)
    let (tools, disallowed) = reg.query_agent_tools_config("any-agent").await;
    assert_eq!(tools, None);
    assert_eq!(disallowed, None);
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
// PlanApprovalTool registration and query tests
// =========================================================================

use crate::builtin::plan_approval::PlanApprovalTool;
use closeclaw_common::ToolRegistryQuery;

#[tokio::test]
async fn test_plan_approval_tool_register_and_query() {
    let reg = ToolRegistry::new();
    let tool = PlanApprovalTool::new();
    let name = tool.name().to_string();
    assert_eq!(name, "plan_approval");

    reg.register(tool).await.unwrap();

    assert!(reg.has_tool(&name).await);
    assert!(!reg.has_tool("nonexistent").await);

    let detail = reg.get_detail(&name).await.unwrap();
    assert!(detail.contains("Plan Mode"));
    assert!(detail.contains("Auto Mode"));
}

#[tokio::test]
async fn test_plan_approval_tool_in_list_by_group() {
    let reg = ToolRegistry::new();
    reg.register(PlanApprovalTool::new()).await.unwrap();

    let plan_tools = reg.list_by_group("plan").await;
    assert!(
        plan_tools.contains(&"plan_approval".to_string()),
        "plan_approval should be in plan group, got: {:?}",
        plan_tools
    );
}

#[tokio::test]
async fn test_plan_approval_tool_descriptor_fields() {
    let reg = ToolRegistry::new();
    reg.register(PlanApprovalTool::new()).await.unwrap();

    let ctx = make_ctx();
    let descriptors = reg.list_descriptors(&ctx).await;
    let desc = descriptors
        .iter()
        .find(|d| d.name == "plan_approval")
        .expect("plan_approval descriptor should exist");

    assert_eq!(desc.group, "plan");
    assert!(!desc.is_deferred);
    assert!(desc.summary.contains("plan"));
}
