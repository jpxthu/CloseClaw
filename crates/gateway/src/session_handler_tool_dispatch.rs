//! Tool-call dispatch integration for `SessionMessageHandler`.
//!
//! When the LLM response contains one or more `ContentBlock::ToolUse`
//! blocks, this module extracts them, builds [`PendingToolCall`] entries,
//! runs them through the [`ToolCallDispatcher`], and converts the
//! results into `ContentBlock::ToolResult` blocks.

use std::sync::Arc;

use closeclaw_common::dispatcher::{PendingToolCall, ToolCallDispatcher, ToolExecutor};
use closeclaw_common::tool_trait::{ToolCallError, ToolContext, ToolResult};
use closeclaw_common::ToolRegistryQuery;
use closeclaw_llm::types::ContentBlock;

use super::session_handler::SessionMessageHandler;
use crate::session_manager::SessionManager;

// ── Permission dependencies ─────────────────────────────────────────

/// Bundled permission dependencies for tool-call dispatch.
///
/// Mirrors `closeclaw_tools::permission_check::PermDeps` but uses
/// types available in the gateway crate to avoid a circular dependency
/// on `closeclaw-tools`.
type ToolPermDeps = (
    Arc<tokio::sync::RwLock<closeclaw_permission::engine::engine_eval::PermissionEngine>>,
    Arc<SessionManager>,
    Arc<closeclaw_config::manager::ConfigManager>,
    Arc<tokio::sync::Mutex<closeclaw_permission::approval_flow::ApprovalFlow>>,
);

// ── PendingToolCall construction ────────────────────────────────────

/// Build [`PendingToolCall`] entries from raw `ToolUse` content blocks.
///
/// Each `ContentBlock::ToolUse { id, name, input }` is converted into
/// a [`PendingToolCall`] using the registry to look up the
/// `is_concurrency_safe` flag.
async fn build_pending_calls(
    tool_uses: &[(String, String, String)],
    registry: &dyn ToolRegistryQuery,
) -> Vec<PendingToolCall> {
    let mut calls = Vec::with_capacity(tool_uses.len());
    for (id, name, input) in tool_uses {
        let args: serde_json::Value =
            serde_json::from_str(input).unwrap_or_else(|_| serde_json::json!({}));
        let is_concurrency_safe = registry
            .get_tool_concurrency_safe(name)
            .await
            .unwrap_or(false);
        let file_path = closeclaw_common::dispatcher::extract_file_path(&args);
        calls.push(PendingToolCall {
            id: id.clone(),
            tool_name: name.clone(),
            args,
            file_path,
            is_concurrency_safe,
        });
    }
    calls
}

// ── TraitObjectExecutor ─────────────────────────────────────────────

/// A [`ToolExecutor`] that delegates to [`ToolRegistryQuery::call_tool`].
///
/// Optionally performs permission checks when `perm_deps` is provided,
/// using the same check logic as the tools-crate `ToolRegistryExecutor`.
struct TraitObjectExecutor {
    registry: Arc<dyn ToolRegistryQuery>,
    base_ctx: ToolContext,
    perm_deps: Option<ToolPermDeps>,
}

impl TraitObjectExecutor {
    fn new(registry: Arc<dyn ToolRegistryQuery>, base_ctx: ToolContext) -> Self {
        Self {
            registry,
            base_ctx,
            perm_deps: None,
        }
    }

    /// Inject permission dependencies for centralized permission checks.
    fn with_perm_deps(mut self, perm_deps: ToolPermDeps) -> Self {
        self.perm_deps = Some(perm_deps);
        self
    }
}

