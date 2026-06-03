//! System prompt rebuild logic for `SessionManager`.
//!
//! Extracted from `session_manager.rs` to keep the file under the
//! 500-line hard limit.

use crate::session::bootstrap::loader::load_bootstrap_files;
use crate::system_prompt::builder::{build_from_workspace, WorkspaceBuildConfig};
use crate::system_prompt::sections::invalidate_all_sections;
use crate::system_prompt::workdir::build_workdir_context;
use crate::tools::ToolContext;

use super::SessionManager;

impl SessionManager {
    /// Rebuild the system prompt for an existing session.
    /// Called after compaction to pick up skill/config changes.
    /// Lock Safety: acquires its own write lock; callers must NOT hold
    /// any external write guard on the same session.
    pub async fn rebuild_system_prompt(&self, session_id: &str) {
        let cs = match self.get_conversation_session(session_id).await {
            Some(cs) => cs,
            None => return,
        };
        let agent_id = {
            let sessions = self.sessions.read().await;
            match sessions.get(session_id) {
                Some(session) => session.agent_id.clone(),
                None => return,
            }
        };
        invalidate_all_sections();
        let bootstrap_files = if let Some(ref workspace) = self.workspace_dir {
            load_bootstrap_files(workspace, self.bootstrap_mode)
                .unwrap_or_default()
                .into_iter()
                .collect()
        } else {
            vec![]
        };
        let tool_registry_guard = self.tool_registry.read().await;
        let tool_registry_ref = tool_registry_guard.as_ref().map(|r| r.as_ref());
        let skill_registry = self.skill_registry.read().await.clone();
        let session_workdir = {
            let cs_read = cs.read().await;
            cs_read.workdir().to_path_buf()
        };
        let tool_ctx = ToolContext {
            agent_id: agent_id.clone(),
            workdir: Some(build_workdir_context(&session_workdir.to_string_lossy())),
            session_id: None,
            call_id: None,
            session: None,
        };
        let workspace_root = self.workspace_dir.clone().unwrap_or_default();
        let prompt = build_from_workspace(
            &workspace_root,
            WorkspaceBuildConfig {
                bootstrap_files,
                tool_registry: tool_registry_ref,
                tool_ctx: &tool_ctx,
                skill_registry,
                agent_id: Some(&agent_id),
                dynamic_sections: vec![],
                append_section: None,
            },
        )
        .await;

        let mut cs = cs.write().await;
        cs.replace_system_prompt(prompt);
    }
}
