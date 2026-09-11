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

/// A minimal [`ToolExecutor`] that delegates to [`ToolRegistryQuery::call_tool`].
///
/// Does not perform permission checks or debug logging — those are
/// handled by the caller or a higher-level executor when available.
struct TraitObjectExecutor {
    registry: Arc<dyn ToolRegistryQuery>,
    base_ctx: ToolContext,
}

impl TraitObjectExecutor {
    fn new(registry: Arc<dyn ToolRegistryQuery>, base_ctx: ToolContext) -> Self {
        Self { registry, base_ctx }
    }
}

#[async_trait::async_trait]
impl ToolExecutor for TraitObjectExecutor {
    async fn execute(&self, call: &PendingToolCall) -> ToolResult {
        let mut ctx = self.base_ctx.clone();
        ctx.call_id = Some(call.id.clone());
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

        // 5. Check provider supports parallel tool calls.
        let provider_supports_parallel = {
            let cs = session_manager.get_conversation_session(session_id).await;
            if let Some(cs) = cs {
                let cs_read = cs.read().await;
                cs_read
                    .llm_caller()
                    .map(|c| c.supports_parallel_tool_calls())
                    .unwrap_or(true)
            } else {
                true
            }
        };

        // 6. Create dispatcher.
        let dispatcher = ToolCallDispatcher::new(
            Arc::clone(file_mutex_map),
            is_parallel_enabled && provider_supports_parallel,
        );

        // 7. Construct ToolContext.
        let base_ctx = if let Some(cs) = session_manager.get_conversation_session(session_id).await
        {
            let cs_read = cs.read().await;
            let workdir = cs_read.workdir().to_path_buf();
            let workdir_ctx = closeclaw_common::WorkdirContext {
                path: workdir.to_string_lossy().to_string(),
                has_git: false,
                branch: None,
                recent_changes: 0,
            };
            ToolContext {
                agent_id: agent_id.unwrap_or_default(),
                workdir: Some(workdir_ctx),
                session_id: Some(session_id.to_string()),
                call_id: None,
                session: None,
                session_mode: None,
                manual_background_signal: None,
                media_store: None,
            }
        } else {
            ToolContext {
                agent_id: agent_id.unwrap_or_default(),
                workdir: None,
                session_id: Some(session_id.to_string()),
                call_id: None,
                session: None,
                session_mode: None,
                manual_background_signal: None,
                media_store: None,
            }
        };

        // 8. Execute.
        let executor = TraitObjectExecutor::new(Arc::clone(&registry), base_ctx);
        let results = dispatcher.dispatch_all(calls, &executor).await;

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