/// Resolve the permission caller's User ID for a session.
///
/// Design doc (`docs/design/permission/README.md`, "代理 User"): "User 来源
/// 取决于场景——IM 消息来自发送者、CLI 调用默认为 Owner……". The session
/// checkpoint's recorded sender is the source those scenarios resolve to
/// (same as the tools-crate permission path — `SessionManager::get_sender_id`,
/// see `crates/tools/src/permission_check.rs`); the session id is only the
/// lookup key, never the identity itself.
///
/// When no sender context exists (no checkpoint / no `sender_id`), falls
/// back to an empty User ID — the engine treats that as a system caller
/// (User phase skipped, mirroring a caller-less `Bare` request).
async fn resolve_caller_user_id(session_mgr: &SessionManager, ctx: &ToolContext) -> String {
    match ctx.session_id.as_deref() {
        Some(sid) if !sid.is_empty() => session_mgr.get_sender_id(sid).await.unwrap_or_default(),
        _ => String::new(),
    }
}

#[async_trait::async_trait]
impl ToolExecutor for TraitObjectExecutor {
    async fn execute(&self, call: &PendingToolCall) -> ToolResult {
        let mut ctx = self.base_ctx.clone();
        ctx.call_id = Some(call.id.clone());

        // --- Permission check (Level 1: ToolCall) ---
        if let Some(ref perm_deps) = self.perm_deps {
            let (perm_engine, session_mgr, _config_mgr, _approval_flow) = perm_deps;
            let tool_group = self
                .registry
                .get_tool_detail(&call.tool_name)
                .await
                .map(|d| d.group)
                .unwrap_or_default();

            let agent_id = if ctx.agent_id.is_empty() {
                String::new()
            } else {
                ctx.agent_id.clone()
            };
            let user_id = resolve_caller_user_id(session_mgr, &ctx).await;

            // Level 1: tool-group permission check.
            let request =
                closeclaw_permission::engine::engine_types::PermissionRequest::WithCaller {
                    caller: closeclaw_permission::engine::engine_types::Caller {
                        user_id: user_id.clone(),
                        agent: agent_id.clone(),
                    },
                    request: closeclaw_permission::engine::engine_types::PermissionRequestBody::ToolCall {
                        agent: agent_id.clone(),
                        skill: tool_group.clone(),
                        method: call.tool_name.clone(),
                    },
                };
            let engine = perm_engine.read().await;
            let eval_result = engine.evaluate(request, None);
            drop(engine);

            match eval_result {
                closeclaw_permission::engine::engine_types::PermissionResponse::Allowed {
                    ..
                } => {}
                closeclaw_permission::engine::engine_types::PermissionResponse::Denied {
                    reason,
                    ..
                } => {
                    return ToolResult {
                        data: serde_json::json!({
                            "error": format!(
                                "tool '{}' in group '{}' is not permitted by policy: {}",
                                call.tool_name, tool_group, reason
                            )
                        }),
                        new_messages: vec![],
                        context_modifier: None,
                    };
                }
            }

            // --- Level 2: Domain-specific permission checks ---
            match tool_group.as_str() {
                "bash" => {
                    if let Some(full_cmd) =
                        call.args.get("command").and_then(serde_json::Value::as_str)
                    {
                        let parts: Vec<&str> = full_cmd.split_whitespace().collect();
                        let base_cmd = parts.first().copied().unwrap_or("");
                        let cmd_args: Vec<String> =
                            parts[1..].iter().map(|s| s.to_string()).collect();
                        let cmd_request =
                            closeclaw_permission::engine::engine_types::PermissionRequest::WithCaller {
                                caller: closeclaw_permission::engine::engine_types::Caller {
                                    user_id: user_id.clone(),
                                    agent: agent_id.clone(),
                                },
                                request: closeclaw_permission::engine::engine_types::PermissionRequestBody::CommandExec {
                                    agent: agent_id.clone(),
                                    cmd: base_cmd.to_string(),
                                    args: cmd_args,
                                },
                            };
                        let engine = perm_engine.read().await;
                        let cmd_result = engine.evaluate(cmd_request, None);
                        drop(engine);

                        if let closeclaw_permission::engine::engine_types::PermissionResponse::Denied {
                            reason,
                            ..
                        } = cmd_result
                        {
                            return ToolResult {
                                data: serde_json::json!({
                                    "error": format!(
                                        "command '{}' is not permitted by policy: {}",
                                        base_cmd, reason
                                    )
                                }),
                                new_messages: vec![],
                                context_modifier: None,
                            };
                        }
                    }
                }
                "file_ops" => {
                    if let Some(path) = call.args.get("path").and_then(serde_json::Value::as_str) {
                        let is_read_only = self
                            .registry
                            .get_tool_detail(&call.tool_name)
                            .await
                            .map(|d| d.flags.is_read_only)
                            .unwrap_or(false);
                        let op = if is_read_only { "read" } else { "write" };
                        let file_request =
                            closeclaw_permission::engine::engine_types::PermissionRequest::WithCaller {
                                caller: closeclaw_permission::engine::engine_types::Caller {
                                    user_id: user_id.clone(),
                                    agent: agent_id.clone(),
                                },
                                request: closeclaw_permission::engine::engine_types::PermissionRequestBody::FileOp {
                                    agent: agent_id.clone(),
                                    path: path.to_string(),
                                    op: op.to_string(),
                                },
                            };
                        let engine = perm_engine.read().await;
                        let file_result = engine.evaluate(file_request, None);
                        drop(engine);

                        if let closeclaw_permission::engine::engine_types::PermissionResponse::Denied {
                            reason,
                            ..
                        } = file_result
                        {
                            return ToolResult {
                                data: serde_json::json!({
                                    "error": format!(
                                        "file operation '{}' on '{}' is not permitted by policy: {}",
                                        op, path, reason
                                    )
                                }),
                                new_messages: vec![],
                                context_modifier: None,
                            };
                        }
                    }
                }
                _ => {}
            }
        }

        match self
            .registry
            .call_tool(&call.tool_name, call.args.clone(), &ctx)
            .await
        {
            Ok(result) => result,
            Err(ToolCallError::NotFound(name)) => ToolResult {
                data: serde_json::json!({ "error": format!("tool not found: {name}") }),
                new_messages: vec![],
                context_modifier: None,
            },
            Err(e) => ToolResult {
                data: serde_json::json!({ "error": e.to_string() }),
                new_messages: vec![],
                context_modifier: None,
            },
        }
    }
}

