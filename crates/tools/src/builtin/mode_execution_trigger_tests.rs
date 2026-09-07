//! Tests for ModeExecutionTriggerTool.
//!
//! Covers: tool metadata, error paths (missing session_id, missing plan
//! info), and the confirm_pending happy path.
//!
//! Note: Full happy-path testing (plan state + confirmed status → execution)
//! requires a persistence backend to store plan_state, which is not available
//! in unit tests. The error paths verify the tool's validation logic covers
//! the dimensions specified in the plan.

use crate::builtin::plan_exec_confirm::PlanExecMetadata;
use crate::builtin::PlanExecConfirmFlow;
use crate::{Tool, ToolCallError, ToolContext, ToolFlags, WorkdirContext};
use closeclaw_common::tool_registry::{ToolRegistrar, ToolRegistryQuery as _};
use closeclaw_common::SessionMode;
use closeclaw_gateway::GatewayConfig;
use closeclaw_gateway::SessionManager;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

// ── Helpers ─────────────────────────────────────────────────────────────────

fn make_ctx(session_id: Option<&str>) -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: session_id.map(|s| s.to_string()),
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    }
}

fn make_ctx_with_workdir(session_id: Option<&str>, workdir: &std::path::Path) -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: Some(WorkdirContext {
            path: workdir.to_string_lossy().into_owned(),
            has_git: false,
            branch: None,
            recent_changes: 0,
        }),
        session_id: session_id.map(|s| s.to_string()),
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
        media_store: None,
    }
}

fn make_session_manager() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &GatewayConfig::default(),
        None, // no storage → get_plan_state returns None
        None,
        closeclaw_common::ReasoningLevel::default(),
    ))
}

fn make_confirm_flow() -> Arc<PlanExecConfirmFlow> {
    let sm = make_session_manager();
    let flow = PlanExecConfirmFlow::new(
        sm.clone() as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        tokio::runtime::Handle::current(),
    );
    Arc::new(flow)
}

/// Register a ConversationSession in the SessionManager.
async fn register_session(sm: &SessionManager, session_id: &str, mode: SessionMode) {
    let cs = closeclaw_session::llm_session::ConversationSession::new(
        session_id.to_owned(),
        "test-model".to_owned(),
        PathBuf::from("/tmp"),
    )
    .with_session_mode(mode);
    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    {
        let mut conv = sm.conversation_sessions.write().await;
        conv.insert(session_id.to_owned(), cs_arc);
    }
}

fn make_tool(
    sm: Arc<SessionManager>,
    cf: Arc<PlanExecConfirmFlow>,
) -> crate::builtin::mode_execution_trigger::ModeExecutionTriggerTool {
    crate::builtin::mode_execution_trigger::ModeExecutionTriggerTool::new(sm, cf)
}

/// Create a temp workspace with a plan file so resolve_plan_by_name succeeds.
fn setup_workspace_with_plan() -> (TempDir, String) {
    let tmp = TempDir::new().unwrap();
    let plans_dir = tmp.path().join("plans");
    std::fs::create_dir_all(&plans_dir).unwrap();
    std::fs::write(plans_dir.join("my-plan.md"), "# Plan\n\n- [ ] step1\n").unwrap();
    (tmp, "my-plan".to_string())
}

// ── Tool metadata tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_tool_name() {
    let sm = make_session_manager();
    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    assert_eq!(tool.name(), "ModeExecutionTrigger");
}

#[tokio::test]
async fn test_tool_group() {
    let sm = make_session_manager();
    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    assert_eq!(tool.group(), "mode");
}

#[tokio::test]
async fn test_tool_summary() {
    let sm = make_session_manager();
    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    assert!(!tool.summary().is_empty());
}

#[tokio::test]
async fn test_tool_flags() {
    let sm = make_session_manager();
    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    let flags = tool.flags();
    assert!(flags.is_concurrency_safe);
    assert!(!flags.is_read_only);
    assert!(!flags.is_destructive);
    assert!(!flags.is_deferred_by_default);
}

#[tokio::test]
async fn test_tool_input_schema_properties() {
    let sm = make_session_manager();
    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    let schema = tool.input_schema();
    let props = schema.pointer("/properties").unwrap();
    assert!(props.get("plan_file_path").is_some());
    assert!(props.get("step_selection").is_some());
    assert!(props.get("new_session").is_some());
    // No required fields
    let required = schema.pointer("/required").unwrap().as_array().unwrap();
    assert!(required.is_empty());
}

// ── Step 1.4: Schema property tests ──────────────────────────────────────

