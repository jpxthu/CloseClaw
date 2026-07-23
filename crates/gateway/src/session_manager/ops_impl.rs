//! [`SessionManagerOps`] implementation for [`SessionManager`].
//!
//! Bridges the session crate's trait abstraction so that session tools
//! can operate on `SessionManager` via `Arc<dyn SessionManagerOps>`
//! without a circular dependency between session and gateway.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_session::spawn::{ChildSessionInfo, SpawnMode};
use closeclaw_session::tools::SessionManagerOps;

use super::SessionManager;

#[async_trait]
impl SessionManagerOps for SessionManager {
    async fn create_child_session(
        &self,
        config: &closeclaw_config::agents::ResolvedAgentConfig,
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
    ) -> Result<String, String> {
        self.create_child_session(
            config,
            parent_session_id,
            depth,
            task,
            light_context,
            workspace,
            mode,
            fork,
            allowed_tools,
            model_override,
            parent_subagents_model,
            max_spawn_depth,
            spawn_timeout,
            label,
            prompt_template_prefix,
        )
        .await
    }

    async fn validate_child_ownership(
        &self,
        parent_id: &str,
        child_id: &str,
    ) -> Option<ChildSessionInfo> {
        self.validate_child_ownership(parent_id, child_id).await
    }

    async fn steer_child(&self, child_id: &str, task: &str) -> Result<(), String> {
        self.steer_child(child_id, task).await
    }

    async fn kill_child(&self, parent_id: &str, child_id: &str) -> Result<(), String> {
        self.kill_child(parent_id, child_id).await
    }

    async fn get_chat_id(&self, session_id: &str) -> Option<String> {
        self.get_chat_id(session_id).await
    }

    async fn get_session_depth(&self, session_id: &str) -> Option<u32> {
        self.get_session_depth(session_id).await
    }

    async fn start_yield_timeout(
        self: Arc<Self>,
        session_id: &str,
        agent_id: &str,
        timeout_secs: Option<u64>,
    ) {
        SessionManager::start_yield_timeout(&self, session_id, agent_id, timeout_secs).await
    }
}
