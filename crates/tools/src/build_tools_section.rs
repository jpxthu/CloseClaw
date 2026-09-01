//! Tools section builder for the system prompt.
//!
//! Owns the `build_tools_section` function and its tests. Migrated from
//! `system_prompt::tools_section` to keep tool-related domain logic in
//! the `closeclaw-tools` crate.
//!
//! Returns a plain `String` (not `Section`) so the caller can wrap it
//! into a `PromptFragment` without depending on system_prompt internals.

use crate::{PromptGenerationContext, ToolContext, ToolRegistry};
use closeclaw_common::SessionMode;

/// Parameters for [`build_tools_section`].
///
/// Groups the agent-level and session-level parameters that influence
/// tool listing generation.  Keeps the function signature within the
/// CONTRIBUTING.md 6-parameter limit.
pub struct ToolsSectionParams {
    /// Agent-level tool whitelist from config.
    pub agent_tools: Option<Vec<String>>,
    /// Agent-level tool blacklist from config.
    pub agent_disallowed_tools: Option<Vec<String>>,
    /// Session mode for mode-aware tool filtering.
    pub session_mode: Option<SessionMode>,
    /// Agent role (human-readable name/purpose) from agent config.
    pub agent_role: Option<String>,
    /// Agent type (e.g. root agent vs spawned child) from agent config.
    pub agent_type: Option<String>,
}

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

/// Parallel tool calls guidance appended to the tools section.
/// Source: docs/design/tools/multi-tool-calls.md §提示词引导
const PARALLEL_TOOL_CALLS_GUIDANCE: &str = concat!(
    "\n\n## Parallel Tool Calls\n\n\n",
    "You may invoke multiple tools in a single message to reduce round-trips:\n",
    "- **Multiple reads** (Read, Ls, Grep): always merge into one message. ",
    "They are safe to run concurrently.\n",
    "- **Multiple Edits to the same file**: merge into a single `edits[]` call. ",
    "Separate Edit calls to the same file will conflict.\n",
    "- **Edits to different files**: issue in parallel — they run concurrently.\n",
    "- **Read file A + Edit file B**: different files, safe to parallelize.\n",
    "- **Expensive tools** (Grep, Bash): control concurrency; avoid launching too ",
    "many at once."
);