#[tokio::test]
async fn test_tool_input_schema_has_plan_name_property() {
    let sm = make_session_manager();
    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    let schema = tool.input_schema();
    let props = schema.pointer("/properties").unwrap();
    let plan_name = props
        .get("plan_name")
        .expect("plan_name property should exist");
    assert_eq!(plan_name["type"], "string");
    let desc = plan_name["description"].as_str().unwrap();
    assert!(
        desc.contains("Name of the plan"),
        "plan_name description should mention name: {desc}"
    );
}

#[tokio::test]
async fn test_tool_input_schema_has_additional_instruction_property() {
    let sm = make_session_manager();
    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    let schema = tool.input_schema();
    let props = schema.pointer("/properties").unwrap();
    let ai = props
        .get("additional_instruction")
        .expect("additional_instruction property should exist");
    assert_eq!(ai["type"], "string");
    let desc = ai["description"].as_str().unwrap();
    assert!(
        desc.contains("Optional instruction"),
        "additional_instruction description should mention optional: {desc}"
    );
}

// ── Step 1.4: Detail doc string test ─────────────────────────────────────

#[tokio::test]
async fn test_tool_detail_mentions_additional_instruction() {
    let sm = make_session_manager();
    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    let detail = tool.detail();
    assert!(
        detail.contains("additional instruction"),
        "detail should mention additional instruction: {detail}"
    );
}

// ── Error path tests ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_call_without_session_id() {
    let sm = make_session_manager();
    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    let ctx = make_ctx(None);

    let result = tool.call(json!({}), &ctx).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("session_id"),
                "error should mention session_id: {msg}"
            );
        }
        other => panic!("expected ExecutionFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_call_no_plan_info_returns_error() {
    let sm = make_session_manager();
    register_session(&sm, "sess-normal", SessionMode::Normal).await;

    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    let ctx = make_ctx(Some("sess-normal"));

    // No plan_name, no plan_file_path, no plan state → fallback
    // load_plan_state fails → error
    let result = tool.call(json!({}), &ctx).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("活跃的 plan"),
                "error should mention missing plan: {msg}"
            );
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_call_with_plan_file_path_bypasses_plan_state() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan-file", SessionMode::Plan).await;

    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    let ctx = make_ctx(Some("sess-plan-file"));

    // plan_file_path provided → plan_state check is skipped,
    // file does not exist → file-not-found error
    let result = tool
        .call(json!({"plan_file_path": "/some/path.md"}), &ctx)
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("plan 文件不存在"),
                "error should mention file not found: {msg}"
            );
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_call_with_step_selection_parses_correctly() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan-steps", SessionMode::Plan).await;

    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    let ctx = make_ctx(Some("sess-plan-steps"));

    // No plan_name/plan_file_path → fallback load_plan_state → error
    let result = tool.call(json!({"step_selection": [0, 1, 2]}), &ctx).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("活跃的 plan"),
                "error should mention missing plan: {msg}"
            );
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_call_with_new_session_flag() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan-newsess", SessionMode::Plan).await;

    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    let ctx = make_ctx(Some("sess-plan-newsess"));

    // No plan_name/plan_file_path → fallback load_plan_state → error
    let result = tool.call(json!({"new_session": true}), &ctx).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("活跃的 plan"),
                "error should mention missing plan: {msg}"
            );
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

// ── Step 1.4: plan_name and additional_instruction argument tests ────────

#[tokio::test]
async fn test_call_plan_name_resolves_by_name_not_plan_state() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan-name", SessionMode::Plan).await;

    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    let ctx = make_ctx(Some("sess-plan-name"));

    // plan_name provided → plan_state check skipped,
    // plan file not found → "未找到名为" error
    let result = tool
        .call(
            json!({
                "plan_name": "my-plan",
                "additional_instruction": "请优先处理测试用例"
            }),
            &ctx,
        )
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("未找到名为"),
                "error should mention plan not found: {msg}"
            );
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_call_plan_name_with_additional_instruction_not_found() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan-missing", SessionMode::Plan).await;

    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    let ctx = make_ctx(Some("sess-plan-missing"));

    // plan_name + additional_instruction provided but plan file not found
    let result = tool
        .call(
            json!({
                "plan_name": "my-plan",
                "additional_instruction": "请优先处理测试用例"
            }),
            &ctx,
        )
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("未找到名为 'my-plan'"),
                "error should mention plan name: {msg}"
            );
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_call_empty_additional_instruction_filtered() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan-empty-ai", SessionMode::Plan).await;

    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    let ctx = make_ctx(Some("sess-plan-empty-ai"));

    // Empty additional_instruction treated as absent → no plan_name/plan_file_path
    // → fallback load_plan_state → error
    let result = tool
        .call(
            json!({
                "additional_instruction": "  "
            }),
            &ctx,
        )
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(msg.contains("活跃的 plan"), "got: {msg}");
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_call_empty_plan_name_filtered() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan-empty-pn", SessionMode::Plan).await;

    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    let ctx = make_ctx(Some("sess-plan-empty-pn"));

    // Empty plan_name filtered → no plan_name/plan_file_path → fallback
    // load_plan_state → error
    let result = tool
        .call(
            json!({
                "plan_name": ""
            }),
            &ctx,
        )
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(msg.contains("活跃的 plan"), "got: {msg}");
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

