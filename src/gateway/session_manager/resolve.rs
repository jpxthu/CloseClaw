//! Session key resolution: the unified entry point for mapping
//! session_key → session_id, with three lookup paths:
//! 1. key_registry hit + active session → return directly
//! 2. key_registry hit + archived session → restore → return
//! 3. key_registry miss → create new session → register → return

use super::session_helpers;
use super::session_helpers::AgentToolSkillConfig;
use super::SessionManager;
use crate::config::agents::ResolvedAgentConfig;
use crate::gateway::Message;
use crate::im::processor::ProcessError;
use crate::llm::session::ConversationSession;
use crate::session::persistence::{SessionCheckpoint, SessionStatus};
use crate::session::workspace;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::warn;

impl SessionManager {
    /// Extract tool/skill filter configuration from an agent config.
    pub(super) fn extract_agent_filters(config: &ResolvedAgentConfig) -> AgentToolSkillConfig {
        let agent_tools = if config.tools.is_empty() || config.tools == ["*"] {
            None
        } else {
            Some(config.tools.clone())
        };
        let agent_disallowed_tools = if config.disallowed_tools.is_empty() {
            None
        } else {
            Some(config.disallowed_tools.clone())
        };
        let agent_skills = if config.skills.is_empty() || config.skills == ["*"] {
            None
        } else {
            Some(config.skills.clone())
        };
        AgentToolSkillConfig {
            agent_tools,
            agent_disallowed_tools,
            agent_skills,
        }
    }

