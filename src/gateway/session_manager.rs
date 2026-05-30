//! SessionManager - extracted session management from Gateway
//!
//! Responsible for session lifecycle: lookup, creation, restoration.
//! On daemon shutdown, `flush_all()` serializes all active sessions to the persistence backend.

use crate::gateway::{DmScope, GatewayConfig, Message, Session};
use crate::im::processor::ProcessError;
use crate::im::IMAdapter;
use crate::llm::session::{ChatSession, ConversationSession};
use crate::session::bootstrap::loader::{load_bootstrap_files, BootstrapMode};
use crate::session::persistence::{
    PendingMessage, PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint,
    SessionStatus,
};
use crate::session::workspace;
use crate::skills::DiskSkillRegistry;
use crate::system_prompt::builder::{build_from_workspace, WorkspaceBuildConfig};
use crate::system_prompt::sections::invalidate_all_sections;
use crate::system_prompt::workdir::build_workdir_context;
use crate::tools::{ToolContext, ToolRegistry};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::warn;

/// SessionManager holds all session state previously belonging to Gateway.
/// It provides find_or_create to lookup or create a session by channel + message.
pub struct SessionManager {
    /// Active sessions: session_id -> Session
    sessions: RwLock<HashMap<String, Session>>,
    /// Persistence backend (for archived session restoration)
    storage: RwLock<Option<Arc<dyn PersistenceService>>>,
    /// DM scope policy (determines how session keys are computed)
    dm_scope: DmScope,
    /// IM adapters for sending notifications during restoration
    adapters: RwLock<HashMap<String, Arc<dyn IMAdapter>>>,
    /// Per-session ConversationSession for llm_busy and pending_messages management
    conversation_sessions: RwLock<HashMap<String, Arc<RwLock<ConversationSession>>>>,
    /// Workspace directory for bootstrap file loading (None means no workspace)
    workspace_dir: Option<PathBuf>,
    /// Bootstrap mode determining which files to load
    bootstrap_mode: BootstrapMode,
    /// Tool registry for building system prompt ToolsSection
    tool_registry: RwLock<Option<Arc<ToolRegistry>>>,
    /// Skill registry for building system prompt SkillListingSection
    skill_registry: RwLock<Option<Arc<std::sync::RwLock<Option<DiskSkillRegistry>>>>>,
    /// Default reasoning level for new sessions
    default_reasoning_level: ReasoningLevel,
}

impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager")
            .field("dm_scope", &self.dm_scope)
            .finish_non_exhaustive()
    }
}