// ── Tool result → ContentBlock conversion ───────────────────────────

/// Convert a [`ToolResult`] into a `ContentBlock::ToolResult`.
fn tool_result_to_content_block(call_id: &str, result: &ToolResult) -> ContentBlock {
    ContentBlock::ToolResult {
        tool_call_id: call_id.to_string(),
        content: result.data.to_string(),
    }
}

// ── Build PermDeps from Gateway ─────────────────────────────────────

/// Try to build [`ToolPermDeps`] from the [`Gateway`] and [`SessionManager`].
///
/// Returns `None` if any required component (permission engine, config
/// manager, or approval flow) is not configured.
async fn build_tool_perm_deps(
    gateway: &crate::Gateway,
    session_manager: &Arc<SessionManager>,
) -> Option<ToolPermDeps> {
    let perm_engine = gateway.get_permission_engine().await?;
    let approval_flow = gateway.get_approval_flow().await?;
    let config_manager = session_manager.get_config_manager().await?;
    Some((
        perm_engine,
        Arc::clone(session_manager),
        config_manager,
        approval_flow,
    ))
}

// ── Public integration point ────────────────────────────────────────

impl SessionMessageHandler {
    /// Dispatch tool calls when the LLM response contains `ToolUse` blocks.
    ///
    /// Returns `Some(new_blocks)` if tool calls were dispatched (the
    /// caller should append these instead of the original `ToolUse`
    /// blocks). Returns `None` if there were no tool calls.
    pub(super) async fn dispatch_tool_calls_if_needed(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        content_blocks: &[ContentBlock],
        file_mutex_map: &Arc<closeclaw_common::file_mutex::FileMutexMap>,
        gateway: Option<&Arc<crate::Gateway>>,
    ) -> Option<Vec<ContentBlock>> {
        // 1. Extract ToolUse blocks.
        let tool_uses: Vec<(String, String, String)> = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolUse { id, name, input } => {
                    Some((id.clone(), name.clone(), input.clone()))
                }
                _ => None,
            })
            .collect();
        if tool_uses.is_empty() {
            return None;
        }

        // 2. Get tool registry.
        let registry = session_manager.get_tool_registry().await?;
        let registry_ref: &dyn ToolRegistryQuery = registry.as_ref();

        // 3. Build PendingToolCall list.
        let calls = build_pending_calls(&tool_uses, registry_ref).await;

        // 4. Determine parallel mode.
        let agent_id = session_manager.get_chat_id(session_id).await;
        let is_parallel_enabled = if let Some(ref aid) = agent_id {
            session_manager
                .get_agent_config(aid)
                .await
                .map(|c| c.parallel_tool_calls)
                .unwrap_or(true)
        } else {
            true
        };

        // 5. Create dispatcher.
        let dispatcher = ToolCallDispatcher::new(Arc::clone(file_mutex_map), is_parallel_enabled);

        // 6. Fetch conversation session once and reuse for provider check + ToolContext.
        let (provider_supports_parallel, base_ctx) =
            if let Some(cs) = session_manager.get_conversation_session(session_id).await {
                let cs_read = cs.read().await;
                let provider_supports_parallel = cs_read
                    .llm_caller()
                    .map(|c| c.supports_parallel_tool_calls())
                    .unwrap_or(false);
                let workdir = cs_read.workdir().to_path_buf();
                let workdir_ctx =
                    closeclaw_common::tool_trait::build_workdir_context(&workdir.to_string_lossy());
                (
                    provider_supports_parallel,
                    ToolContext {
                        agent_id: agent_id.unwrap_or_default(),
                        workdir: Some(workdir_ctx),
                        session_id: Some(session_id.to_string()),
                        call_id: None,
                        session: None,
                        session_mode: None,
                        manual_background_signal: None,
                        media_store: None,
                    },
                )
            } else {
                (
                    false,
                    ToolContext {
                        agent_id: agent_id.unwrap_or_default(),
                        workdir: None,
                        session_id: Some(session_id.to_string()),
                        call_id: None,
                        session: None,
                        session_mode: None,
                        manual_background_signal: None,
                        media_store: None,
                    },
                )
            };

        // 7. Build permission deps (optional — absent means no permission checks).
        let perm_deps = if let Some(gw) = gateway {
            build_tool_perm_deps(gw, session_manager).await
        } else {
            None
        };

        // 8. Execute.
        let mut executor = TraitObjectExecutor::new(Arc::clone(&registry), base_ctx);
        if let Some(deps) = perm_deps {
            executor = executor.with_perm_deps(deps);
        }
        let results = dispatcher
            .dispatch_all(calls, &executor, provider_supports_parallel)
            .await;

        // 9. Convert results to ContentBlock::ToolResult, preserving order.
        let tool_result_blocks: Vec<ContentBlock> = tool_uses
            .iter()
            .zip(results.iter())
            .map(|((id, _, _), result)| tool_result_to_content_block(id, result))
            .collect();

        Some(tool_result_blocks)
    }

    /// Replace `ToolUse` blocks in a response with executed `ToolResult` blocks.
    ///
    /// If the response contains `ToolUse` blocks, dispatches them through
    /// the `ToolCallDispatcher` and returns a modified response with
    /// `ToolResult` blocks instead. Non-ToolUse blocks are preserved.
    pub(super) async fn maybe_dispatch_tool_calls(
        session_manager: &Arc<SessionManager>,
        session_id: &str,
        content_blocks: Vec<ContentBlock>,
        file_mutex_map: &Arc<closeclaw_common::file_mutex::FileMutexMap>,
        gateway: Option<&Arc<crate::Gateway>>,
    ) -> Vec<ContentBlock> {
        let has_tool_use = content_blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
        if !has_tool_use {
            return content_blocks;
        }

        match Self::dispatch_tool_calls_if_needed(
            session_manager,
            session_id,
            &content_blocks,
            file_mutex_map,
            gateway,
        )
        .await
        {
            Some(tool_result_blocks) => {
                // Keep non-ToolUse blocks, append tool results.
                let mut output: Vec<ContentBlock> = content_blocks
                    .into_iter()
                    .filter(|b| !matches!(b, ContentBlock::ToolUse { .. }))
                    .collect();
                output.extend(tool_result_blocks);
                output
            }
            None => content_blocks,
        }
    }
}

