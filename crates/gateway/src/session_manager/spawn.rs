//! Child session creation and tracking for `SessionManager`.
//!
//! `create_child_session` delegates the core ConversationSession creation
//! to `closeclaw_session::spawn::create_child_conversation_session` and
//! handles the gateway-specific registration steps (conversation_sessions
//! map, sessions map, children table, checkpoint persistence, timeout).

use super::SessionManager;
use crate::session_manager::communication::CommunicationError;
use crate::Session;
use closeclaw_config::agents::ResolvedAgentConfig;
use closeclaw_session::persistence::{
    PendingMessage, PersistenceError, SessionCheckpoint, SessionStatus,
};
use closeclaw_session::spawn as session_spawn;
use std::collections::HashSet;
use tracing::warn;

#[cfg(test)]
use closeclaw_session::spawn::creation::build_spawn_context as build_spawn_context_inner;
pub use closeclaw_session::spawn::{ChildSessionInfo, SpawnMode};

impl SessionManager {
    /// Get the depth of a session. Returns None if session does not exist.
    pub async fn get_session_depth(&self, session_id: &str) -> Option<u32> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).map(|s| s.depth)
    }

    /// Get the effective max spawn depth budget for a session.
    pub async fn get_effective_max_spawn_depth(&self, session_id: &str) -> Option<u32> {
        let storage = self.storage.read().await;
        let storage = storage.as_ref()?;
        match storage.load_checkpoint(session_id).await {
            Ok(Some(cp)) => cp.effective_max_spawn_depth,
            _ => None,
        }
    }

    /// Get the parent session ID of a given child session.
    pub async fn get_parent_of(&self, child_id: &str) -> Option<String> {
        let children = self.children.read().await;
        children.get_parent(child_id)
    }

    /// Count active (non-completed) child sessions for a parent.
    pub async fn count_active_children(&self, parent_id: &str) -> usize {
        let child_ids: Vec<String> = {
            let children = self.children.read().await;
            children
                .list_children(parent_id)
                .into_iter()
                .map(|info| info.session_id.clone())
                .collect()
        };
        if child_ids.is_empty() {
            return 0;
        }
        let conv = self.conversation_sessions.read().await;
        child_ids
            .iter()
            .filter(|id| conv.contains_key(id.as_str()))
            .count()
    }

    /// List all active child session IDs for a parent.
    #[allow(dead_code)]
    pub async fn list_active_child_ids(&self, parent_id: &str) -> Vec<String> {
        let children = self.children.read().await;
        children
            .list_children(parent_id)
            .into_iter()
            .map(|info| info.session_id.clone())
            .collect()
    }

    /// Register a child session under its parent.
    pub async fn register_child(&self, parent_id: &str, info: ChildSessionInfo) {
        let mut children = self.children.write().await;
        children.register_child(parent_id, info);
    }

    /// Build the spawn context paragraph appended to child system prompts.
    /// Delegates to the session crate's implementation.
    #[cfg(test)]
    pub(crate) fn build_spawn_context(
        depth: u32,
        max_spawn_depth: u32,
        parent_session_id: &str,
        spawn_mode: &SpawnMode,
        fork: bool,
    ) -> String {
        build_spawn_context_inner(depth, max_spawn_depth, parent_session_id, spawn_mode, fork)
    }

    /// Create a child session for a spawned sub-agent.
    ///
    /// Delegates core ConversationSession creation to the session crate,
    /// then handles gateway-specific registration (maps, checkpoint,
    /// spawn tree, timeout).
    #[allow(clippy::too_many_arguments)]
    pub async fn create_child_session(
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
    ) -> Result<String, String> {
        // ── Increment busy count for drain tracking ────────────────────
        if let Some(sh) = self.get_shutdown_handle().await {
            sh.increment_busy();
        }

        // ── Shutdown gate: reject new child session creation ──────────
        if let Some(sh) = self.get_shutdown_handle().await {
            if sh.is_shutting_down() {
                tracing::warn!(
                    parent_session_id = %parent_session_id,
                    "rejecting child session creation: daemon is shutting down"
                );
                sh.decrement_busy();
                return Err("daemon is shutting down".into());
            }
        }

        // Apply tool whitelist override.
        let config = if let Some(ref tools) = allowed_tools {
            let mut overridden = config.clone();
            overridden.tools = tools.clone();
            overridden
        } else {
            config.clone()
        };

        // Tool-level spawn prevention (design doc §两层防护).
        let config = if max_spawn_depth == 0 {
            let mut filtered = config.clone();
            filtered.tools.retain(|t| t != "sessions_spawn");
            filtered
        } else {
            config
        };

        // Resolve parent agent ID for communication config.
        let parent_agent_id = self
            .get_chat_id(parent_session_id)
            .await
            .unwrap_or_default();

        // ── Delegate core creation to session crate ──────────────────
        let params = session_spawn::ChildSessionCreationParams {
            parent_session_id,
            parent_agent_id: &parent_agent_id,
            depth,
            task,
            light_context,
            workspace,
            mode: mode.clone(),
            fork,
            model_override,
            parent_subagents_model,
            max_spawn_depth,
        };
        let created = session_spawn::create_child_conversation_session(
            self, // SpawnCreationContext impl
            &config, &params,
        )
        .await?;

        let child_session_id = created.session_id.clone();
        let child_cs_arc = created.conversation_session;

        // Retain the cancel token for the optional spawn timeout.
        let timeout_token = {
            let guard = child_cs_arc.read().await;
            guard.cancel_token.clone()
        };

        // ── Gateway-specific registration ────────────────────────────

        // Register in conversation_sessions.
        {
            let mut conv_sessions = self.conversation_sessions.write().await;
            conv_sessions.insert(child_session_id.clone(), child_cs_arc.clone());
        }

        // Register child handle with parent for cascade stop.
        {
            let conv_sessions = self.conversation_sessions.read().await;
            if let Some(parent_cs) = conv_sessions.get(parent_session_id) {
                parent_cs.read().await.register_child_handle(
                    &child_session_id,
                    std::sync::Arc::downgrade(&child_cs_arc),
                );
            }
        }

        // Register in sessions map.
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(
                child_session_id.clone(),
                Session {
                    id: child_session_id.clone(),
                    agent_id: config.id.clone(),
                    channel: "spawn".to_string(),
                    created_at: chrono::Utc::now().timestamp(),
                    depth,
                },
            );
        }

        // Persist checkpoint.
        let cp = SessionCheckpoint::new(child_session_id.clone())
            .with_status(SessionStatus::Active)
            .with_platform("spawn".to_string())
            .with_agent_id(config.id.clone())
            .with_parent_session_id(parent_session_id.to_string())
            .with_depth(depth)
            .with_effective_max_spawn_depth(Some(max_spawn_depth));
        if let Some(storage) = self.storage.read().await.as_ref() {
            if let Err(e) = storage.save_checkpoint(&cp).await {
                warn!(
                    session_id = %child_session_id,
                    error = %e,
                    "failed to save child session checkpoint"
                );
            }
        }

        // Register in children tracking table.
        self.register_child(
            parent_session_id,
            ChildSessionInfo {
                session_id: child_session_id.clone(),
                parent_session_id: parent_session_id.to_string(),
                agent_id: config.id.clone(),
                depth,
                mode,
            },
        )
        .await;

        // Apply spawn timeout.
        if let Some(timeout_secs) = spawn_timeout {
            let token = timeout_token;
            let child_id = child_session_id.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)).await;
                tracing::info!(
                    session_id = %child_id,
                    timeout_secs,
                    "spawn timeout expired, cancelling child session"
                );
                token.cancel();
            });
        }

        Ok(child_session_id)
    }

    /// Validate that a child session is owned by the given parent.
    #[allow(dead_code)]
    pub async fn validate_child_ownership(
        &self,
        parent_id: &str,
        child_id: &str,
    ) -> Option<ChildSessionInfo> {
        let children = self.children.read().await;
        children
            .list_children(parent_id)
            .into_iter()
            .find(|info| info.session_id == child_id)
            .cloned()
    }

    /// Inject a new task into a persistent child session's pending
    /// message queue.
    #[allow(dead_code)]
    pub async fn steer_child(&self, child_id: &str, task: &str) -> Result<(), String> {
        let parent_session_id = self
            .get_parent_of(child_id)
            .await
            .ok_or_else(|| format!("no parent registered for child session: {}", child_id))?;

        self.check_session_communication(&parent_session_id, child_id)
            .await
            .map_err(|e| match e {
                CommunicationError::Denied { reason } => {
                    format!("steer blocked by communication policy: {}", reason)
                }
                CommunicationError::SessionNotFound(s) => {
                    format!("session not found: {}", s)
                }
                other => format!("communication check failed: {}", other),
            })?;

        let cs = self
            .get_conversation_session(child_id)
            .await
            .ok_or_else(|| format!("child session not found: {}", child_id))?;
        let pending_msg = PendingMessage::with_role(
            format!("{}-steer", child_id),
            task.to_string(),
            "assistant".to_string(),
        );
        cs.write().await.push_pending(pending_msg);
        Ok(())
    }

    /// Force-terminate a child session and all its descendants.
    pub async fn kill_child(&self, parent_id: &str, child_id: &str) -> Result<(), String> {
        let descendant_ids = {
            let children = self.children.read().await;
            children.list_descendants(child_id)
        };

        for id in &descendant_ids {
            if let Some(cs) = self.get_conversation_session(id).await {
                cs.read().await.stop(true).await;
            }
            self.conversation_sessions.write().await.remove(id);
            if let Some(info) = self.children.read().await.find_child(id) {
                let pid = info.parent_session_id.clone();
                if let Some(pcs) = self.conversation_sessions.read().await.get(&pid) {
                    pcs.read().await.unregister_child_handle(id);
                }
            }
            self.sessions.write().await.remove(id);
            self.children
                .write()
                .await
                .remove_descendant_entries(std::slice::from_ref(id));
        }

        if let Some(cs) = self.get_conversation_session(child_id).await {
            cs.read().await.stop(true).await;
        }
        self.conversation_sessions.write().await.remove(child_id);
        if let Some(pcs) = self.conversation_sessions.read().await.get(parent_id) {
            pcs.read().await.unregister_child_handle(child_id);
        }
        self.sessions.write().await.remove(child_id);
        self.children
            .write()
            .await
            .remove_child(parent_id, child_id);
        Ok(())
    }

    /// Cascade-kill all active children of a session.
    pub async fn cascade_kill_all_children(&self, parent_id: &str) {
        let child_ids: Vec<String> = {
            let children = self.children.read().await;
            children
                .list_children(parent_id)
                .into_iter()
                .map(|info| info.session_id.clone())
                .collect()
        };
        for child_id in child_ids {
            if let Err(e) = self.kill_child(parent_id, &child_id).await {
                tracing::warn!(
                    parent = %parent_id,
                    child = %child_id,
                    error = %e,
                    "cascade_kill_all_children: failed to kill child"
                );
            }
        }
    }

    /// Rebuild the spawn tree (children table) from persisted checkpoints.
    pub async fn rebuild_spawn_tree(&self) -> Result<(), PersistenceError> {
        let storage_arc = {
            let guard = self.storage.read().await;
            match guard.as_ref() {
                Some(s) => std::sync::Arc::clone(s),
                None => return Ok(()),
            }
        };

        let mut all_ids: Vec<String> = {
            let active = storage_arc.list_active_sessions().await?;
            let archived = storage_arc.list_archived_sessions().await?;
            let mut ids = active;
            ids.extend(archived);
            ids
        };
        all_ids.sort();
        all_ids.dedup();

        let known_ids: HashSet<&str> = all_ids.iter().map(|s| s.as_str()).collect();
        let mut rebuilt: u32 = 0;
        let mut orphan_ids: Vec<String> = Vec::new();

        for session_id in &all_ids {
            let cp = match storage_arc.load_checkpoint(session_id).await {
                Ok(Some(cp)) => cp,
                Ok(None) => {
                    warn!(
                        session_id = %session_id,
                        "checkpoint returned None during spawn_tree rebuild, skipping"
                    );
                    continue;
                }
                Err(e) => {
                    warn!(
                        session_id = %session_id,
                        error = %e,
                        "failed to load checkpoint during spawn_tree rebuild, skipping"
                    );
                    continue;
                }
            };
            let parent_id = match cp.parent_session_id.as_deref() {
                Some(p) => p,
                None => continue,
            };
            if !known_ids.contains(parent_id) {
                orphan_ids.push(session_id.clone());
                continue;
            }
            self.register_child(
                parent_id,
                ChildSessionInfo {
                    session_id: session_id.clone(),
                    parent_session_id: parent_id.to_string(),
                    agent_id: cp.agent_id.unwrap_or_default(),
                    depth: cp.depth,
                    mode: SpawnMode::Session,
                },
            )
            .await;
            rebuilt += 1;
        }

        if !orphan_ids.is_empty() {
            let mut sessions = self.sessions.write().await;
            for orphan_id in &orphan_ids {
                if let Some(s) = sessions.get_mut(orphan_id) {
                    s.depth = 0;
                }
            }
        }

        tracing::info!(rebuilt, "spawn_tree rebuilt from checkpoints");
        Ok(())
    }
}
