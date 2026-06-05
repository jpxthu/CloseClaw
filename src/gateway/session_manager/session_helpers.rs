//! Helper functions extracted from SessionManager::find_or_create
//! to keep the main file under the 500-line limit.

use crate::gateway::Message;
use crate::llm::session::ConversationSession;
use crate::session::bootstrap::loader::{load_bootstrap_files, BootstrapMode};
use crate::session::workspace;
use crate::skills::DiskSkillRegistry;
use crate::system_prompt::builder::{build_from_workspace, WorkspaceBuildConfig};
use crate::system_prompt::workdir::build_workdir_context;
use crate::tools::{ToolContext, ToolRegistry};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::warn;

/// Build the system prompt for a new session.
pub(super) async fn build_session_system_prompt(
    workspace_dir: &Option<PathBuf>,
    bootstrap_mode: BootstrapMode,
    tool_registry: &Option<Arc<ToolRegistry>>,
    skill_registry: Option<Arc<std::sync::RwLock<Option<DiskSkillRegistry>>>>,
    agent_id: &str,
) -> String {
    let bootstrap_files = if let Some(ref workspace) = workspace_dir {
        load_bootstrap_files(workspace, bootstrap_mode)
            .unwrap_or_default()
            .into_iter()
            .collect()
    } else {
        vec![]
    };
    let tool_registry_ref = tool_registry.as_ref().map(|r| r.as_ref());
    let workdir_ctx = workspace_dir
        .as_ref()
        .map(|p| build_workdir_context(&p.to_string_lossy()));
    let tool_ctx = ToolContext {
        agent_id: agent_id.to_string(),
        workdir: workdir_ctx,
        session_id: None,
        call_id: None,
        session: None,
    };
    let workspace_root = workspace_dir.clone().unwrap_or_default();
    build_from_workspace(
        &workspace_root,
        WorkspaceBuildConfig {
            bootstrap_files,
            tool_registry: tool_registry_ref,
            tool_ctx: &tool_ctx,
            skill_registry,
            agent_id: Some(agent_id),
            dynamic_sections: vec![],
            append_section: None,
        },
    )
    .await
}

/// Compute the workdir path for a new session.
pub(super) async fn compute_session_workdir(
    restored: bool,
    session_id: &str,
    message: &Message,
    workspace_dir: &Option<PathBuf>,
    storage: &Arc<dyn crate::session::persistence::PersistenceService>,
) -> Result<PathBuf, crate::im::processor::ProcessError> {
    if restored {
        let checkpoint_agent_id = {
            match storage
                .load_checkpoint(session_id)
                .await
                .ok()
                .flatten()
                .and_then(|cp| cp.agent_id)
            {
                Some(aid) => aid,
                None => message.to.clone(),
            }
        };
        let aid = &checkpoint_agent_id;
        if let Some(ref wd) = workspace_dir {
            Ok(workspace::ensure_workspace_dir(wd, aid, aid)
                .unwrap_or_else(|_| PathBuf::from("/tmp")))
        } else {
            Ok(PathBuf::from("/tmp"))
        }
    } else if let Some(ref workspace_dir) = workspace_dir {
        workspace::ensure_workspace_dir(workspace_dir, &message.to, &message.from).map_err(|e| {
            crate::im::processor::ProcessError::ProcessingFailed(format!(
                "workspace creation failed: {}",
                e
            ))
        })
    } else {
        Ok(PathBuf::from("/tmp"))
    }
}

/// Restore a session from an archived checkpoint.
pub(super) async fn restore_checkpoint_session(
    storage: &Arc<dyn crate::session::persistence::PersistenceService>,
    session_id: &str,
    channel: &str,
    message: &Message,
    conv_sessions: &RwLock<std::collections::HashMap<String, Arc<RwLock<ConversationSession>>>>,
) -> Option<crate::gateway::Session> {
    let mut cp = storage.load_checkpoint(session_id).await.ok().flatten()?;

    // Restore pending_messages and system_appends into ConversationSession
    {
        let sessions = conv_sessions.read().await;
        if let Some(cs) = sessions.get(session_id) {
            let mut cs = cs.write().await;
            cs.restore_pending_messages(cp.pending_messages.clone());
            cs.restore_system_appends(cp.system_appends.clone());
        }
    }

    let session = crate::gateway::Session {
        id: session_id.to_string(),
        agent_id: cp.chat_id.clone().unwrap_or_else(|| message.to.clone()),
        channel: channel.to_string(),
        created_at: chrono::Utc::now().timestamp(),
        depth: 0,
    };

    // 覆盖式写入: 每次入站都用 message.thread_id 覆盖 checkpoint.thread_id
    cp.thread_id = message.thread_id.clone();
    if let Err(e) = storage.save_checkpoint(&cp).await {
        warn!(session_id = %session_id, error = %e,
            "failed to save checkpoint with updated thread_id");
    }

    Some(session)
}

/// Create and persist a brand-new session.
pub(super) fn create_new_session(
    session_id: &str,
    message: &Message,
    channel: &str,
) -> crate::gateway::Session {
    crate::gateway::Session {
        id: session_id.to_string(),
        agent_id: message.to.clone(),
        channel: channel.to_string(),
        created_at: chrono::Utc::now().timestamp(),
        depth: 0,
    }
}