// ── Tests: permission caller user_id wiring (plan Step 1.6) ─────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GatewayConfig;
    use closeclaw_common::{
        PendingMessage, PlanState, SessionLookup, SessionMode, ToolDescriptor, ToolFlags,
    };
    use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
    use closeclaw_session::persistence::{
        PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint,
    };
    use std::collections::HashMap;

    // ── Mock persistence (checkpoint / sender_id lookups) ─────────────────

    struct MockPersist(tokio::sync::Mutex<HashMap<String, SessionCheckpoint>>);

    impl MockPersist {
        fn new() -> Self {
            Self(tokio::sync::Mutex::new(HashMap::new()))
        }

        async fn put(&self, cp: &SessionCheckpoint) {
            self.0
                .lock()
                .await
                .insert(cp.session_id.clone(), cp.clone());
        }
    }

    #[async_trait::async_trait]
    impl PersistenceService for MockPersist {
        async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
            self.put(cp).await;
            Ok(())
        }
        async fn load_checkpoint(
            &self,
            sid: &str,
        ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
            Ok(self.0.lock().await.get(sid).cloned())
        }
        async fn delete_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
            Ok(())
        }
        async fn purge_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
            Ok(())
        }
        async fn archive_checkpoint(
            &self,
            _cp: &SessionCheckpoint,
        ) -> Result<(), PersistenceError> {
            Ok(())
        }
        async fn restore_checkpoint(
            &self,
            _sid: &str,
        ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
            Ok(None)
        }
        async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
            Ok(vec![])
        }
    }

    fn make_session_manager(persist: &Arc<MockPersist>) -> SessionManager {
        SessionManager::new(
            &GatewayConfig::default(),
            Some(Arc::clone(persist) as Arc<dyn PersistenceService>),
            None,
            ReasoningLevel::default(),
        )
    }

    /// SessionManager whose checkpoint for `sid` records `sender` as sender_id.
    async fn sm_with_sender(sid: &str, sender: Option<&str>) -> SessionManager {
        let persist = Arc::new(MockPersist::new());
        let mut cp = SessionCheckpoint::new(sid.to_string());
        cp.sender_id = sender.map(str::to_string);
        persist.put(&cp).await;
        make_session_manager(&persist)
    }

    fn tool_ctx(session_id: Option<&str>) -> ToolContext {
        ToolContext {
            agent_id: "master".to_string(),
            workdir: None,
            session_id: session_id.map(str::to_string),
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        }
    }

    // ── Dimension ③: user_id source = checkpoint sender, never session id ─

    /// The resolved caller user_id is the checkpoint's recorded sender;
    /// the session id is only the lookup key, never the identity.
    #[tokio::test]
    async fn resolve_caller_user_id_is_checkpoint_sender_not_session_id() {
        let sid = "sess-wiring-source";
        let sm = sm_with_sender(sid, Some("u_1001")).await;
        let uid = resolve_caller_user_id(&sm, &tool_ctx(Some(sid))).await;
        assert_eq!(uid, "u_1001");
        assert_ne!(uid, sid, "session id must never be used as user_id");
    }

    /// A recorded "owner" sender passes through, keeping the design
    /// Owner shortcut ("CLI 调用默认为 Owner") reachable from dispatch.
    #[tokio::test]
    async fn resolve_caller_user_id_passes_owner_through() {
        let sid = "sess-owner-sender";
        let sm = sm_with_sender(sid, Some("owner")).await;
        let uid = resolve_caller_user_id(&sm, &tool_ctx(Some(sid))).await;
        assert_eq!(uid, "owner");
    }

    // ── Dimension ④: fallback → empty user_id ─────────────────────────────

    /// No checkpoint / checkpoint without sender_id / missing or empty
    /// session id all fall back to an empty user_id (engine empty-uid
    /// branch: User phase skipped, Agent dimension decides alone).
    #[tokio::test]
    async fn resolve_caller_user_id_falls_back_to_empty() {
        let sm = sm_with_sender("sess-no-sender", None).await;
        let no_sender = resolve_caller_user_id(&sm, &tool_ctx(Some("sess-no-sender"))).await;
        assert_eq!(no_sender, "", "checkpoint without sender_id");
        let no_ckpt = resolve_caller_user_id(&sm, &tool_ctx(Some("sess-unknown"))).await;
        assert_eq!(no_ckpt, "", "session without checkpoint");
        let no_sid = resolve_caller_user_id(&sm, &tool_ctx(None)).await;
        assert_eq!(no_sid, "", "missing session_id");
        let empty_sid = resolve_caller_user_id(&sm, &tool_ctx(Some(""))).await;
        assert_eq!(empty_sid, "", "empty session_id");
    }

    // ── Executor-level integration: PermissionRequest built from the wiring ─

    struct ProbeRegistry;

    #[async_trait::async_trait]
    impl ToolRegistryQuery for ProbeRegistry {
        async fn list_tool_names(&self) -> Vec<String> {
            vec!["probe_tool".to_string()]
        }
        async fn get_tool_descriptors(
            &self,
            _agent_id: Option<&str>,
            _agent_tools: Option<&[String]>,
            _agent_disallowed_tools: Option<&[String]>,
        ) -> Vec<ToolDescriptor> {
            vec![]
        }
        async fn has_tool(&self, name: &str) -> bool {
            name == "probe_tool"
        }
        async fn get_tool_schema(&self, _name: &str) -> Option<serde_json::Value> {
            None
        }
        async fn get_tool_detail(&self, name: &str) -> Option<ToolDescriptor> {
            (name == "probe_tool").then(|| ToolDescriptor {
                name: "probe_tool".to_string(),
                group: "test_tools".to_string(),
                summary: String::new(),
                detail: String::new(),
                input_schema: serde_json::json!({}),
                flags: ToolFlags {
                    is_concurrency_safe: true,
                    is_read_only: true,
                    is_destructive: false,
                    is_expensive: false,
                    is_deferred_by_default: false,
                },
            })
        }
        async fn list_tool_names_by_group(&self, _group: &str) -> Vec<String> {
            vec![]
        }
        async fn get_tool_concurrency_safe(&self, _name: &str) -> Option<bool> {
            Some(true)
        }
        async fn call_tool(
            &self,
            _name: &str,
            _args: serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<ToolResult, ToolCallError> {
            Ok(ToolResult {
                data: serde_json::json!({"content": "PROBE_OK"}),
                new_messages: vec![],
                context_modifier: None,
            })
        }
    }

    struct ProbeLookup;

    #[async_trait::async_trait]
    impl SessionLookup for ProbeLookup {
        async fn get_parent_of(&self, _child_id: &str) -> Option<String> {
            None
        }
        async fn get_chat_id(&self, _session_id: &str) -> Option<String> {
            None
        }
        async fn push_pending_message(&self, _: &str, _: PendingMessage) -> Result<(), String> {
            Ok(())
        }
        async fn get_plan_state(&self, _: &str) -> Option<PlanState> {
            None
        }
        async fn set_plan_state(&self, _: &str, _: PlanState) {}
        async fn set_session_mode(&self, _: &str, _: SessionMode) {}
    }

    /// Engine rules: Agent Allow + UserAndAgent("u_1001") Allow for
    /// `test_tools::probe_tool`; everything else (Agent/User defaults)
    /// Deny. So the tool only executes when the PermissionRequest's
    /// caller user_id resolves to `u_1001` (or an empty uid — the
    /// documented fallback); a leaked session id would miss the User
    /// rule and hit user_defaults → Deny.
    fn probe_ruleset() -> closeclaw_permission::RuleSet {
        use closeclaw_permission::{Action, Effect, MatchType, Rule, Subject};

        let action = Action::ToolCall {
            skill: "test_tools".to_string(),
            methods: vec!["probe_tool".to_string()],
        };
        let rules = vec![
            Rule {
                name: "agent-allow-probe".to_string(),
                subject: Subject::AgentOnly {
                    agent: "master".to_string(),
                    match_type: MatchType::Exact,
                },
                effect: Effect::Allow,
                actions: vec![action.clone()],
                template: None,
                priority: 10,
            },
            Rule {
                name: "user-allow-probe".to_string(),
                subject: Subject::UserAndAgent {
                    user_id: "u_1001".to_string(),
                    agent: "master".to_string(),
                    user_match: MatchType::Exact,
                    agent_match: MatchType::Exact,
                },
                effect: Effect::Allow,
                actions: vec![action],
                template: None,
                priority: 10,
            },
        ];
        closeclaw_permission::RuleSet {
            rules,
            ..Default::default()
        }
    }

    /// Run `probe_tool` through [`TraitObjectExecutor`] with the real
    /// permission check wiring for a session whose checkpoint sender is
    /// `sender` (`None` = checkpoint without sender_id).
    async fn run_probe(sender: Option<&str>) -> ToolResult {
        use closeclaw_permission::engine::engine_types as et;

        let sid = "sess-probe";
        let sm = Arc::new(sm_with_sender(sid, sender).await);
        let engine =
            closeclaw_permission::PermissionEngine::new_with_default_data_root(probe_ruleset());
        let perm = Arc::new(tokio::sync::RwLock::new(engine));
        let config_dir = tempfile::tempdir().expect("config dir");
        let cm = Arc::new(
            closeclaw_config::manager::ConfigManager::new(config_dir.path().to_path_buf())
                .expect("ConfigManager::new"),
        );
        let af = Arc::new(tokio::sync::Mutex::new(ApprovalFlow::new(
            Arc::new(ProbeLookup) as Arc<dyn SessionLookup>,
            Arc::new(|_| {}),
            Arc::new(|_: &str| {}),
            tokio::runtime::Handle::current(),
            HeartbeatApprovalMode::default(),
            config_dir.path().to_path_buf(),
            et::RuleSet::default(),
        )));
        let executor = TraitObjectExecutor::new(Arc::new(ProbeRegistry), tool_ctx(Some(sid)))
            .with_perm_deps((
                perm,
                sm,
                Arc::clone(&cm) as Arc<closeclaw_config::manager::ConfigManager>,
                af,
            ));
        let call = PendingToolCall {
            id: "call-1".to_string(),
            tool_name: "probe_tool".to_string(),
            args: serde_json::json!({}),
            file_path: None,
            is_concurrency_safe: true,
        };
        executor.execute(&call).await
    }

    /// ③ integration: the tool executes because the PermissionRequest
    /// user_id resolved to the checkpoint sender `u_1001` — under the
    /// old session-id wiring the User phase would miss and deny.
    #[tokio::test]
    async fn executor_tool_call_allowed_when_sender_matches_user_rule() {
        let result = run_probe(Some("u_1001")).await;
        let data = result.data.to_string();
        assert!(data.contains("PROBE_OK"), "expected tool run, got {data}");
    }

    /// ② integration: sender present but no User rule matches →
    /// user_defaults Deny vetoes the Agent Allow (intersection model).
    #[tokio::test]
    async fn executor_tool_call_denied_when_sender_has_no_user_rule() {
        let result = run_probe(Some("u_9999")).await;
        let data = result.data.to_string();
        assert!(
            data.contains("not permitted"),
            "expected intersection denial, got {data}"
        );
    }

    /// ④ integration: sender missing → empty user_id → engine empty-uid
    /// branch (User phase skipped, Agent Allow decides) → tool executes.
    #[tokio::test]
    async fn executor_tool_call_falls_back_to_agent_dimension_without_sender() {
        let result = run_probe(None).await;
        let data = result.data.to_string();
        assert!(
            data.contains("PROBE_OK"),
            "expected fallback allow, got {data}"
        );
    }
}
