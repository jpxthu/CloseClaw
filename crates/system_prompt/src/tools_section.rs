//! Tools section builder for the system prompt.
//!
//! Owns the `build_tools_section` function and its tests. Extracted from
//! `builder.rs` to keep that file under the 500-line limit.

use crate::sections::Section;
use closeclaw_common::SessionMode;
use closeclaw_tools::{PromptGenerationContext, ToolContext, ToolRegistry};

/// Task writing guidance appended to the tools section when spawn is available.
/// Source: docs/design/agent/agent-spawn.md §父 Agent 的 Task 编写指引
const TASK_WRITING_GUIDANCE: &str = concat!(
    "\n\n## Task Writing Guidance for Spawning Sub-Agents\n\n\n",
    "When spawning a sub-agent, write the task as you would brief a smart colleague ",
    "who just walked into the room \u{2014} explain what you need done and why.\n\n",
    "- Do NOT push judgment calls onto the sub-agent. The parent agent should ",
    "complete understanding and decision-making; the sub-agent executes.\n",
    "- Use fork mode when the sub-agent needs full context of the ongoing conversation. ",
    "Use normal spawn for independent, self-contained tasks."
);

/// Background task guidance appended to the tools section when Bash is available.
/// Source: docs/design/tools/background-tasks.md §提示词引导
const BACKGROUND_TASK_GUIDANCE: &str = concat!(
    "\n\n## Background Task Guidance\n\n\n",
    "- Background commands send an automatic notification when they complete; ",
    "you do not need to poll or check their status manually.\n",
    "- Do not call process-query tools to check whether a background task has finished. ",
    "The push-based notification ensures results appear in the next turn.\n",
    "- Use `run_in_background: true` for commands expected to take over 10 seconds. ",
    "This keeps you unblocked while long-running work completes."
);

