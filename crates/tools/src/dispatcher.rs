//! Multi-tool parallel dispatcher.
//!
//! [`ToolCallDispatcher`] routes a batch of tool calls to parallel, mutex-by-file,
//! or serial execution based on [`ToolFlags::is_concurrency_safe`] and per-file
//! mutex ownership.
//!
//! Core types and dispatch logic are defined in [`closeclaw_common::dispatcher`].
//! This module re-exports them and adds the tools-specific
//! [`ToolRegistryExecutor`] and [`build_pending_call`] helper.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::debug_log::{emit_tool_event, ToolsDebugLogContext, ToolsEmitEventParams};
use crate::media_ref::{resolve_media_refs, MediaRefError};
use crate::permission_check::{
    check_command_permission, check_config_write_permission, check_file_op_permission,
    check_tool_permission, CommandPermissionResult, PermDeps,
};
use crate::registry::ToolRegistryImpl;

// Re-export types from common
pub use closeclaw_common::dispatcher::{
    extract_file_path, DispatchGroup, PendingToolCall, ToolCallDispatcher, ToolExecutor,
};
pub use closeclaw_common::file_mutex::FileMutexMap;

// ---------------------------------------------------------------------------
// PendingToolCall construction helpers
// ---------------------------------------------------------------------------

/// Build a [`PendingToolCall`] from LLM tool call components.
///
/// Looks up the tool in `registry` to read its `is_concurrency_safe`
/// flag. Extracts `file_path` from `args` via [`extract_file_path`].
pub async fn build_pending_call(
    id: String,
    tool_name: &str,
    args: Value,
    registry: &dyn closeclaw_common::ToolRegistryQuery,
) -> PendingToolCall {
    let is_concurrency_safe = registry
        .get_tool_concurrency_safe(tool_name)
        .await
        .unwrap_or(false);
    let file_path = extract_file_path(&args);

    PendingToolCall {
        id,
        tool_name: tool_name.to_string(),
        args,
        file_path,
        is_concurrency_safe,
    }
}

// ===========================================================================
// Runtime executor — bridges dispatcher to Tool::call()
// ===========================================================================

/// Runtime [`ToolExecutor`] that looks up tools from a
/// [`ToolRegistryImpl`] and calls [`Tool::call`] with the
/// arguments carried in [`PendingToolCall`].
///
/// Created per-dispatch batch; holds a base [`ToolContext`] whose
/// `call_id` is overridden from each [`PendingToolCall::id`].
///
/// Optionally holds a [`DebugLog`] context for emitting structured
/// debug-log events at tool execution start/end and full params.
pub struct ToolRegistryExecutor {
    registry: Arc<ToolRegistryImpl>,
    base_ctx: crate::ToolContext,
    perm_deps: Option<PermDeps>,
    debug_log: Option<Arc<closeclaw_debug_log::DebugLog>>,
    trace_id: String,
    session_key: Option<String>,
    parent_span: Option<closeclaw_debug_log::TraceContext>,
    /// Optional media store for resolving `[type: key]` references in tool args.
    media_store: Option<Arc<dyn closeclaw_common::MediaStoreAccess>>,
}

impl ToolRegistryExecutor {
    /// Create a new executor.
    ///
    /// `base_ctx` is cloned for each call; its `call_id` is replaced
    /// with the individual `PendingToolCall::id`.
    pub fn new(registry: Arc<ToolRegistryImpl>, base_ctx: crate::ToolContext) -> Self {
        let media_store = base_ctx.media_store.clone();
        Self {
            registry,
            base_ctx,
            perm_deps: None,
            debug_log: None,
            trace_id: String::new(),
            session_key: None,
            parent_span: None,
            media_store,
        }
    }

    /// Inject permission dependencies for centralized permission checks.
    pub fn with_perm_deps(mut self, perm_deps: PermDeps) -> Self {
        self.perm_deps = Some(perm_deps);
        self
    }

