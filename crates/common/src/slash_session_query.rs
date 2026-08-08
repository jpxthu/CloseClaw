//! Trait for session queries used by slash command handlers.
//!
//! Abstracts the [`SessionManager`] (in the gateway crate) so that slash
//! handlers can query session state without depending on the gateway crate
//! directly. This breaks the slash → gateway dependency while preserving
//! the ability to test handlers with a mock implementation.

use std::path::PathBuf;

use async_trait::async_trait;

use crate::session_lookup::PendingMessage;
use crate::PlanState;

/// Session query interface for slash command handlers.
///
/// Implemented by [`closeclaw_gateway::SessionManager`] in the gateway crate;
/// slash handlers depend only on this trait (defined in common).
#[async_trait]
pub trait SlashSessionQuery: Send + Sync {
    // ── Plan state ─────────────────────────────────────────────────────

    /// Get the plan state for a session.
    async fn get_plan_state(&self, session_id: &str) -> Option<PlanState>;

    /// Update the plan state for a session.
    async fn set_plan_state(&self, session_id: &str, plan_state: PlanState);

    // ── Pending messages ───────────────────────────────────────────────

    /// Push a pending message onto a session's queue.
    async fn push_pending_message(
        &self,
        session_id: &str,
        msg: PendingMessage,
    ) -> Result<(), String>;

    // ── Session lifecycle ──────────────────────────────────────────────

    /// Trigger manual background execution for a session.
    ///
    /// Returns `Ok(true)` if the signal was fired.
    async fn trigger_manual_background(&self, session_id: &str) -> Result<bool, String>;

    /// Set the active workflow run for a session and persist the checkpoint.
    ///
    /// The `run` parameter is type-erased as `Box<dyn Any + Send + Sync>`
    /// to avoid a dependency on the workflow crate from common.
    /// Implementations should downcast to `closeclaw_workflow::run::WorkflowRun`.
    async fn set_workflow_run(
        &self,
        session_id: &str,
        run: Option<Box<dyn std::any::Any + Send + Sync>>,
    ) -> Result<(), String>;

    // ── System prompt ──────────────────────────────────────────────────

    /// Invalidate the static-layer system prompt cache.
    async fn invalidate_static_cache(&self);

    /// Rebuild the system prompt for a session.
    async fn rebuild_system_prompt_for_session(&self, session_id: &str);

    /// Add a system append to a session.
    async fn add_system_append(&self, session_id: &str, content: String);

    // ── Session state queries (return primitive types, no session dep) ─

    /// Get the model name for a session.
    async fn get_model(&self, session_id: &str) -> Option<String>;

    /// Get the effective reasoning level name for a session.
    async fn get_reasoning_level(&self, session_id: &str) -> Option<String>;

    /// Get the verbosity level name for a session.
    async fn get_verbosity_level(&self, session_id: &str) -> Option<String>;

    /// Get the session mode name for a session.
    async fn get_session_mode(&self, session_id: &str) -> Option<String>;

    /// Get the workdir for a session.
    async fn get_workdir(&self, session_id: &str) -> Option<PathBuf>;

    /// Get the system appends for a session.
    async fn get_system_appends(&self, session_id: &str) -> Vec<String>;

    /// Set the workdir for a session.
    async fn set_workdir(&self, session_id: &str, path: PathBuf);

    /// Get the LLM busy state for a session.
    async fn is_llm_busy(&self, session_id: &str) -> bool;

    /// Get stats for a session (total_tokens, prompt_tokens, cache_read, cache_write).
    async fn get_stats(&self, session_id: &str) -> Option<(usize, usize, usize, usize)>;

    /// Get the last cache break notification for a session.
    async fn get_last_cache_break(&self, session_id: &str) -> Option<String>;

    /// Get the count of active child handles for a session.
    async fn get_active_child_count(&self, session_id: &str) -> usize;
}