    /// Resolve a session_key to a session_id.
    ///
    /// Lookup flow:
    /// 1. key_registry hit + active session → return session_id
    /// 2. key_registry hit + archived session → restore → return session_id
    /// 3. key_registry miss → create new session → register → return session_id
    pub async fn resolve(
        &self,
        session_key: &str,
        channel: &str,
        message: &Message,
        _account_id: Option<&str>, // used by key_registry rebuild
    ) -> Result<String, ProcessError> {
        // Path 1: key_registry hit — check if session is active
        let registry_hit = {
            let registry = self.key_registry.read().await;
            registry.get(session_key).cloned()
        };

        if let Some(session_id) = registry_hit {
            let session_exists = {
                let sessions = self.sessions.read().await;
                sessions.contains_key(&session_id)
            };
            if session_exists {
                self.update_checkpoint_thread_id(&session_id, &message.thread_id)
                    .await;
                return Ok(session_id);
            }

            // Path 2: key_registry hit but session not active — try restore
            let restored = self
                .try_restore_archived_session(&session_id, channel)
                .await;
            if restored {
                // Load checkpoint and set up conversation session + Session entry
                let storage_arc = {
                    let guard = self.storage.read().await;
                    guard.as_ref().map(Arc::clone)
                };
                if let Some(storage) = storage_arc {
                    if let Some(cp) = storage.load_checkpoint(&session_id).await.ok().flatten() {
                        // Ensure ConversationSession exists
                        let needs_conv = {
                            let cs = self.conversation_sessions.read().await;
                            !cs.contains_key(&session_id)
                        };
                        if needs_conv {
                            let agent_id =
                                cp.agent_id.clone().unwrap_or_else(|| message.to.clone());
                            let workdir_path = session_helpers::compute_session_workdir(
                                true,
                                &session_id,
                                message,
                                &self.workspace_dir,
                                &storage,
                            )
                            .await?;

                            let tool_registry = self.tool_registry.read().await;
                            let skill_registry = self.skill_registry.read().await.clone();
                            let agent_cfg = self.get_agent_config(&agent_id).await;
                            let filters = agent_cfg
                                .as_ref()
                                .map(Self::extract_agent_filters)
                                .unwrap_or_default();
                            let prompt = session_helpers::build_session_system_prompt(
                                &self.workspace_dir,
                                self.bootstrap_mode,
                                &tool_registry,
                                skill_registry,
                                &agent_id,
                                &filters,
                            )
                            .await;

                            let conv_session = ConversationSession::new(
                                session_id.clone(),
                                "default".to_string(),
                                workdir_path,
                            )
                            .with_system_prompt(prompt)
                            .with_reasoning_level(self.default_reasoning_level);
                            {
                                let mut cs = self.conversation_sessions.write().await;
                                cs.insert(session_id.clone(), Arc::new(RwLock::new(conv_session)));
                            }
                        }

                        // Restore pending messages and system_appends
                        {
                            let cs = self.conversation_sessions.read().await;
                            if let Some(cs) = cs.get(&session_id) {
                                let mut cs = cs.write().await;
                                cs.restore_pending_messages(cp.pending_messages.clone());
                                cs.restore_system_appends(cp.system_appends.clone());
                            }
                        }

                        // Create Session entry
                        {
                            let mut sessions = self.sessions.write().await;
                            if !sessions.contains_key(&session_id) {
                                sessions.insert(
                                    session_id.clone(),
                                    super::session_helpers::create_new_session(
                                        &session_id,
                                        message,
                                        channel,
                                    ),
                                );
                            }
                        }

                        // Save checkpoint with updated thread_id
                        let mut cp = cp;
                        cp.thread_id = message.thread_id.clone();
                        if let Err(e) = storage.save_checkpoint(&cp).await {
                            warn!(
                                session_id = %session_id,
                                error = %e,
                                "failed to save checkpoint after restore"
                            );
                        }
                    }
                }

                self.update_checkpoint_thread_id(&session_id, &message.thread_id)
                    .await;
                return Ok(session_id);
            }
        }

        // Path 3: key_registry miss — create a brand-new session
        let session_id = session_helpers::generate_session_id(&message.to);

        // Write to key_registry
        {
            let mut registry = self.key_registry.write().await;
            registry.insert(session_key.to_string(), session_id.clone());
        }

        // Build system prompt
        let tool_registry = self.tool_registry.read().await;
        let skill_registry = self.skill_registry.read().await.clone();
        let agent_id = message.to.clone();
        let agent_cfg = self.get_agent_config(&agent_id).await;
        let filters = agent_cfg
            .as_ref()
            .map(Self::extract_agent_filters)
            .unwrap_or_default();
        let prompt = session_helpers::build_session_system_prompt(
            &self.workspace_dir,
            self.bootstrap_mode,
            &tool_registry,
            skill_registry,
            &agent_id,
            &filters,
        )
        .await;

        // Compute workdir
        let workdir_path = if let Some(ref workspace_dir) = self.workspace_dir {
            workspace::ensure_workspace_dir(workspace_dir, &message.to, &message.from).map_err(
                |e| ProcessError::ProcessingFailed(format!("workspace creation failed: {}", e)),
            )?
        } else {
            PathBuf::from("/tmp")
        };

        // Create ConversationSession
        let conv_session =
            ConversationSession::new(session_id.clone(), "default".to_string(), workdir_path)
                .with_system_prompt(prompt)
                .with_reasoning_level(self.default_reasoning_level);
        {
            let mut conv_sessions = self.conversation_sessions.write().await;
            conv_sessions.insert(session_id.clone(), Arc::new(RwLock::new(conv_session)));
        }

        // Create Session entry
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(
                session_id.clone(),
                super::session_helpers::create_new_session(&session_id, message, channel),
            );
        }

        // Save checkpoint
        let mut cp = SessionCheckpoint::new(session_id.clone())
            .with_status(SessionStatus::Active)
            .with_platform(channel.to_string())
            .with_peer_id(message.to.clone())
            .with_agent_id(message.to.clone());
        if let Some(ref thread_id) = message.thread_id {
            cp = cp.with_thread_id(thread_id.clone());
        }
        // Persist sender (message.from) so rebuild_key_registry can reconstruct
        // the correct session_key format "{channel}:{from}:{to}".
        cp.sender_id = Some(message.from.clone());
        if let Some(storage) = self.storage.read().await.as_ref() {
            if let Err(e) = storage.save_checkpoint(&cp).await {
                warn!(
                    session_id = %session_id,
                    error = %e,
                    "failed to save new session checkpoint"
                );
            }
        }

        Ok(session_id)
    }
}
