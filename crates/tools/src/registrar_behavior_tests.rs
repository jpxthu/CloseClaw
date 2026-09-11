//! Behavior tests for tool registration patterns.
//!
//! Verifies:
//! - ModeExecutionTriggerTool is NOT registered through the standard
//!   registrar chain (`register_all`), matching the design doc requirement
//!   that Mode tools are registered independently via `register_before_freeze`.
//! - SkillsToolsRegistrar only registers "skills" group tools, not
//!   "skill_creator".

use super::*;
use crate::builtin::skill_tool::SkillTool;
use crate::test_adapters::{ApprovalFlowAdapter, PermissionEngineAdapter};
use crate::{CoreToolsRegistrar, SkillsToolsRegistrar, ToolRegistrar};
use closeclaw_agent::registry::AgentRegistry;
use closeclaw_common::ToolRegistryQuery;
use closeclaw_config::ConfigManager;
use closeclaw_gateway::SpawnController;
use closeclaw_gateway::{GatewayConfig, SessionManager};
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::RuleSet;
use closeclaw_permission::rules::RuleSetBuilder;
use closeclaw_session::persistence::ReasoningLevel;
use closeclaw_session::tools::SessionToolsRegistrar;
use closeclaw_skills::DiskSkillRegistry;
use closeclaw_tasks::BackgroundTaskManager;
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers (mirrored from build_tools_section::tests to keep imports clean)
// ---------------------------------------------------------------------------

fn test_permission_engine() -> Arc<tokio::sync::RwLock<PermissionEngine>> {
    Arc::new(tokio::sync::RwLock::new(
        PermissionEngine::new_with_default_data_root(RuleSetBuilder::new().build().unwrap()),
    ))
}

fn test_approval_flow(
    session_manager: &Arc<SessionManager>,
) -> Arc<tokio::sync::Mutex<ApprovalFlow>> {
    Arc::new(tokio::sync::Mutex::new(ApprovalFlow::new(
        Arc::clone(session_manager) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        RuleSet::default(),
    )))
}