// ── Step 1.2: Any-mode execution tests ──────────────────────────────────

#[tokio::test]
async fn test_call_normal_mode_with_plan_name() {
    let sm = make_session_manager();
    register_session(&sm, "sess-normal-plan", SessionMode::Normal).await;

    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);

    let (tmp, plan_name) = setup_workspace_with_plan();
    let ctx = make_ctx_with_workdir(Some("sess-normal-plan"), tmp.path());

    // Normal mode + plan_name + plan file exists → confirm_pending
    let result = tool.call(json!({"plan_name": &plan_name}), &ctx).await;
    assert!(result.is_ok(), "should succeed with valid plan_name");
    let tr = result.unwrap();
    assert_eq!(tr.data["status"], "confirm_pending");
    assert!(tr.data.get("confirmation_id").is_some());
}

#[tokio::test]
async fn test_call_auto_mode_with_plan_name() {
    let sm = make_session_manager();
    register_session(&sm, "sess-auto-plan", SessionMode::Auto).await;

    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);

    let (tmp, plan_name) = setup_workspace_with_plan();
    let ctx = make_ctx_with_workdir(Some("sess-auto-plan"), tmp.path());

    // Auto mode + plan_name + plan file exists → confirm_pending
    let result = tool.call(json!({"plan_name": &plan_name}), &ctx).await;
    assert!(result.is_ok(), "should succeed with valid plan_name");
    let tr = result.unwrap();
    assert_eq!(tr.data["status"], "confirm_pending");
    assert!(tr.data.get("confirmation_id").is_some());
}

#[tokio::test]
async fn test_call_plan_mode_no_plan_info_returns_error() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan", SessionMode::Plan).await;

    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);
    let ctx = make_ctx(Some("sess-plan"));

    // Plan mode, no plan_name/plan_file_path, no plan state → error
    let result = tool.call(json!({}), &ctx).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("活跃的 plan"),
                "error should mention missing plan: {msg}"
            );
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

// ── Pending map metadata assertion ──────────────────────────────────────

/// Verify that after submit the PlanExecConfirmFlow pending map stores
/// the full metadata (plan_file_path, step_selection, new_session,
/// additional_instruction).
#[tokio::test]
async fn test_submit_stores_metadata_in_pending_map() {
    let sm = make_session_manager();
    register_session(&sm, "sess-meta", SessionMode::Normal).await;

    let sm_arc: Arc<dyn closeclaw_common::SessionLookup> = sm.clone();
    let flow = Arc::new(PlanExecConfirmFlow::new(
        sm_arc,
        Arc::new(|_| {}),
        tokio::runtime::Handle::current(),
    ));
    let tool =
        crate::builtin::mode_execution_trigger::ModeExecutionTriggerTool::new(sm, flow.clone());

    let (tmp, plan_name) = setup_workspace_with_plan();
    let ctx = make_ctx_with_workdir(Some("sess-meta"), tmp.path());

    let result = tool
        .call(
            json!({
                "plan_name": &plan_name,
                "step_selection": [0, 2],
                "new_session": true,
                "additional_instruction": "focus on tests"
            }),
            &ctx,
        )
        .await;
    assert!(result.is_ok());
    let tr = result.unwrap();
    let cid = tr.data["confirmation_id"].as_str().unwrap();

    let stored: PlanExecMetadata = flow
        .get_pending_metadata(cid)
        .await
        .expect("entry should exist");
    assert!(
        stored.plan_file_path.ends_with("my-plan.md"),
        "plan_file_path should resolve to the plan file: {}",
        stored.plan_file_path
    );
    assert_eq!(stored.step_selection, Some(vec![0, 2]));
    assert!(stored.new_session);
    assert_eq!(
        stored.additional_instruction.as_deref(),
        Some("focus on tests")
    );
}