impl SessionManager {
    /// Create a new SessionManager with the given config, optional storage,
    /// workspace directory and bootstrap mode.
    pub fn new(
        config: &GatewayConfig,
        storage: Option<Arc<dyn PersistenceService>>,
        workspace_dir: Option<PathBuf>,
        bootstrap_mode: BootstrapMode,
        default_reasoning_level: ReasoningLevel,
    ) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            storage: RwLock::new(storage),
            dm_scope: config.dm_scope,
            adapters: RwLock::new(HashMap::new()),
            conversation_sessions: RwLock::new(HashMap::new()),
            workspace_dir,
            bootstrap_mode,
            tool_registry: RwLock::new(None),
            skill_registry: RwLock::new(None),
            default_reasoning_level,
        }
    }

    /// Set the tool registry for building system prompt ToolsSection.
    pub async fn set_tool_registry(&self, registry: Arc<ToolRegistry>) {
        *self.tool_registry.write().await = Some(registry);
    }

    /// Set the skill registry for building system prompt SkillListingSection.
    pub async fn set_skill_registry(
        &self,
        registry: Arc<std::sync::RwLock<Option<DiskSkillRegistry>>>,
    ) {
        *self.skill_registry.write().await = Some(registry);
    }

    /// Get the current tool registry, if set.
    pub async fn get_tool_registry(&self) -> Option<Arc<ToolRegistry>> {
        self.tool_registry.read().await.clone()
    }

    /// Get the current skill registry, if set.
    pub async fn get_skill_registry(
        &self,
    ) -> Option<Arc<std::sync::RwLock<Option<DiskSkillRegistry>>>> {
        self.skill_registry.read().await.clone()
    }

    /// Register an IM adapter.
    pub async fn register_adapter(&self, name: String, adapter: Arc<dyn IMAdapter>) {
        let mut adapters = self.adapters.write().await;
        adapters.insert(name, adapter);
    }

    /// Set the persistence backend.
    pub async fn set_storage(&self, storage: Arc<dyn PersistenceService>) {
        *self.storage.write().await = Some(storage);
    }

    /// Compute session key from channel, message and optional account_id.
    fn compute_session_key(
        &self,
        channel: &str,
        message: &Message,
        account_id: Option<&str>,
    ) -> String {
        self.dm_scope
            .compute_session_key(channel, message, account_id)
    }

    /// Attempt to restore an archived session.
    /// Returns true iff restoration was attempted and succeeded.
    async fn try_restore_archived_session(&self, session_id: &str, channel: &str) -> bool {
        let storage = self.storage.read().await;
        let Some(storage) = storage.as_ref() else {
            return false;
        };
        let checkpoint = match storage.load_checkpoint(session_id).await {
            Ok(Some(cp)) => cp,
            Ok(None) | Err(_) => return false,
        };
        if checkpoint.status != SessionStatus::Archived {
            return false;
        }
        // Send restore notification
        let adapters = self.adapters.read().await;
        if let Some(adapter) = adapters.get(channel) {
            let notification = Message {
                id: format!("restore-{}", session_id),
                from: "system".to_string(),
                to: checkpoint
                    .chat_id
                    .as_deref()
                    .unwrap_or(session_id)
                    .to_string(),
                content: "正在恢复会话...".to_string(),
                channel: channel.to_string(),
                timestamp: chrono::Utc::now().timestamp(),
                metadata: std::collections::HashMap::new(),
            };
            if let Err(e) = adapter.send_message(&notification).await {
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

    /// Find or create a session for the given channel and message.
    ///
    /// 1. Compute session_id from channel + message + account_id
    /// 2. If session exists in active table → return it
    /// 3. If not, try to restore from archived storage
    /// 4. If restoration succeeds → return restored session
    /// 5. Otherwise → create and register a new session
    ///
    /// Returns the session_id string on success.
    pub async fn find_or_create(
        &self,
        channel: &str,
        message: &Message,
        account_id: Option<&str>,
    ) -> Result<String, ProcessError> {
        let session_id = self.compute_session_key(channel, message, account_id);
        // Fast path: session already exists
        {
            let sessions = self.sessions.read().await;
            if sessions.contains_key(&session_id) {
                return Ok(session_id);
            }
        }
        // Slow path: try archived session restoration
        let restored = self
            .try_restore_archived_session(&session_id, channel)
            .await;
        let mut sessions = self.sessions.write().await;
        if sessions.contains_key(&session_id) {
            return Ok(session_id);
        }
        // Build system prompt
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
        let agent_id = message.to.clone();
        let workdir_ctx = self
            .workspace_dir
            .as_ref()
            .map(|p| build_workdir_context(&p.to_string_lossy()));
        let tool_ctx = ToolContext {
            agent_id: agent_id.clone(),
            workdir: workdir_ctx,
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

        // Compute per-session workdir path
        let workdir_path = if restored {
            let checkpoint_agent_id = {
                let storage_guard = self.storage.read().await;
                match storage_guard.as_ref() {
                    Some(storage) => storage
                        .load_checkpoint(&session_id)
                        .await
                        .ok()
                        .flatten()
                        .and_then(|cp| cp.agent_id),
                    None => None,
                }
            };
            let aid = checkpoint_agent_id.as_deref().unwrap_or(&message.to);
            if let Some(ref wd) = self.workspace_dir {
                workspace::ensure_workspace_dir(wd, aid, aid)
                    .unwrap_or_else(|_| PathBuf::from("/tmp"))
            } else {
                PathBuf::from("/tmp")
            }
        } else if let Some(ref workspace_dir) = self.workspace_dir {
            workspace::ensure_workspace_dir(workspace_dir, &message.to, &message.from).map_err(
                |e| ProcessError::ProcessingFailed(format!("workspace creation failed: {}", e)),
            )?
        } else {
            PathBuf::from("/tmp")
        };

        let conv_session =
            ConversationSession::new(session_id.clone(), "default".to_string(), workdir_path)
                .with_system_prompt(prompt)
                .with_reasoning_level(self.default_reasoning_level);
        {
            let mut conv_sessions = self.conversation_sessions.write().await;
            conv_sessions.insert(session_id.clone(), Arc::new(RwLock::new(conv_session)));
        }

        if restored {
            // Reload checkpoint to obtain chat_id / agent_id and pending_messages
            let storage = self.storage.read().await;
            if let Some(storage) = storage.as_ref() {
                if let Ok(Some(cp)) = storage.load_checkpoint(&session_id).await {
                    // Restore pending_messages into ConversationSession
                    let conv_sessions = self.conversation_sessions.read().await;
                    if let Some(cs) = conv_sessions.get(&session_id) {
                        let mut cs = cs.write().await;
                        cs.restore_pending_messages(cp.pending_messages);
                    }
                    drop(conv_sessions);

                    sessions.insert(
                        session_id.clone(),
                        Session {
                            id: session_id.clone(),
                            agent_id: cp.chat_id.unwrap_or_else(|| message.to.clone()),
                            channel: channel.to_string(),
                            created_at: chrono::Utc::now().timestamp(),
                        },
                    );
                }
            }
        } else {
            // No archived session — create a brand-new Session
            sessions.insert(
                session_id.clone(),
                Session {
                    id: session_id.clone(),
                    agent_id: message.to.clone(),
                    channel: channel.to_string(),
                    created_at: chrono::Utc::now().timestamp(),
                },
            );
        }

        Ok(session_id)
    }

    /// Get active sessions for an agent.
    pub async fn get_agent_sessions(&self, agent_id: &str) -> Vec<Session> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .filter(|s| s.agent_id == agent_id)
            .cloned()
            .collect()
    }

    /// Check if a session with the given ID exists.
    pub async fn has_session(&self, session_id: &str) -> bool {
        let sessions = self.sessions.read().await;
        sessions.contains_key(session_id)
    }

    /// Get chat_id for a session.
    /// Returns the `agent_id` field of the session
    /// (which holds the chat_id per SessionManager convention).
    pub async fn get_chat_id(&self, session_id: &str) -> Option<String> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).map(|s| s.agent_id.clone())
    }

    /// Flush all active sessions to persistence.
    /// Returns the number of sessions successfully saved.
    pub async fn flush_all(&self) -> Result<usize, PersistenceError> {
        let storage = self.storage.read().await;
        let Some(storage) = storage.as_ref() else {
            return Ok(0);
        };
        let sessions = self.sessions.read().await;
        // Collect session ids first to avoid holding sessions lock across I/O
        let session_ids: Vec<String> = sessions.keys().cloned().collect();
        drop(sessions);

        // Collect pending messages using async RwLock read (no blocking_read)
        let conv_sessions = self.conversation_sessions.read().await;
        let mut pending_map: HashMap<String, Vec<PendingMessage>> = HashMap::new();
        for sid in &session_ids {
            if let Some(cs) = conv_sessions.get(sid) {
                let cs = cs.read().await;
                pending_map.insert(sid.clone(), cs.get_pending_messages());
            }
        }
        let sessions = self.sessions.read().await;
        let mut saved = 0;
        for (session_id, session) in sessions.iter() {
            let pending = pending_map.get(session_id).cloned().unwrap_or_default();
            let cp = SessionCheckpoint::new(session_id.clone())
                .with_status(SessionStatus::Active)
                .with_channel(session.channel.clone())
                .with_chat_id(session.agent_id.clone())
                .with_agent_id(session.agent_id.clone())
                .with_pending_messages(pending);
            if storage.save_checkpoint(&cp).await.is_ok() {
                saved += 1;
            } else {
                warn!(session_id = %session_id, "failed to save session checkpoint");
            }
        }
        Ok(saved)
    }

    /// Get the ConversationSession for a given session_id.
    /// Returns None if the session does not exist.
    pub async fn get_conversation_session(
        &self,
        session_id: &str,
    ) -> Option<Arc<RwLock<ConversationSession>>> {
        let conv_sessions = self.conversation_sessions.read().await;
        conv_sessions.get(session_id).cloned()
    }

    /// Check whether the LLM is busy for a given session.
    /// Returns false if the session does not exist.
    pub async fn is_session_busy(&self, session_id: &str) -> bool {
        let conv_sessions = self.conversation_sessions.read().await;
        match conv_sessions.get(session_id) {
            Some(cs) => {
                let cs = cs.read().await;
                cs.is_llm_busy()
            }
            None => false,
        }
    }

    /// Push a pending message onto the queue for a given session.
    /// Returns an error if the session does not exist.
    pub async fn push_pending_message(
        &self,
        session_id: &str,
        msg: PendingMessage,
    ) -> Result<(), String> {
        let conv_sessions = self.conversation_sessions.read().await;
        let cs = conv_sessions
            .get(session_id)
            .ok_or_else(|| format!("session not found: {}", session_id))?;
        let mut cs = cs.write().await;
        cs.push_pending(msg);
        Ok(())
    }

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

    /// Pop the oldest pending message for a given session.
    /// Returns None if the session does not exist or the queue is empty.
    pub async fn pop_pending_message(&self, session_id: &str) -> Option<PendingMessage> {
        let conv_sessions = self.conversation_sessions.read().await;
        let cs = conv_sessions.get(session_id)?;
        let mut cs = cs.write().await;
        cs.pop_pending()
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod flush_tests;
#[cfg(test)]
mod tests;