fn test_spawn_deps() -> (
    Arc<SpawnController>,
    Arc<SessionManager>,
    Arc<ConfigManager>,
    Arc<AgentRegistry>,
) {
    let tmp = TempDir::new().expect("tempdir for test");
    let cfg_mgr = Arc::new(
        ConfigManager::new(tmp.path().to_path_buf())
            .expect("failed to create ConfigManager for test"),
    );
    let cfg = GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 65536,
        ..Default::default()
    };
    let session_manager = Arc::new(SessionManager::new(
        &cfg,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let agent_registry = Arc::new(AgentRegistry::new());
    let permission_engine = test_permission_engine();
    let permission_checker: Arc<dyn closeclaw_common::PermissionChecker> = Arc::new(
        closeclaw_gateway::session_manager::spawn_adapter::GatewayPermissionChecker::new(
            Arc::clone(&session_manager),
            Arc::clone(&cfg_mgr),
            permission_engine,
        ),
    );
    let spawn_controller = Arc::new(SpawnController::new(
        Arc::clone(&cfg_mgr),
        Arc::clone(&session_manager) as Arc<dyn closeclaw_session::spawn::controller::SpawnContext>,
        permission_checker,
    ));
    (spawn_controller, session_manager, cfg_mgr, agent_registry)
}

fn make_standard_registrars(
    tool_registry: Arc<dyn closeclaw_common::tool_registry::ToolRegistryQuery>,
) -> Vec<Box<dyn ToolRegistrar>> {
    let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
    let permission_engine = test_permission_engine();
    let (spawn_controller, session_manager, config_manager, agent_registry) = test_spawn_deps();
    let task_manager = Arc::new(BackgroundTaskManager::new());
    let approval_flow = test_approval_flow(&session_manager);

    vec![
        Box::new(CoreToolsRegistrar::new(
            permission_engine.clone(),
            task_manager as Arc<dyn closeclaw_tasks::TaskManager>,
            session_manager.clone(),
            config_manager,
            approval_flow.clone(),
            tool_registry,
        )),
        Box::new(SessionToolsRegistrar::new(
            spawn_controller.clone() as Arc<dyn crate::SpawnValidator>,
            session_manager.clone() as Arc<dyn closeclaw_session::tools::SessionManagerOps>,
            agent_registry.clone() as Arc<dyn closeclaw_agent::AgentConfigLookup>,
            Arc::new(PermissionEngineAdapter(permission_engine)),
            Arc::new(tokio::sync::Mutex::new(ApprovalFlowAdapter(
                approval_flow.clone(),
            ))),
        )),
        Box::new(SkillsToolsRegistrar::new(vec![Arc::new(SkillTool::new(
            disk_registry,
            Arc::new(closeclaw_skills::BuiltinSkillRegistry::new()),
        ))])),
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// ModeExecutionTriggerTool is NOT registered through the standard
/// registrar chain (`register_all`). This verifies the design doc
/// requirement that Mode tools bypass the four standard registrars
/// and are registered independently via `register_before_freeze`.
#[tokio::test]
async fn test_register_all_does_not_register_mode_execution_trigger() {
    let registry = Arc::new(ToolRegistry::new());
    let registrars = make_standard_registrars(
        Arc::clone(&registry) as Arc<dyn closeclaw_common::tool_registry::ToolRegistryQuery>
    );
    registry.register_all(registrars).await.unwrap();

    // ModeExecutionTriggerTool should NOT be in the registry after register_all.
    let names: Vec<String> = registry.list_tool_names().await;
    assert!(
        !names.contains(&"ModeExecutionTrigger".to_string()),
        "ModeExecutionTriggerTool must NOT be registered through register_all, got: {:?}",
        names
    );
    // Also verify via get_detail.
    assert!(
        registry.get_detail("ModeExecutionTrigger").await.is_err(),
        "ModeExecutionTriggerTool should not be queryable after register_all"
    );
}

/// ModeExecutionTriggerTool CAN be registered via `register_before_freeze`
/// after the standard chain has run. This mirrors the production pattern
/// in `daemon/registries.rs`.
#[tokio::test]
async fn test_mode_execution_trigger_registerable_via_before_freeze() {
    let registry = Arc::new(ToolRegistry::new());
    let registrars = make_standard_registrars(
        Arc::clone(&registry) as Arc<dyn closeclaw_common::tool_registry::ToolRegistryQuery>
    );
    registry.register_all(registrars).await.unwrap();

    // ModeExecutionTriggerTool should not be present yet.
    assert!(registry.get_detail("ModeExecutionTrigger").await.is_err());

    // Register via register_before_freeze (the production pattern).
    let mode_tool: Arc<dyn closeclaw_common::Tool> =
        Arc::new(crate::builtin::ModeExecutionTriggerTool::new(
            Arc::new(SessionManager::new(
                &GatewayConfig::default(),
                None,
                None,
                ReasoningLevel::default(),
            )),
            Arc::new(crate::builtin::PlanExecConfirmFlow::new(
                Arc::new(SessionManager::new(
                    &GatewayConfig::default(),
                    None,
                    None,
                    ReasoningLevel::default(),
                )) as Arc<dyn closeclaw_common::SessionLookup>,
                Arc::new(|_| {}),
                tokio::runtime::Handle::current(),
            )),
        ));
    registry
        .register_before_freeze(mode_tool, "SystemLevel")
        .await
        .unwrap();

    // Now it should be present.
    let detail = registry.get_detail("ModeExecutionTrigger").await;
    assert!(
        detail.is_ok(),
        "ModeExecutionTriggerTool should be queryable after register_before_freeze"
    );
}

/// SkillsToolsRegistrar only registers tools in the "skills" group.
/// Specifically, it should NOT register SkillCreatorTool (which belongs
/// to the "skill_creator" group). This verifies the fix from Step 1.1.
#[tokio::test]
async fn test_skills_registrar_only_registers_skills_group() {
    let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
    let skill_tool = SkillTool::new(
        disk_registry,
        Arc::new(closeclaw_skills::BuiltinSkillRegistry::new()),
    );
    let skill_group = skill_tool.group().to_string();

    let registrar = SkillsToolsRegistrar::new(vec![Arc::new(skill_tool)]);
    let registry = ToolRegistry::new();

    // Register via the registrar.
    registrar
        .register(&registry as &dyn closeclaw_common::tool_registry::ToolRegistry)
        .await
        .unwrap();

    let names: Vec<String> = registry.list_tool_names().await;

    // SkillTool should be registered.
    assert!(
        names.contains(&"SkillTool".to_string()),
        "SkillTool should be registered, got: {:?}",
        names
    );

    // Verify the registered tool is in the "skills" group, not "skill_creator".
    assert_eq!(
        skill_group, "skills",
        "SkillTool should belong to the 'skills' group"
    );

    // SkillCreatorTool (group "skill_creator") must NOT be registered
    // through SkillsToolsRegistrar — only SkillTool should be present
    // when SkillsToolsRegistrar is constructed with a single SkillTool.
    assert!(
        !names.contains(&"SkillCreator".to_string()),
        "SkillCreatorTool must NOT be registered by SkillsToolsRegistrar with only SkillTool, got: {:?}",
        names
    );
}