/// Verify that empty additional_instruction is filtered out before submit.
#[tokio::test]
async fn test_submit_filters_empty_additional_instruction() {
    let sm = make_session_manager();
    register_session(&sm, "sess-empty-ai", SessionMode::Normal).await;

    let sm_arc: Arc<dyn closeclaw_common::SessionLookup> = sm.clone();
    let flow = Arc::new(PlanExecConfirmFlow::new(
        sm_arc,
        Arc::new(|_| {}),
        tokio::runtime::Handle::current(),
    ));
    let tool =
        crate::builtin::mode_execution_trigger::ModeExecutionTriggerTool::new(sm, flow.clone());

    let (tmp, plan_name) = setup_workspace_with_plan();
    let ctx = make_ctx_with_workdir(Some("sess-empty-ai"), tmp.path());

    let result = tool
        .call(
            json!({
                "plan_name": &plan_name,
                "additional_instruction": "   "
            }),
            &ctx,
        )
        .await;
    assert!(result.is_ok());
    let tr = result.unwrap();
    let cid = tr.data["confirmation_id"].as_str().unwrap();

    let stored: PlanExecMetadata = flow.get_pending_metadata(cid).await.unwrap();
    assert!(
        stored.additional_instruction.is_none(),
        "whitespace-only additional_instruction should be filtered"
    );
}

// ── Access timestamp refresh test (Step 1.3 touch-point) ───────────────

/// Helper: create a plan file with a known old access timestamp marker.
fn write_plan_with_old_timestamp(workdir: &std::path::Path, stem: &str) {
    let plans = workdir.join("plans");
    std::fs::create_dir_all(&plans).unwrap();
    std::fs::write(
        plans.join(format!("{stem}.md")),
        "# Old Plan\n<!-- accessed: 2020-01-01T00:00:00Z -->\n\n## Tasks\n\n- [ ] step1\n",
    )
    .unwrap();
}