    /// Attach a media store for resolving `[type: key]` references in tool args.
    pub fn with_media_store(
        mut self,
        media_store: Option<Arc<dyn closeclaw_common::MediaStoreAccess>>,
    ) -> Self {
        self.media_store = media_store;
        self
    }

    /// Attach a debug-log context for emitting structured events.
    pub fn with_debug_log(
        mut self,
        debug_log: Option<Arc<closeclaw_debug_log::DebugLog>>,
        trace_id: String,
        session_key: Option<String>,
        parent_span: Option<closeclaw_debug_log::TraceContext>,
    ) -> Self {
        self.debug_log = debug_log;
        self.trace_id = trace_id;
        self.session_key = session_key;
        self.parent_span = parent_span;
        self
    }
}

#[async_trait]
impl ToolExecutor for ToolRegistryExecutor {
    async fn execute(&self, call: &PendingToolCall) -> closeclaw_common::tool_trait::ToolResult {
        let guard = self.registry.tools.read().await;
        let tool = guard
            .get(&call.tool_name)
            .expect("tool should have been validated before dispatch");
        let tool = Arc::clone(tool);
        drop(guard);

        // Resolve media references in tool args before execution.
        let args = if let Some(ref store) = self.media_store {
            match resolve_media_refs(&call.args, store.as_ref()) {
                Ok(resolved) => resolved,
                Err(MediaRefError::NotFound { media_type, key }) => {
                    return closeclaw_common::tool_trait::ToolResult {
                        data: serde_json::json!({
                            "error": format!("media reference not found: [{media_type}: {key}]")
                        }),
                        new_messages: vec![],
                        context_modifier: None,
                    };
                }
                Err(MediaRefError::StoreUnavailable) => call.args.clone(),
                Err(MediaRefError::Store(e)) => {
                    return closeclaw_common::tool_trait::ToolResult {
                        data: serde_json::json!({ "error": format!("media store error: {e}") }),
                        new_messages: vec![],
                        context_modifier: None,
                    };
                }
            }
        } else {
            call.args.clone()
        };

        let mut ctx = self.base_ctx.clone();
        ctx.call_id = Some(call.id.clone());

        // --- Centralized permission check (Level 1: ToolCall) ---
        let tool_group = tool.group().to_string();
        if tool_group != "workflow" {
            if let Some(ref perm_deps) = self.perm_deps {
                let debug_ctx = ToolsDebugLogContext {
                    debug_log: self.debug_log.as_deref(),
                    trace_id: &self.trace_id,
                    session_key: self.session_key.as_deref(),
                };
                match check_tool_permission(perm_deps, &ctx, &tool_group, "call", Some(debug_ctx))
                    .await
                {
                    Ok(Some(denied)) => return denied,
                    Ok(None) => {}
                    Err(e) => {
                        return closeclaw_common::tool_trait::ToolResult {
                            data: serde_json::json!({ "error": e.to_string() }),
                            new_messages: vec![],
                            context_modifier: None,
                        };
                    }
                }

                // --- Level 2: domain-specific permission checks ---
                let debug_ctx2 = ToolsDebugLogContext {
                    debug_log: self.debug_log.as_deref(),
                    trace_id: &self.trace_id,
                    session_key: self.session_key.as_deref(),
                };
                match tool_group.as_str() {
                    "bash" => {
                        if let Some(full_cmd) =
                            args.get("command").and_then(serde_json::Value::as_str)
                        {
                            let parts: Vec<&str> = full_cmd.split_whitespace().collect();
                            let base_cmd = parts.first().copied().unwrap_or("");
                            let cmd_args: Vec<String> =
                                parts[1..].iter().map(|s| s.to_string()).collect();
                            match check_command_permission(
                                perm_deps,
                                &ctx,
                                base_cmd,
                                &cmd_args,
                                Some(debug_ctx2),
                            )
                            .await
                            {
                                CommandPermissionResult::Permitted => {}
                                CommandPermissionResult::PendingApproval(result) => return result,
                                CommandPermissionResult::Denied(reason) => {
                                    return closeclaw_common::tool_trait::ToolResult {
                                        data: serde_json::json!({
                                            "error": format!(
                                                "command permission denied: {reason}"
                                            )
                                        }),
                                        new_messages: vec![],
                                        context_modifier: None,
                                    };
                                }
                            }
                        }
                    }
                    "file_ops" => {
                        if let Some(path) = args.get("path").and_then(serde_json::Value::as_str) {
                            let op = if tool.flags().is_read_only {
                                "read"
                            } else {
                                "write"
                            };
                            match check_file_op_permission(
                                perm_deps,
                                &ctx,
                                path,
                                op,
                                Some(debug_ctx2),
                            )
                            .await
                            {
                                Ok(Some(denied)) => return denied,
                                Ok(None) => {}
                                Err(e) => {
                                    return closeclaw_common::tool_trait::ToolResult {
                                        data: serde_json::json!({ "error": e.to_string() }),
                                        new_messages: vec![],
                                        context_modifier: None,
                                    };
                                }
                            }
                            if op == "write" {
                                let config_manager = &perm_deps.2;
                                let data_root = config_manager.config_dir();
                                if closeclaw_permission::is_config_file_path(data_root, path) {
                                    match check_config_write_permission(perm_deps, &ctx, path).await
                                    {
                                        Ok(Some(denied)) => return denied,
                                        Ok(None) => {}
                                        Err(e) => {
                                            return closeclaw_common::tool_trait::ToolResult {
                                                data: serde_json::json!({ "error": e.to_string() }),
                                                new_messages: vec![],
                                                context_modifier: None,
                                            };
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Emit tool.execution.start
        emit_tool_event(ToolsEmitEventParams {
            ctx: ToolsDebugLogContext {
                debug_log: self.debug_log.as_deref(),
                trace_id: &self.trace_id,
                session_key: self.session_key.as_deref(),
            },
            level: closeclaw_debug_log::LogLevel::Info,
            source_module: "tools",
            event_type: "tool.execution.start",
            payload: serde_json::json!({
                "tool_name": call.tool_name,
                "call_id": call.id,
            }),
            parent: self.parent_span.as_ref(),
        });

        let result = match tool.call(args.clone(), &ctx).await {
            Ok(result) => result,
            Err(e) => closeclaw_common::tool_trait::ToolResult {
                data: serde_json::json!({ "error": e.to_string() }),
                new_messages: vec![],
                context_modifier: None,
            },
        };

        // Emit tool.params
        emit_tool_event(ToolsEmitEventParams {
            ctx: ToolsDebugLogContext {
                debug_log: self.debug_log.as_deref(),
                trace_id: &self.trace_id,
                session_key: self.session_key.as_deref(),
            },
            level: closeclaw_debug_log::LogLevel::Debug,
            source_module: "tools",
            event_type: "tool.params",
            payload: serde_json::json!({
                "tool_name": call.tool_name,
                "call_id": call.id,
                "params": call.args,
                "result": result.data,
            }),
            parent: self.parent_span.as_ref(),
        });

        // Emit tool.execution.end
        let has_error = result.data.get("error").is_some();
        emit_tool_event(ToolsEmitEventParams {
            ctx: ToolsDebugLogContext {
                debug_log: self.debug_log.as_deref(),
                trace_id: &self.trace_id,
                session_key: self.session_key.as_deref(),
            },
            level: if has_error {
                closeclaw_debug_log::LogLevel::Warn
            } else {
                closeclaw_debug_log::LogLevel::Info
            },
            source_module: "tools",
            event_type: "tool.execution.end",
            payload: serde_json::json!({
                "tool_name": call.tool_name,
                "call_id": call.id,
                "has_error": has_error,
            }),
            parent: self.parent_span.as_ref(),
        });

        result
    }
}

#[cfg(test)]
#[path = "dispatcher_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "dispatcher_level2_tests.rs"]
mod level2_tests;
