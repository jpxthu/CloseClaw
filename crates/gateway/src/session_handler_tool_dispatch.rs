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

#[async_trait::async_trait]
impl ToolExecutor for TraitObjectExecutor {
    async fn execute(&self, call: &PendingToolCall) -> ToolResult {
        let mut ctx = self.base_ctx.clone();
        ctx.call_id = Some(call.id.clone());

        // --- Permission check (Level 1: ToolCall) ---
        if let Some(ref perm_deps) = self.perm_deps {
            let (perm_engine, _session_mgr, _config_mgr, _approval_flow) = perm_deps;
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
            let user_id = ctx.session_id.clone().unwrap_or_default();

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