/// Build the Tools section content from a registry.
///
/// The registry's `build_tools_section` requires a [`PromptGenerationContext`]
/// (which carries the list of available tool names, the agent id, and the
/// workdir). We acquire that list via a single short-lived lock on
/// `list_descriptors`, release it, and then call the registry's
/// `build_tools_section` with the freshly-built context. This keeps locks
/// non-overlapping.
///
/// Returns the rendered tools section as a plain `String`. An empty string
/// signals that there is no content to contribute.
pub async fn build_tools_section(
    registry: &ToolRegistry,
    ctx: &ToolContext,
    params: &ToolsSectionParams,
) -> String {
    // 1. Independent lock: get available tool names, then drop the lock.
    let descriptors: Vec<crate::ToolSummary> = registry.list_descriptors(ctx).await;
    let available_tool_names: Vec<String> = descriptors.into_iter().map(|d| d.name).collect();

    // 2. Resolve agent-level tool filtering.
    //    Priority: explicit parameters > AgentRegistry query.
    let (tools, disallowed_tools) =
        if params.agent_tools.is_some() || params.agent_disallowed_tools.is_some() {
            (
                params.agent_tools.clone(),
                params.agent_disallowed_tools.clone(),
            )
        } else {
            // Query AgentRegistry directly (design-doc query path).
            registry.query_agent_tools_config(&ctx.agent_id).await
        };

    // 3. Build the prompt-generation context from the names + the existing
    //    execution context, including agent-level tool filtering and
    //    agent definition (role and type).
    let prompt_ctx = PromptGenerationContext {
        agent_id: ctx.agent_id.clone(),
        workdir: ctx.workdir.clone(),
        available_tool_names,
        tools,
        disallowed_tools,
        session_mode: params.session_mode,
        agent_role: params.agent_role.clone(),
        agent_type: params.agent_type.clone(),
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

    // 7. Append parallel tool calls guidance when content is non-empty.
    let content = if !content.is_empty() {
        let guidance = PARALLEL_TOOL_CALLS_GUIDANCE;
        format!("{}\n{}", content, guidance)
    } else {
        content
    };

    content
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::SkillTool;
    use crate::test_adapters::{ApprovalFlowAdapter, PermissionEngineAdapter};
    use crate::{CoreToolsRegistrar, PlanToolsRegistrar, SkillsToolsRegistrar, ToolRegistrar};
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
    use closeclaw_session::tools::SessionToolsRegistrar;
    use closeclaw_skills::DiskSkillRegistry;
    use closeclaw_tasks::BackgroundTaskManager;
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
                spawn_controller.clone() as Arc<dyn crate::SpawnValidator>,
                session_manager.clone() as Arc<dyn closeclaw_session::tools::SessionManagerOps>,
                agent_registry.clone() as Arc<dyn closeclaw_agent::AgentConfigLookup>,
                Arc::new(PermissionEngineAdapter(permission_engine)),
                Arc::new(tokio::sync::Mutex::new(ApprovalFlowAdapter(
                    approval_flow.clone(),
                ))),
            )),
            Box::new(SkillsToolsRegistrar::new(Arc::new(SkillTool::new(
                disk_registry,
                Arc::new(closeclaw_skills::BuiltinSkillRegistry::new()),
            )))),
            Box::new(PlanToolsRegistrar::new(
                Arc::new(Mutex::new(PlanState::new())),
                session_manager.clone(),
                approval_flow.clone(),
            )),
        ]
    }

    #[tokio::test]
    async fn test_build_tools_section_returns_string() {
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
        let ctx = crate::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        let content = build_tools_section(
            &registry,
            &ctx,
            &ToolsSectionParams {
                agent_tools: None,
                agent_disallowed_tools: None,
                session_mode: None,
                agent_role: None,
                agent_type: None,
            },
        )
        .await;
        assert!(!content.is_empty());
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
        let ctx = crate::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        let content = build_tools_section(
            &registry,
            &ctx,
            &ToolsSectionParams {
                agent_tools: None,
                agent_disallowed_tools: None,
                session_mode: None,
                agent_role: None,
                agent_type: None,
            },
        )
        .await;
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
        let ctx = crate::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        let content = build_tools_section(
            &registry,
            &ctx,
            &ToolsSectionParams {
                agent_tools: None,
                agent_disallowed_tools: None,
                session_mode: None,
                agent_role: None,
                agent_type: None,
            },
        )
        .await;
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
        let ctx = crate::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        let content = build_tools_section(
            &registry,
            &ctx,
            &ToolsSectionParams {
                agent_tools: None,
                agent_disallowed_tools: None,
                session_mode: None,
                agent_role: None,
                agent_type: None,
            },
        )
        .await;
        assert!(
            content.chars().count() <= 15000,
            "section too long: {}",
            content
        );
    }

    #[tokio::test]
    async fn test_build_tools_section_empty_registry() {
        let registry = ToolRegistry::new();
        let ctx = crate::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        let content = build_tools_section(
            &registry,
            &ctx,
            &ToolsSectionParams {
                agent_tools: None,
                agent_disallowed_tools: None,
                session_mode: None,
                agent_role: None,
                agent_type: None,
            },
        )
        .await;
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
        let ctx = crate::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        let content = build_tools_section(
            &registry,
            &ctx,
            &ToolsSectionParams {
                agent_tools: None,
                agent_disallowed_tools: None,
                session_mode: None,
                agent_role: None,
                agent_type: None,
            },
        )
        .await;
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
        let ctx = crate::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        let content = build_tools_section(
            &registry,
            &ctx,
            &ToolsSectionParams {
                agent_tools: None,
                agent_disallowed_tools: None,
                session_mode: None,
                agent_role: None,
                agent_type: None,
            },
        )
        .await;
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
        let ctx = crate::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        let content = build_tools_section(
            &registry,
            &ctx,
            &ToolsSectionParams {
                agent_tools: None,
                agent_disallowed_tools: None,
                session_mode: None,
                agent_role: None,
                agent_type: None,
            },
        )
        .await;
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
        let ctx = crate::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        let content = build_tools_section(
            &registry,
            &ctx,
            &ToolsSectionParams {
                agent_tools: None,
                agent_disallowed_tools: None,
                session_mode: None,
                agent_role: None,
                agent_type: None,
            },
        )
        .await;
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
    async fn test_sessions_spawn_always_present_in_tools() {
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
        let ctx = crate::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        // sessions_spawn is always visible in the tools section
        // (budget filtering is now handled at session creation time).
        let content = build_tools_section(
            &registry,
            &ctx,
            &ToolsSectionParams {
                agent_tools: None,
                agent_disallowed_tools: None,
                session_mode: None,
                agent_role: None,
                agent_type: None,
            },
        )
        .await;
        assert!(
            content.contains("sessions_spawn"),
            "sessions_spawn should always be present in tools section, got: {}",
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
        let ctx = crate::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        // Budget = 1 → sessions_spawn should be present.
        let content = build_tools_section(
            &registry,
            &ctx,
            &ToolsSectionParams {
                agent_tools: None,
                agent_disallowed_tools: None,
                session_mode: None,
                agent_role: None,
                agent_type: None,
            },
        )
        .await;
        assert!(
            content.contains("sessions_spawn"),
            "sessions_spawn should be present when budget = 1, got: {}",
            content
        );
    }

    #[tokio::test]
    async fn test_parallel_tool_calls_guidance_present() {
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
        let ctx = crate::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        let content = build_tools_section(
            &registry,
            &ctx,
            &ToolsSectionParams {
                agent_tools: None,
                agent_disallowed_tools: None,
                session_mode: None,
                agent_role: None,
                agent_type: None,
            },
        )
        .await;
        assert!(
            content.contains("Parallel Tool Calls"),
            "missing 'Parallel Tool Calls' header in: {}",
            &content[content.len().saturating_sub(300)..]
        );
        assert!(
            content.contains("Multiple reads"),
            "missing 'Multiple reads' in parallel guidance"
        );
        assert!(
            content.contains("edits[]"),
            "missing 'edits[]' in parallel guidance"
        );
        assert!(
            content.contains("Expensive tools"),
            "missing 'Expensive tools' in parallel guidance"
        );
    }

    #[tokio::test]
    async fn test_parallel_tool_calls_guidance_absent_empty_registry() {
        let registry = ToolRegistry::new();
        let ctx = crate::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        };
        let content = build_tools_section(
            &registry,
            &ctx,
            &ToolsSectionParams {
                agent_tools: None,
                agent_disallowed_tools: None,
                session_mode: None,
                agent_role: None,
                agent_type: None,
            },
        )
        .await;
        assert!(
            !content.contains("Parallel Tool Calls"),
            "parallel guidance should NOT appear with empty registry, got: {}",
            content
        );
    }
}
