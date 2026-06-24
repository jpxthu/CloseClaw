//! Helper functions extracted from SessionManager::find_or_create
//! to keep the main file under the 500-line limit.

use crate::agent::registry::AgentRegistry;
use crate::gateway::Message;
use crate::im::IMAdapter;
use crate::session::bootstrap::loader::{load_bootstrap_files, BootstrapMode};
use crate::session::persistence::{PersistenceService, SessionStatus};
use crate::session::workspace;
use crate::skills::DiskSkillRegistry;
use crate::system_prompt::builder::{build_from_workspace, WorkspaceBuildConfig};
use crate::system_prompt::workdir::build_workdir_context;
use crate::tools::{ToolContext, ToolRegistry};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::warn;
use uuid::Uuid;

/// Encapsulates agent-level tool and skill filter configuration.
#[derive(Debug, Clone, Default)]
pub(super) struct AgentToolSkillConfig {
    pub agent_tools: Option<Vec<String>>,
    pub agent_disallowed_tools: Option<Vec<String>>,
    pub agent_skills: Option<Vec<String>>,
}

/// Generate a unique session ID.
///
/// Format: `{agent_id}_{timestamp_ms}_{8-hex}`
pub(super) fn generate_session_id(agent_id: &str) -> String {
    let ts = chrono::Utc::now().timestamp_millis();
    let uuid = Uuid::new_v4();
    let hex_part = format!("{:08x}", uuid.as_fields().0);
    format!("{}_{}_{}", agent_id, ts, hex_part)
}

/// Build the system prompt for a new session.
pub(super) async fn build_session_system_prompt(
    workspace_dir: &Option<PathBuf>,
    bootstrap_mode: BootstrapMode,
    tool_registry: &Option<Arc<ToolRegistry>>,
    skill_registry: Option<Arc<std::sync::RwLock<Option<DiskSkillRegistry>>>>,
    agent_id: &str,
    filters: &AgentToolSkillConfig,
    agent_registry: Option<Arc<AgentRegistry>>,
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
            agent_tools: filters.agent_tools.clone(),
            agent_disallowed_tools: filters.agent_disallowed_tools.clone(),
            agent_skills: filters.agent_skills.clone(),
            dynamic_sections: vec![],
            append_section: None,
            agent_registry,
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
) -> Result<PathBuf, crate::processor_chain::error::ProcessError> {
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
            crate::processor_chain::error::ProcessError::ChainFailed(format!(
                "workspace creation failed: {}",
                e
            ))
        })
    } else {
        Ok(PathBuf::from("/tmp"))
    }
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

/// Update the thread_id in a session's checkpoint.
///
/// Loads the checkpoint from storage, overwrites `thread_id` with the
/// provided value, and saves it back.
///
/// Silently skips (with warn!) when:
/// - storage is not available
/// - checkpoint does not exist
pub(super) async fn update_checkpoint_thread_id(
    storage: &Arc<dyn PersistenceService>,
    session_id: &str,
    thread_id: &Option<String>,
) {
    let mut cp = match storage.load_checkpoint(session_id).await {
        Ok(Some(cp)) => cp,
        Ok(None) | Err(_) => {
            warn!(
                session_id = %session_id,
                "checkpoint not found, skipping thread_id update"
            );
            return;
        }
    };
    cp.thread_id = thread_id.clone();
    if let Err(e) = storage.save_checkpoint(&cp).await {
        warn!(
            session_id = %session_id,
            error = %e,
            "failed to save checkpoint with updated thread_id"
        );
    }
}

/// Attempt to restore an archived session.
///
/// Returns `true` if restoration was attempted and succeeded.
pub(super) async fn try_restore_archived_session_inner(
    storage: &Arc<dyn PersistenceService>,
    adapters: &HashMap<String, Arc<dyn IMAdapter>>,
    session_id: &str,
    channel: &str,
) -> bool {
    let checkpoint = match storage.load_checkpoint(session_id).await {
        Ok(Some(cp)) => cp,
        Ok(None) | Err(_) => return false,
    };
    if checkpoint.status != SessionStatus::Archived {
        return false;
    }
    // Send restore notification
    if let Some(adapter) = adapters.get(channel) {
        let notification = Message {
            id: format!("restore-{}", session_id),
            from: "system".to_string(),
            to: checkpoint
                .peer_id
                .as_deref()
                .unwrap_or(session_id)
                .to_string(),
            content: "正在恢复会话...".to_string(),
            channel: channel.to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            metadata: std::collections::HashMap::new(),
            thread_id: None,
        };
        if let Err(e) = adapter.send_message(&notification, None).await {
            warn!(session_id = %session_id, error = %e,
                "failed to send restore notification");
        }
    }
    if let Err(e) = storage.restore_checkpoint(session_id).await {
        warn!(session_id = %session_id, error = %e,
            "failed to restore archived session");
        return false;
    }
    true
}
