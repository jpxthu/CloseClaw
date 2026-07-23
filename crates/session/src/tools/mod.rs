//! Session tools — tool implementations for session management.
//!
//! This module contains the 4 session tools (`sessions_spawn`, `sessions_steer`,
//! `sessions_kill`, `sessions_yield`) and the [`SessionManagerOps`] trait that
//! abstracts the session management operations they need.
//!
//! The trait is defined here (in the session crate) and implemented by
//! `SessionManager` in the gateway crate. This avoids a circular dependency
//! since gateway depends on session.

use async_trait::async_trait;
use std::sync::Arc;

use crate::spawn::{ChildSessionInfo, SpawnMode};
use closeclaw_config::agents::ResolvedAgentConfig;

pub mod prompt_template;
pub mod registrar;
pub mod sessions_kill;
pub mod sessions_spawn;
pub mod sessions_steer;
pub mod sessions_yield;

pub use prompt_template::PromptTemplate;
pub use registrar::SessionToolsRegistrar;
pub use sessions_kill::SessionsKillTool;
pub use sessions_spawn::SessionsSpawnTool;
pub use sessions_steer::SessionsSteerTool;
pub use sessions_yield::SessionsYieldTool;

/// Trait abstracting session management operations needed by session tools.
///
/// Implemented by `SessionManager` in the gateway crate. Session tools depend
/// on this trait (via `Arc<dyn SessionManagerOps>`) instead of directly on
/// `SessionManager`, avoiding a circular dependency between session and gateway.
#[async_trait]
pub trait SessionManagerOps: Send + Sync {
    /// Create a child session for the given parent.
    #[allow(clippy::too_many_arguments)]
    async fn create_child_session(
        &self,
        config: &ResolvedAgentConfig,
        parent_session_id: &str,
        depth: u32,
        task: &str,
        light_context: bool,
        workspace: Option<&str>,
        mode: SpawnMode,
        fork: bool,
        allowed_tools: Option<Vec<String>>,
        model_override: Option<&str>,
        parent_subagents_model: Option<&str>,
        max_spawn_depth: u32,
        spawn_timeout: Option<u64>,
        label: Option<&str>,
        prompt_template_prefix: Option<&str>,
    ) -> Result<String, String>;

    /// Validate that a child session is owned by the given parent.
    async fn validate_child_ownership(
        &self,
        parent_id: &str,
        child_id: &str,
    ) -> Option<ChildSessionInfo>;

    /// Inject a new task into a persistent child session's pending queue.
    async fn steer_child(&self, child_id: &str, task: &str) -> Result<(), String>;

    /// Force-terminate a child session and all its descendants.
    async fn kill_child(&self, parent_id: &str, child_id: &str) -> Result<(), String>;

    /// Get the chat ID (agent ID) for a session.
    async fn get_chat_id(&self, session_id: &str) -> Option<String>;

    /// Get the depth of a session in the spawn tree.
    async fn get_session_depth(&self, session_id: &str) -> Option<u32>;

    /// Start a yield timeout for the given session.
    ///
    /// Takes an `Arc<Self>` so the implementation can spawn a background task
    /// that holds a strong reference.
    async fn start_yield_timeout(
        self: Arc<Self>,
        session_id: &str,
        agent_id: &str,
        timeout_secs: Option<u64>,
    );
}