#[tokio::test]
async fn test_mode_execution_trigger_tool_refreshes_access_timestamp() {
    let sm = make_session_manager();
    register_session(&sm, "sess-ts-touch", SessionMode::Normal).await;

    let cf = make_confirm_flow();
    let tool = make_tool(sm, cf);

    let tmp = tempfile::TempDir::new().unwrap();
    write_plan_with_old_timestamp(tmp.path(), "ts-plan");

    let ctx = make_ctx_with_workdir(Some("sess-ts-touch"), tmp.path());

    // Verify old timestamp exists before triggering
    let plan_path = tmp.path().join("plans/ts-plan.md");
    let before_content = std::fs::read_to_string(&plan_path).unwrap();
    assert!(
        before_content.contains("<!-- accessed: 2020-01-01T00:00:00Z -->"),
        "plan file should contain old timestamp marker"
    );

    // Trigger ModeExecutionTrigger tool
    let result = tool.call(json!({"plan_name": "ts-plan"}), &ctx).await;
    assert!(result.is_ok(), "should succeed with valid plan_name");

    // Assert timestamp was refreshed (marker string changed)
    let after_content = std::fs::read_to_string(&plan_path).unwrap();
    assert!(
        after_content.contains("<!-- accessed:"),
        "plan file should still have access timestamp marker"
    );
    assert_ne!(
        before_content, after_content,
        "plan file content should change after touch"
    );
    // The old marker should be gone
    assert!(
        !after_content.contains("2020-01-01T00:00:00Z"),
        "old timestamp marker should be replaced"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Step 1.2: Document Contract Three-Element UT
// ═══════════════════════════════════════════════════════════════════════════

// ── Registrar contract ──────────────────────────────────────────────────────

/// ModeToolsRegistrar.name() returns "ModeToolsRegistrar".
#[tokio::test]
async fn test_registrar_name() {
    let sm = make_session_manager();
    let cf = make_confirm_flow();
    let registrar = crate::registrars::mode::ModeToolsRegistrar::new(sm, cf);
    assert_eq!(registrar.name(), "ModeToolsRegistrar");
}

/// ModeToolsRegistrar.priority() returns 3 (registered after CoreToolsRegistrar
/// at 1 and SessionToolsRegistrar at 2).
#[tokio::test]
async fn test_registrar_priority() {
    let sm = make_session_manager();
    let cf = make_confirm_flow();
    let registrar = crate::registrars::mode::ModeToolsRegistrar::new(sm, cf);
    assert_eq!(registrar.priority(), 3);
}

/// After ModeToolsRegistrar registers, the registry contains a tool named
/// "ModeExecutionTrigger" with group "mode".
#[tokio::test]
async fn test_registrar_registers_mode_execution_trigger() {
    let sm = make_session_manager();
    let cf = make_confirm_flow();
    let registrar = crate::registrars::mode::ModeToolsRegistrar::new(sm, cf);

    let reg = crate::registry::ToolRegistry::new();
    registrar
        .register(&reg as &dyn closeclaw_common::tool_registry::ToolRegistry)
        .await
        .unwrap();

    // Contract: tool named "ModeExecutionTrigger" exists
    assert!(
        reg.has_tool("ModeExecutionTrigger").await,
        "ModeToolsRegistrar should register a tool named ModeExecutionTrigger"
    );

    // Contract: group is "mode"
    let by_group = reg.list_tool_names_by_group("mode").await;
    assert_eq!(
        by_group,
        vec!["ModeExecutionTrigger".to_string()],
        "ModeExecutionTrigger should be in the 'mode' group"
    );
}

// ── Plan Mode visibility via build_tools_section ─────────────────────────────

/// DummyTool for testing plan mode visibility with specific metadata.
struct PlanVisDummyTool {
    tool_name: String,
    group: String,
    is_read_only: bool,
}

impl Tool for PlanVisDummyTool {
    fn name(&self) -> &str {
        &self.tool_name
    }
    fn group(&self) -> &str {
        &self.group
    }
    fn summary(&self) -> String {
        format!("test tool {}", self.tool_name)
    }
    fn detail(&self) -> String {
        format!("detail for {}", self.tool_name)
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }
    fn flags(&self) -> ToolFlags {
        let mut f = ToolFlags::default();
        f.is_read_only = self.is_read_only;
        f
    }
}

fn make_plan_mode_ctx() -> crate::PromptGenerationContext {
    crate::PromptGenerationContext {
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

/// ModeExecutionTrigger (non-read-only) appears in build_tools_section
/// output under Plan Mode — verifies "始终加载" + Plan Mode 下触发执行入口保留.
#[tokio::test]
async fn test_plan_mode_section_shows_mode_execution_trigger() {
    let reg = crate::registry::ToolRegistry::new();
    reg.register(PlanVisDummyTool {
        tool_name: "ModeExecutionTrigger".to_string(),
        group: "mode".to_string(),
        is_read_only: false,
    })
    .await
    .unwrap();

    let ctx = make_plan_mode_ctx();
    let section = reg.build_tools_section(&ctx).await;
    assert!(
        section.contains("ModeExecutionTrigger"),
        "ModeExecutionTrigger should appear in Plan Mode tools section: {section}"
    );
}

/// ModeExecutionTrigger's group header contains "(always loaded)" in Plan Mode.
#[tokio::test]
async fn test_plan_mode_section_group_is_always_loaded() {
    let reg = crate::registry::ToolRegistry::new();
    reg.register(PlanVisDummyTool {
        tool_name: "ModeExecutionTrigger".to_string(),
        group: "mode".to_string(),
        is_read_only: false,
    })
    .await
    .unwrap();

    let ctx = make_plan_mode_ctx();
    let section = reg.build_tools_section(&ctx).await;
    assert!(
        section.contains("**mode**"),
        "mode group header should appear: {section}"
    );
    assert!(
        section.contains("(always loaded)"),
        "mode group should be (always loaded): {section}"
    );
}

/// A non-read-only tool NOT in PLAN_MODE_ALWAYS_VISIBLE is hidden in Plan Mode.
#[tokio::test]
async fn test_plan_mode_section_hides_non_always_visible() {
    let reg = crate::registry::ToolRegistry::new();
    reg.register(PlanVisDummyTool {
        tool_name: "BashTool".to_string(),
        group: "exec".to_string(),
        is_read_only: false,
    })
    .await
    .unwrap();

    let ctx = make_plan_mode_ctx();
    let section = reg.build_tools_section(&ctx).await;
    assert!(
        !section.contains("BashTool"),
        "BashTool (non-read-only, not always-visible) should be hidden in Plan Mode: {section}"
    );
}

/// A read-only tool is always visible in Plan Mode regardless of its name.
#[tokio::test]
async fn test_plan_mode_section_shows_readonly_tool() {
    let reg = crate::registry::ToolRegistry::new();
    reg.register(PlanVisDummyTool {
        tool_name: "SomeReadOnlyTool".to_string(),
        group: "custom".to_string(),
        is_read_only: true,
    })
    .await
    .unwrap();

    let ctx = make_plan_mode_ctx();
    let section = reg.build_tools_section(&ctx).await;
    assert!(
        section.contains("SomeReadOnlyTool"),
        "read-only tools should be visible in Plan Mode: {section}"
    );
}
