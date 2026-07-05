//! Session key resolution: the unified entry point for mapping
//! session_key → session_id, with three lookup paths:
//! 1. key_registry hit + active session → return directly
//! 2. key_registry hit + archived session → restore → return
//! 3. key_registry miss → create new session → register → return

use super::session_helpers;
use super::session_helpers::AgentToolSkillConfig;
use super::SessionManager;
use crate::Message;
use closeclaw_common::processor::ProcessError;
use closeclaw_config::agents::ResolvedAgentConfig;
use closeclaw_llm::session::ConversationSession;
use closeclaw_session::persistence::{SessionCheckpoint, SessionStatus};
use closeclaw_session::workspace;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

impl SessionManager {
    /// Extract tool/skill filter configuration from an agent config.
    pub(super) fn extract_agent_filters(config: &ResolvedAgentConfig) -> AgentToolSkillConfig {
        AgentToolSkillConfig {
            agent_tools: config.effective_tools(),
            agent_disallowed_tools: config.effective_disallowed_tools(),
            agent_skills: config.effective_skills(),
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
        _session_key: &str,
        channel: &str,
        message: &Message,
        account_id: Option<&str>,
    ) -> Result<String, ProcessError> {
        // Compute stable routing_key from message fields (no timestamp).
        // Format: sha256("{account_id}:{channel}:{from}:{to}")
        let routing_key = Self::compute_routing_key(channel, message, account_id);

        // Path 1: key_registry hit — check if session is active
        let registry_hit = {
            let registry = self.key_registry.read().await;
            registry.get(&routing_key).cloned()
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
            if self
                .try_restore_archived_session(&session_id, channel)
                .await
            {
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

                            let _tool_registry = self.tool_registry.read().await;
                            let _skill_registry = self.skill_registry.read().await.clone();
                            let agent_cfg = self.get_agent_config(&agent_id).await;
                            let _filters = agent_cfg
                                .as_ref()
                                .map(Self::extract_agent_filters)
                                .unwrap_or_default();
                            let overrides = self.prompt_overrides.read().await.clone();
                            let prompt = if let Some(builder) =
                                self.system_prompt_builder.read().await.as_ref()
                            {
                                session_helpers::build_session_system_prompt(
                                    builder.as_ref(),
                                    &session_id,
                                    &agent_id,
                                    overrides.as_ref(),
                                )
                                .await
                            } else {
                                String::new()
                            };

                            let mut conv_session = ConversationSession::new(
                                session_id.clone(),
                                "default".to_string(),
                                workdir_path,
                            )
                            .with_system_prompt(prompt)
                            .with_reasoning_level(self.default_reasoning_level);
                            // Wire shutdown handle for busy-count tracking.
                            if let Some(sh) = self.get_shutdown_handle().await {
                                conv_session.set_shutdown_handle(sh);
                            }
                            // Inject LLM caller and system prompt builder for delegation.
                            if let Some(caller) = self.get_llm_caller().await {
                                conv_session.set_llm_caller(caller);
                            }
                            if let Some(builder) = self.get_system_prompt_builder().await {
                                conv_session.set_system_prompt_builder(builder);
                            }
                            conv_session.set_prompt_overrides(self.get_prompt_overrides().await);
                            {
                                let mut cs = self.conversation_sessions.write().await;
                                cs.insert(session_id.clone(), Arc::new(RwLock::new(conv_session)));
                            }
                        }

                        // Restore pending messages, system_appends, and verbosity_level
                        {
                            let cs = self.conversation_sessions.read().await;
                            if let Some(cs) = cs.get(&session_id) {
                                let mut cs = cs.write().await;
                                cs.restore_pending_messages(cp.pending_messages.clone());
                                cs.restore_system_appends(cp.system_appends.clone());
                                cs.set_verbosity_level(cp.verbosity_level);
                            }
                        }

                        // Inject recovery notifications and tool failure results
                        // from checkpoint (set by SessionRecoveryService during startup).
                        if let Some(ref notification) = cp.recovery_notification {
                            let cs = self.conversation_sessions.read().await;
                            if let Some(cs) = cs.get(&session_id) {
                                let mut cs = cs.write().await;
                                cs.inject_system_message(notification.clone());
                                for failure in &cp.pending_tool_failures {
                                    // Extract op_id from the JSON failure string to use
                                    // as tool_call_id.  Falls back to "recovery" if parsing
                                    // fails (defensive — the JSON is built by the recovery
                                    // service and always contains op_id).
                                    let tool_call_id =
                                        serde_json::from_str::<serde_json::Value>(failure)
                                            .ok()
                                            .and_then(|v| {
                                                v.get("op_id")?.as_str().map(String::from)
                                            })
                                            .unwrap_or_else(|| "recovery".to_string());
                                    cs.inject_tool_result(&tool_call_id, failure);
                                }
                                info!(
                                    session_id = %session_id,
                                    "injected recovery notification and {} tool failure(s)",
                                    cp.pending_tool_failures.len()
                                );
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

        // Write to key_registry using routing_key (no timestamps)
        {
            let mut registry = self.key_registry.write().await;
            registry.insert(routing_key.to_string(), session_id.clone());
        }

        // Build system prompt
        let _tool_registry = self.tool_registry.read().await;
        let _skill_registry = self.skill_registry.read().await.clone();
        let agent_id = message.to.clone();
        let agent_cfg = self.get_agent_config(&agent_id).await;
        let _filters = agent_cfg
            .as_ref()
            .map(Self::extract_agent_filters)
            .unwrap_or_default();
        let overrides = self.prompt_overrides.read().await.clone();
        let prompt = if let Some(builder) = self.system_prompt_builder.read().await.as_ref() {
            session_helpers::build_session_system_prompt(
                builder.as_ref(),
                &session_id,
                &agent_id,
                overrides.as_ref(),
            )
            .await
        } else {
            String::new()
        };

        // Compute workdir
        let workdir_path = if let Some(ref workspace_dir) = self.workspace_dir {
            workspace::ensure_workspace_dir(workspace_dir, &message.to, &message.from).map_err(
                |e| ProcessError::ChainFailed(format!("workspace creation failed: {}", e)),
            )?
        } else {
            PathBuf::from("/tmp")
        };

        // Create ConversationSession
        let mut conv_session =
            ConversationSession::new(session_id.clone(), "default".to_string(), workdir_path)
                .with_system_prompt(prompt)
                .with_reasoning_level(self.default_reasoning_level);
        // Wire shutdown handle for busy-count tracking.
        if let Some(sh) = self.get_shutdown_handle().await {
            conv_session.set_shutdown_handle(sh);
        }
        // Inject LLM caller and system prompt builder for delegation.
        if let Some(caller) = self.get_llm_caller().await {
            conv_session.set_llm_caller(caller);
        }
        if let Some(builder) = self.get_system_prompt_builder().await {
            conv_session.set_system_prompt_builder(builder);
        }
        conv_session.set_prompt_overrides(self.get_prompt_overrides().await);
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
        // Persist routing fields so rebuild_key_registry can reconstruct
        // the correct routing_key format "{account_id}:{channel}:{from}:{to}".
        cp.sender_id = Some(message.from.clone());
        cp.account_id = account_id.map(String::from);
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