/// Build the Tools section content from a registry.
///
/// The registry's `build_tools_section` requires a [`PromptGenerationContext`]
/// (which carries the list of available tool names, the agent id, and the
/// workdir). We acquire that list via a single short-lived lock on
/// `list_descriptors`, release it, and then call the registry's
/// `build_tools_section` with the freshly-built context. This keeps locks
/// non-overlapping.
pub async fn build_tools_section(
    registry: &ToolRegistry,
    ctx: &ToolContext,
    agent_tools: Option<Vec<String>>,
    agent_disallowed_tools: Option<Vec<String>>,
    session_mode: Option<SessionMode>,
    effective_spawn_budget: Option<u32>,
) -> Section {
    // 1. Independent lock: get available tool names, then drop the lock.
    let descriptors = registry.list_descriptors(ctx).await;
    let available_tool_names: Vec<String> = descriptors.into_iter().map(|d| d.name).collect();

    // 2. Resolve agent-level tool filtering.
    //    Priority: explicit parameters > AgentRegistry query.
    let (tools, disallowed_tools) = if agent_tools.is_some() || agent_disallowed_tools.is_some() {
        (agent_tools, agent_disallowed_tools)
    } else {
        // Query AgentRegistry directly (design-doc query path).
        registry.query_agent_tools_config(&ctx.agent_id).await
    };

    // 3. Build the prompt-generation context from the names + the existing
    //    execution context, including agent-level tool filtering.
    let prompt_ctx = PromptGenerationContext {
        agent_id: ctx.agent_id.clone(),
        workdir: ctx.workdir.clone(),
        available_tool_names,
        tools,
        disallowed_tools,
        session_mode,
        effective_spawn_budget,
    };

    // 4. Acquire the registry lock again and render the section.
    let content = registry.build_tools_section(&prompt_ctx).await;

    // 5. If the spawn tool is available, append task writing guidance.
    let content = if prompt_ctx
        .available_tool_names
        .iter()
        .any(|n| n == "sessions_spawn")
    {
        let guidance = TASK_WRITING_GUIDANCE;
        format!("{}\n{}", content, guidance)
    } else {
        content
    };

    // 6. If Bash tool is available, append background task guidance.
    let content = if prompt_ctx.available_tool_names.iter().any(|n| n == "Bash") {
        let guidance = BACKGROUND_TASK_GUIDANCE;
        format!("{}\n{}", content, guidance)
    } else {
        content
    };

    Section::ToolsSection(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_agent::registry::AgentRegistry;
    use closeclaw_common::PlanState;
    use closeclaw_config::ConfigManager;
    use closeclaw_gateway::SpawnController;
    use closeclaw_gateway::{GatewayConfig, SessionManager};
    use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
    use closeclaw_permission::engine::engine_eval::PermissionEngine;
    use closeclaw_permission::engine::engine_types::RuleSet;
    use closeclaw_permission::rules::RuleSetBuilder;
    use closeclaw_session::persistence::ReasoningLevel;
    use closeclaw_skills::DiskSkillRegistry;
    use closeclaw_tasks::BackgroundTaskManager;
    use closeclaw_tools::{
        CoreToolsRegistrar, PlanToolsRegistrar, SessionToolsRegistrar, SkillsToolsRegistrar,
        ToolRegistrar,
    };
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

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

    /// Build a minimal SpawnController + SessionManager pair for tests
    /// that only need to exercise the tool-registration path.
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
        let spawn_controller = Arc::new(SpawnController::new(
            Arc::clone(&agent_registry),
            Arc::clone(&cfg_mgr),
            Arc::clone(&session_manager),
            Arc::new(tokio::sync::RwLock::new(
                PermissionEngine::new_with_default_data_root(
                    RuleSetBuilder::new().build().unwrap(),
                ),
            )),
        ));
        (spawn_controller, session_manager, cfg_mgr, agent_registry)
    }

    fn make_registrars(
        disk_registry: Arc<DiskSkillRegistry>,
        permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
        spawn_controller: Arc<SpawnController>,
        session_manager: Arc<SessionManager>,
        config_manager: Arc<ConfigManager>,
        agent_registry: Arc<AgentRegistry>,
        approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
    ) -> Vec<Box<dyn ToolRegistrar>> {
        let task_manager = Arc::new(BackgroundTaskManager::new());
        vec![
            Box::new(CoreToolsRegistrar::new(
                permission_engine.clone(),
                task_manager as Arc<dyn closeclaw_tasks::TaskManager>,
                session_manager.clone(),
                config_manager,
                approval_flow.clone(),
            )),
            Box::new(SessionToolsRegistrar::new(
                spawn_controller.clone() as Arc<dyn closeclaw_tools::SpawnValidator>,
                session_manager.clone(),
                agent_registry.clone() as Arc<dyn closeclaw_agent::AgentConfigLookup>,
                permission_engine,
                approval_flow.clone(),
            )),
            Box::new(SkillsToolsRegistrar::new(
                disk_registry,
                Arc::new(closeclaw_skills::BuiltinSkillRegistry::new()),
                spawn_controller as Arc<dyn closeclaw_tools::SpawnValidator>,
                session_manager.clone(),
            )),
            Box::new(PlanToolsRegistrar::new(
                Arc::new(Mutex::new(PlanState::new())),
                session_manager.clone(),
                approval_flow.clone(),
            )),
        ]
    }

    #[tokio::test]
    async fn test_build_tools_section_returns_tools_section() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (spawn_controller, session_manager, config_manager, agent_registry) = test_spawn_deps();
        registry
            .register_all(make_registrars(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager.clone(),
                config_manager,
                agent_registry,
                test_approval_flow(&session_manager),
            ))
            .await
            .unwrap();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let section = build_tools_section(&registry, &ctx, None, None, None, None).await;
        match section {
            Section::ToolsSection(_) => {}
            _ => panic!("expected ToolsSection, got {:?}", section),
        }
    }

    #[tokio::test]
    async fn test_build_tools_section_contains_group_headers() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (spawn_controller, session_manager, config_manager, agent_registry) = test_spawn_deps();
        registry
            .register_all(make_registrars(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager.clone(),
                config_manager,
                agent_registry,
                test_approval_flow(&session_manager),
            ))
            .await
            .unwrap();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let section = build_tools_section(&registry, &ctx, None, None, None, None).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        assert!(
            content.contains("file_ops"),
            "missing file_ops group: {}",
            content
        );
        assert!(content.contains("meta"), "missing meta group: {}", content);
    }

    #[tokio::test]
    async fn test_build_tools_section_contains_tool_names() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (spawn_controller, session_manager, config_manager, agent_registry) = test_spawn_deps();
        registry
            .register_all(make_registrars(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager.clone(),
                config_manager,
                agent_registry,
                test_approval_flow(&session_manager),
            ))
            .await
            .unwrap();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let section = build_tools_section(&registry, &ctx, None, None, None, None).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        for name in &[
            "Read",
            "Write",
            "Edit",
            "Grep",
            "Ls",
            "ToolSearch",
            "PermissionQuery",
        ] {
            assert!(
                content.contains(name),
                "tool {} not found in: {}",
                name,
                content
            );
        }
    }

    #[tokio::test]
    async fn test_build_tools_section_respects_max_length() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (spawn_controller, session_manager, config_manager, agent_registry) = test_spawn_deps();
        registry
            .register_all(make_registrars(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager.clone(),
                config_manager,
                agent_registry,
                test_approval_flow(&session_manager),
            ))
            .await
            .unwrap();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let section = build_tools_section(&registry, &ctx, None, None, None, None).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        assert!(
            content.chars().count() <= 15000,
            "section too long: {}",
            content
        );
    }

    #[tokio::test]
    async fn test_build_tools_section_empty_registry() {
        let registry = ToolRegistry::new();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let section = build_tools_section(&registry, &ctx, None, None, None, None).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        assert!(
            content.is_empty(),
            "expected empty content, got: {}",
            content
        );
    }

    #[tokio::test]
    async fn test_task_writing_guidance_when_spawn_available() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (spawn_controller, session_manager, config_manager, agent_registry) = test_spawn_deps();
        registry
            .register_all(make_registrars(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager.clone(),
                config_manager,
                agent_registry,
                test_approval_flow(&session_manager),
            ))
            .await
            .unwrap();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let section = build_tools_section(&registry, &ctx, None, None, None, None).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        assert!(
            content.contains("smart colleague"),
            "missing 'smart colleague' in guidance: {}",
            &content[content.len().min(content.len().saturating_sub(500))..]
        );
        assert!(
            content.contains("judgment calls"),
            "missing 'judgment calls' in guidance"
        );
        assert!(
            content.contains("fork mode"),
            "missing 'fork mode' in guidance"
        );
    }

    #[tokio::test]
    async fn test_task_writing_guidance_absent_when_spawn_unavailable() {
        // Empty registry → sessions_spawn is not in available_tool_names
        let registry = ToolRegistry::new();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let section = build_tools_section(&registry, &ctx, None, None, None, None).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        assert!(
            !content.contains("smart colleague"),
            "task writing guidance should NOT appear without sessions_spawn, got: {}",
            content
        );
        assert!(
            !content.contains("judgment calls"),
            "task writing guidance should NOT appear without sessions_spawn"
        );
        assert!(
            !content.contains("fork mode"),
            "task writing guidance should NOT appear without sessions_spawn"
        );
    }

    #[tokio::test]
    async fn test_background_task_guidance_when_bash_available() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (spawn_controller, session_manager, config_manager, agent_registry) = test_spawn_deps();
        registry
            .register_all(make_registrars(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager.clone(),
                config_manager,
                agent_registry,
                test_approval_flow(&session_manager),
            ))
            .await
            .unwrap();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let section = build_tools_section(&registry, &ctx, None, None, None, None).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        assert!(
            content.contains("Background Task Guidance"),
            "missing 'Background Task Guidance' header in: {}",
            &content[content.len().saturating_sub(300)..]
        );
        assert!(
            content.contains("do not need to poll"),
            "missing 'do not need to poll' in background guidance"
        );
        assert!(
            content.contains("Do not call process-query tools"),
            "missing 'Do not call process-query tools' in background guidance"
        );
        assert!(
            content.contains("10 seconds"),
            "missing '10 seconds' threshold in background guidance"
        );
    }

    #[tokio::test]
    async fn test_background_task_guidance_absent_when_bash_unavailable() {
        // Empty registry → Bash is not in available_tool_names
        let registry = ToolRegistry::new();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        let section = build_tools_section(&registry, &ctx, None, None, None, None).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        assert!(
            !content.contains("Background Task Guidance"),
            "background task guidance should NOT appear without Bash, got: {}",
            content
        );
        assert!(
            !content.contains("do not need to poll"),
            "background guidance text should NOT appear without Bash"
        );
    }

    #[tokio::test]
    async fn test_budget_zero_filters_sessions_spawn() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (spawn_controller, session_manager, config_manager, agent_registry) = test_spawn_deps();
        registry
            .register_all(make_registrars(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager.clone(),
                config_manager,
                agent_registry,
                test_approval_flow(&session_manager),
            ))
            .await
            .unwrap();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        // Budget = 0 → sessions_spawn should be filtered out.
        let section = build_tools_section(&registry, &ctx, None, None, None, Some(0)).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        assert!(
            !content.contains("sessions_spawn"),
            "sessions_spawn should be filtered when budget = 0, got: {}",
            content
        );
        // Other tools should still be present.
        assert!(
            content.contains("file_ops"),
            "file_ops should still be present"
        );
    }

    #[tokio::test]
    async fn test_budget_one_keeps_sessions_spawn() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (spawn_controller, session_manager, config_manager, agent_registry) = test_spawn_deps();
        registry
            .register_all(make_registrars(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager.clone(),
                config_manager,
                agent_registry,
                test_approval_flow(&session_manager),
            ))
            .await
            .unwrap();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        };
        // Budget = 1 → sessions_spawn should be present.
        let section = build_tools_section(&registry, &ctx, None, None, None, Some(1)).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        assert!(
            content.contains("sessions_spawn"),
            "sessions_spawn should be present when budget = 1, got: {}",
            content
        );
    }
}
