//! Session key resolution: the unified entry point for mapping
//! session_key → session_id, with three lookup paths:
//! 1. key_registry hit + active session → return directly
//! 2. key_registry hit + archived session → restore → return
//! 3. key_registry miss → create new session → register → return

use super::session_helpers;
use super::SessionManager;
use crate::Message;
use closeclaw_common::processor::ProcessError;
use closeclaw_session::bootstrap::loader::BootstrapMode;
use closeclaw_session::checkpoint_manager::CheckpointManager;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::{PersistenceService, SessionCheckpoint, SessionStatus};
use closeclaw_session::run_health::TranscriptOp;
use closeclaw_session::workspace;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

impl SessionManager {
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
        account_id: Option<&str>,
        agent_id: &str,
    ) -> Result<String, ProcessError> {
        // Acquire per-agent lock to serialize resolve for the same agent_id.
        // Different agent_ids run in parallel; the same agent_id is serialized.
        let agent_id = agent_id.to_string();
        let agent_lock = {
            let mut locks = self.agent_locks.write().await;
            locks
                .entry(agent_id.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _agent_guard = agent_lock.lock().await;

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
                // Verify checkpoint status: if Sweeper archived this session,
                // the stale registry entry must be removed so we fall through
                // to Path 3 (create new session) instead of returning a dead
                // in-memory reference.
                let cm_arc = {
                    let guard = self.checkpoint_manager.read().await;
                    guard.as_ref().map(Arc::clone)
                };
                if let Some(ref cm) = cm_arc {
                    match cm.load(&session_id).await {
                        Ok(Some(cp)) => match cp.status {
                            SessionStatus::Active => {
                                self.update_checkpoint_thread_id(&session_id, &message.thread_id)
                                    .await;
                                return Ok(session_id);
                            }
                            SessionStatus::Migrating => {
                                // Per design doc: migrating session → wait for
                                // archive completion → restore as archived.
                                warn!(
                                    session_key = %session_key,
                                    session_id = %session_id,
                                    routing_key = %routing_key,
                                    status = %cp.status,
                                    "session in registry is migrating, waiting for archive to complete"
                                );
                                // Inject archiving notification (consumed by Gateway
                                // before this resolve returns).
                                {
                                    let mut pending =
                                        self.pending_restore_notifications.write().await;
                                    pending.insert(
                                        session_id.clone(),
                                        (
                                            channel.to_string(),
                                            Some("⏳ 会话归档中，稍后恢复…".to_string()),
                                        ),
                                    );
                                }
                                // Wait for archive completion (bounded poll,
                                // shared helper with registry miss path).
                                let archived =
                                    Self::wait_for_archive_completion(cm, &session_id).await;
                                if archived {
                                    info!(
                                        session_key = %session_key,
                                        session_id = %session_id,
                                        routing_key = %routing_key,
                                        "migrating session finished archiving, restoring archived session"
                                    );
                                } else {
                                    warn!(
                                        session_key = %session_key,
                                        session_id = %session_id,
                                        routing_key = %routing_key,
                                        "migrating session archive timed out after 5 s, falling through to create new session"
                                    );
                                }
                                // Remove stale registry entry and in-memory session.
                                {
                                    let mut registry = self.key_registry.write().await;
                                    registry.remove(&routing_key);
                                }
                                self.remove_session(&session_id).await;
                                // Fall through to Path 3.  If archived, the
                                // archived check there will pick it up.
                            }
                            SessionStatus::Archived => {
                                warn!(
                                    session_key = %session_key,
                                    session_id = %session_id,
                                    routing_key = %routing_key,
                                    "session in registry is archived, removing stale entry"
                                );
                                let mut registry = self.key_registry.write().await;
                                registry.remove(&routing_key);
                                // Clean up sessions map and conversation_sessions map
                                // to prevent stale entries from lingering.
                                self.remove_session(&session_id).await;
                                // Fall through to Path 3
                            }
                        },
                        Ok(None) => {
                            // No checkpoint on disk — treat as active (defensive)
                            self.update_checkpoint_thread_id(&session_id, &message.thread_id)
                                .await;
                            return Ok(session_id);
                        }
                        Err(e) => {
                            warn!(
                                session_key = %session_key,
                                session_id = %session_id,
                                routing_key = %routing_key,
                                error = %e,
                                "failed to load checkpoint status, falling back to existing session"
                            );
                            self.update_checkpoint_thread_id(&session_id, &message.thread_id)
                                .await;
                            return Ok(session_id);
                        }
                    }
                } else {
                    // No checkpoint manager — fall back to original behavior
                    self.update_checkpoint_thread_id(&session_id, &message.thread_id)
                        .await;
                    return Ok(session_id);
                }
            }

            // Path 2: key_registry hit but session not active — try restore
            if self
                .try_restore_archived_session(&session_id, channel)
                .await
            {
                // Load checkpoint and set up conversation session + Session entry
                let cm_arc = {
                    let guard = self.checkpoint_manager.read().await;
                    guard.as_ref().map(Arc::clone)
                };
                if let Some(cm) = cm_arc {
                    if let Some(cp) = cm.load(&session_id).await.ok().flatten() {
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
                                cm.as_ref(),
                            )
                            .await?;

                            let mut conv_session = ConversationSession::new(
                                session_id.clone(),
                                "default".to_string(),
                                workdir_path,
                            )
                            .with_system_prompt("")
                            .with_reasoning_level(self.default_reasoning_level);
                            self.apply_default_cache_break_thresholds(&mut conv_session);
                            // Wire shutdown handle for busy-count tracking.
                            if let Some(sh) = self.get_shutdown_handle().await {
                                conv_session.set_shutdown_handle(sh);
                            }
                            // Inject LLM caller and system prompt builder for delegation.
                            let agent_hooks = self
                                .get_agent_config(&agent_id)
                                .await
                                .map(|c| c.hooks)
                                .unwrap_or_default();
                            if let Some(caller) = self.get_llm_caller().await {
                                conv_session.set_llm_caller(caller.clone());
                                conv_session.init_health_checker(caller, agent_hooks);
                            }
                            if let Some(builder) = self.get_system_prompt_builder().await {
                                conv_session.set_system_prompt_builder(builder);
                            }
                            conv_session.set_prompt_overrides(self.get_prompt_overrides().await);
                            // Inject dynamic prompt builder for per-request
                            // dynamic-layer injection (ChannelContext, etc.).
                            if let Some(dpb) = self.get_dynamic_prompt_builder().await {
                                conv_session.set_dynamic_prompt_builder(dpb);
                            }
                            // Inject skill listing provider and agent skills.
                            self.wire_skill_listing_deps(&mut conv_session, &agent_id)
                                .await;
                            // Query bootstrap mode from AgentRegistry and cache.
                            let bootstrap_mode = self
                                .query_agent_bootstrap_mode(&agent_id)
                                .await
                                .unwrap_or(BootstrapMode::Full);
                            conv_session = conv_session.with_bootstrap_mode(bootstrap_mode);
                            // Build initial system prompt via session's own builder.
                            info!(
                                session_id = %session_id,
                                event = "session_injection",
                                trigger = "archived_session_restore",
                                "full injection for archived session (new ConversationSession)"
                            );
                            conv_session
                                .rebuild_system_prompt(&session_id, &agent_id, Some(bootstrap_mode))
                                .await;
                            // Inject snapshot meta store for persistence.
                            self.inject_snapshot_meta_store(&session_id, &mut conv_session)
                                .await;
                            // Inject checkpoint storage for pending-operation persistence.
                            self.inject_checkpoint_storage(&mut conv_session).await;
                            // Apply session config (git_status switch).
                            if let Some(cfg) = self.get_session_config_for_agent(&agent_id).await {
                                conv_session.set_git_status(cfg.is_git_status_enabled);
                            }
                            {
                                let mut cs = self.conversation_sessions.write().await;
                                cs.insert(session_id.clone(), Arc::new(RwLock::new(conv_session)));
                            }
                        } else {
                            info!(
                                session_id = %session_id,
                                event = "session_injection",
                                trigger = "archived_session_restore",
                                "rebuilding prompt for archived session in memory"
                            );
                            self.rebuild_archived_session_prompt(&session_id, &cp, message)
                                .await;
                        }

                        // Restore pending messages, system_appends, verbosity_level,
                        // and communication_config from checkpoint.
                        // NOTE: system_appends must be restored AFTER rebuild_system_prompt
                        // so that user appends layer on top of the rebuilt prompt.
                        {
                            let cs = self.conversation_sessions.read().await;
                            if let Some(cs) = cs.get(&session_id) {
                                let mut cs = cs.write().await;
                                cs.restore_pending_messages(cp.outbound_pending.clone());
                                cs.restore_system_appends(cp.system_appends.clone());
                                cs.set_verbosity_level(cp.verbosity_level);
                                // Restore communication config for spawned sessions.
                                if let Some(ref comm_config) = cp.communication_config {
                                    cs.set_communication_config(comm_config.clone());
                                }
                                // Restore transcript from checkpoint ("transcript is the
                                // single source of truth" per design doc).
                                if !cp.pending_messages.is_empty() {
                                    cs.apply_transcript_op(
                                        TranscriptOp::Rewrite,
                                        cp.pending_messages.clone(),
                                    );
                                }
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
                                    session_key = %session_key,
                                    session_id = %session_id,
                                    routing_key = %routing_key,
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
                        if let Err(e) = cm.save_raw(&cp).await {
                            warn!(
                                session_key = %session_key,
                                session_id = %session_id,
                                routing_key = %routing_key,
                                error = %e,
                                "failed to save checkpoint after restore"
                            );
                        }
                    }
                }

                // Re-register routing_key so subsequent lookups find
                // the restored session instead of creating a duplicate.
                {
                    let mut registry = self.key_registry.write().await;
                    registry.insert(routing_key.clone(), session_id.clone());
                }
                self.update_checkpoint_thread_id(&session_id, &message.thread_id)
                    .await;
                return Ok(session_id);
            }
        }

        // Path 3: key_registry miss — create a brand-new session
        // Collision check: if routing_key already exists in the registry,
        // another thread may be concurrently creating a session for the
        // same routing fields. Wait 10ms and retry.
        // Per design doc: "极罕见碰撞时 SessionManager 等待 10ms 后重试".
        {
            let registry = self.key_registry.read().await;
            if let Some(existing_id) = registry.get(&routing_key) {
                let existing_id = existing_id.clone();
                drop(registry);
                // Check if the session created by the other thread is active
                let session_exists = {
                    let sessions = self.sessions.read().await;
                    sessions.contains_key(&existing_id)
                };
                if session_exists {
                    self.update_checkpoint_thread_id(&existing_id, &message.thread_id)
                        .await;
                    return Ok(existing_id);
                }
                // Session not yet visible (concurrent creation in progress)
                warn!(
                    session_key = %session_key,
                    routing_key = %routing_key,
                    "session_key collision detected, sleeping 10ms and retrying"
                );
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                // Re-check after delay — the other thread should have finished
                let registry = self.key_registry.read().await;
                if let Some(retry_id) = registry.get(&routing_key) {
                    let retry_id = retry_id.clone();
                    drop(registry);
                    let session_exists = {
                        let sessions = self.sessions.read().await;
                        sessions.contains_key(&retry_id)
                    };
                    if session_exists {
                        self.update_checkpoint_thread_id(&retry_id, &message.thread_id)
                            .await;
                        return Ok(retry_id);
                    }
                }
            }
        }
        // SQLite double-check: query storage for an existing active session
        // with the same routing fields. This covers the edge case where the
        // key_registry was not yet written but SQLite already has a record
        // (e.g., concurrent creation, or key_registry lost on restart).
        let sqlite_check = {
            let cm_guard = self.checkpoint_manager.read().await;
            match cm_guard.as_ref() {
                Some(cm) => cm
                    .storage()
                    .find_active_session_by_routing(account_id, channel, &message.from, &message.to)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            }
        };
        if let Some(existing_id) = sqlite_check {
            // Self-heal: register the existing session in key_registry.
            {
                let mut registry = self.key_registry.write().await;
                registry.insert(routing_key.clone(), existing_id.clone());
            }
            // Also ensure it's visible in the in-memory sessions map.
            let session_exists = {
                let sessions = self.sessions.read().await;
                sessions.contains_key(&existing_id)
            };
            if !session_exists {
                let mut sessions = self.sessions.write().await;
                sessions.insert(
                    existing_id.clone(),
                    super::session_helpers::create_new_session(&existing_id, message, channel),
                );
            }
            self.update_checkpoint_thread_id(&existing_id, &message.thread_id)
                .await;
            info!(
                session_key = %session_key,
                session_id = %existing_id,
                routing_key = %routing_key,
                "SQLite double-check: found existing active session, self-healed"
            );
            return Ok(existing_id);
        }
        // Migrating session check: if no active session found in SQLite,
        // check for a migrating session and wait for archive completion
        // before falling through to the archived check.
        let migrating_check = {
            let cm_guard = self.checkpoint_manager.read().await;
            match cm_guard.as_ref() {
                Some(cm) => cm
                    .storage()
                    .find_migrating_session_by_routing(
                        account_id,
                        channel,
                        &message.from,
                        &message.to,
                    )
                    .await
                    .ok()
                    .flatten(),
                None => None,
            }
        };
        if let Some(migrating_id) = migrating_check {
            warn!(
                session_key = %session_key,
                session_id = %migrating_id,
                routing_key = %routing_key,
                "found migrating session in SQLite, waiting for archive"
            );
            // Inject archiving notification.
            {
                let mut pending = self.pending_restore_notifications.write().await;
                pending.insert(
                    migrating_id.clone(),
                    (
                        channel.to_string(),
                        Some("⏳ 会话归档中，稍后恢复…".to_string()),
                    ),
                );
            }
            // Bounded poll: wait for Sweeper to transition status.
            let cm_arc = {
                let guard = self.checkpoint_manager.read().await;
                guard.as_ref().map(Arc::clone)
            };
            if let Some(ref cm) = cm_arc {
                let archived = Self::wait_for_archive_completion(cm, &migrating_id).await;
                if archived {
                    info!(
                        session_key = %session_key,
                        session_id = %migrating_id,
                        routing_key = %routing_key,
                        "migrating session finished archiving, falling through to archived restore"
                    );
                } else {
                    warn!(
                        session_key = %session_key,
                        session_id = %migrating_id,
                        routing_key = %routing_key,
                        "migrating session archive timed out, creating new session"
                    );
                }
            }
            // Fall through to archived check; if archived, it will
            // pick up the session.  If not, a new session is created.
        }
        // Archived session check: if no active session found in SQLite,
        // check for an archived session that can be restored.
        let archived_check = {
            let cm_guard = self.checkpoint_manager.read().await;
            match cm_guard.as_ref() {
                Some(cm) => cm
                    .storage()
                    .find_archived_session_by_routing(
                        account_id,
                        channel,
                        &message.from,
                        &message.to,
                    )
                    .await
                    .ok()
                    .flatten(),
                None => None,
            }
        };
        if let Some(archived_id) = archived_check {
            if self
                .try_restore_archived_session(&archived_id, channel)
                .await
            {
                // Load checkpoint and set up conversation session + Session entry
                let cm_arc = {
                    let guard = self.checkpoint_manager.read().await;
                    guard.as_ref().map(Arc::clone)
                };
                if let Some(cm) = cm_arc {
                    if let Some(cp) = cm.load(&archived_id).await.ok().flatten() {
                        // Ensure ConversationSession exists
                        let needs_conv = {
                            let cs = self.conversation_sessions.read().await;
                            !cs.contains_key(&archived_id)
                        };
                        if needs_conv {
                            let agent_id =
                                cp.agent_id.clone().unwrap_or_else(|| message.to.clone());
                            let workdir_path = session_helpers::compute_session_workdir(
                                true,
                                &archived_id,
                                message,
                                &self.workspace_dir,
                                cm.as_ref(),
                            )
                            .await?;

                            let mut conv_session = ConversationSession::new(
                                archived_id.clone(),
                                "default".to_string(),
                                workdir_path,
                            )
                            .with_system_prompt("")
                            .with_reasoning_level(self.default_reasoning_level);
                            self.apply_default_cache_break_thresholds(&mut conv_session);
                            // Wire shutdown handle for busy-count tracking.
                            if let Some(sh) = self.get_shutdown_handle().await {
                                conv_session.set_shutdown_handle(sh);
                            }
                            // Inject LLM caller and system prompt builder.
                            let agent_hooks = self
                                .get_agent_config(&agent_id)
                                .await
                                .map(|c| c.hooks)
                                .unwrap_or_default();
                            if let Some(caller) = self.get_llm_caller().await {
                                conv_session.set_llm_caller(caller.clone());
                                conv_session.init_health_checker(caller, agent_hooks);
                            }
                            if let Some(builder) = self.get_system_prompt_builder().await {
                                conv_session.set_system_prompt_builder(builder);
                            }
                            conv_session.set_prompt_overrides(self.get_prompt_overrides().await);
                            // Inject dynamic prompt builder for per-request
                            // dynamic-layer injection (ChannelContext, etc.).
                            if let Some(dpb) = self.get_dynamic_prompt_builder().await {
                                conv_session.set_dynamic_prompt_builder(dpb);
                            }
                            // Inject skill listing provider and agent skills.
                            self.wire_skill_listing_deps(&mut conv_session, &agent_id)
                                .await;
                            // Query bootstrap mode from AgentRegistry and cache.
                            let bootstrap_mode = self
                                .query_agent_bootstrap_mode(&agent_id)
                                .await
                                .unwrap_or(BootstrapMode::Full);
                            conv_session = conv_session.with_bootstrap_mode(bootstrap_mode);
                            // Build initial system prompt via session's own builder.
                            info!(
                                session_id = %archived_id,
                                agent_id = %agent_id,
                                event = "session_injection",
                                trigger = "archived_session_restore",
                                "archived session: full deps injection (new ConversationSession)"
                            );
                            conv_session
                                .rebuild_system_prompt(
                                    &archived_id,
                                    &agent_id,
                                    Some(bootstrap_mode),
                                )
                                .await;
                            // Inject snapshot meta store for persistence.
                            self.inject_snapshot_meta_store(&archived_id, &mut conv_session)
                                .await;
                            // Inject checkpoint storage for pending-operation persistence.
                            self.inject_checkpoint_storage(&mut conv_session).await;
                            // Apply session config (git_status switch).
                            if let Some(cfg) = self.get_session_config_for_agent(&agent_id).await {
                                conv_session.set_git_status(cfg.is_git_status_enabled);
                            }
                            {
                                let mut cs = self.conversation_sessions.write().await;
                                cs.insert(archived_id.clone(), Arc::new(RwLock::new(conv_session)));
                            }
                        } else {
                            info!(
                                session_id = %archived_id,
                                event = "session_injection",
                                trigger = "archived_session_restore",
                                "rebuilding prompt for archived session already in memory"
                            );
                            self.rebuild_archived_session_prompt(&archived_id, &cp, message)
                                .await;
                        }
                        // Restore pending messages, system_appends, verbosity_level,
                        // and communication_config from checkpoint.
                        // NOTE: system_appends must be restored AFTER rebuild_system_prompt
                        // so that user appends layer on top of the rebuilt prompt.
                        {
                            let cs = self.conversation_sessions.read().await;
                            if let Some(cs) = cs.get(&archived_id) {
                                let mut cs = cs.write().await;
                                cs.restore_pending_messages(cp.outbound_pending.clone());
                                cs.restore_system_appends(cp.system_appends.clone());
                                cs.set_verbosity_level(cp.verbosity_level);
                                // Restore communication config for spawned sessions.
                                if let Some(ref comm_config) = cp.communication_config {
                                    cs.set_communication_config(comm_config.clone());
                                }
                                // Restore transcript from checkpoint.
                                if !cp.pending_messages.is_empty() {
                                    cs.apply_transcript_op(
                                        TranscriptOp::Rewrite,
                                        cp.pending_messages.clone(),
                                    );
                                }
                            }
                        }

                        // Inject recovery notifications and tool failure results
                        // from checkpoint.
                        if let Some(ref notification) = cp.recovery_notification {
                            let cs = self.conversation_sessions.read().await;
                            if let Some(cs) = cs.get(&archived_id) {
                                let mut cs = cs.write().await;
                                cs.inject_system_message(notification.clone());
                                for failure in &cp.pending_tool_failures {
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
                                    session_key = %session_key,
                                    session_id = %archived_id,
                                    routing_key = %routing_key,
                                    "injected recovery notification and {} tool failure(s)",
                                    cp.pending_tool_failures.len()
                                );
                            }
                        }

                        // Create Session entry
                        {
                            let mut sessions = self.sessions.write().await;
                            if !sessions.contains_key(&archived_id) {
                                sessions.insert(
                                    archived_id.clone(),
                                    super::session_helpers::create_new_session(
                                        &archived_id,
                                        message,
                                        channel,
                                    ),
                                );
                            }
                        }

                        // Save checkpoint with updated thread_id
                        let mut cp = cp;
                        cp.thread_id = message.thread_id.clone();
                        if let Err(e) = cm.save_raw(&cp).await {
                            warn!(
                                session_key = %session_key,
                                session_id = %archived_id,
                                routing_key = %routing_key,
                                error = %e,
                                "failed to save checkpoint after restore"
                            );
                        }
                    }
                }
                // Re-register routing_key so subsequent lookups find
                // the restored session.
                {
                    let mut registry = self.key_registry.write().await;
                    registry.insert(routing_key.clone(), archived_id.clone());
                }

                self.update_checkpoint_thread_id(&archived_id, &message.thread_id)
                    .await;
                info!(
                    session_key = %session_key,
                    session_id = %archived_id,
                    routing_key = %routing_key,
                    "SQLite archived check: found and restored archived session"
                );
                return Ok(archived_id);
            }
        }
        let session_id = session_helpers::generate_session_id(&message.to);

        // Write to key_registry using routing_key (no timestamps)
        {
            let mut registry = self.key_registry.write().await;
            registry.insert(routing_key.to_string(), session_id.clone());
        }
        // Build system prompt
        let agent_id = message.to.clone();
        // Compute workdir: prefer per-agent workspace from AgentRegistry,
        // fall back to global workspace_dir.
        let workdir_path = if let Some(per_agent_ws) = self.query_agent_workspace(&agent_id).await {
            workspace::ensure_workspace_dir(&per_agent_ws, &message.to, &message.from).map_err(
                |e| ProcessError::ChainFailed(format!("workspace creation failed: {}", e)),
            )?
        } else if let Some(ref workspace_dir) = self.workspace_dir {
            workspace::ensure_workspace_dir(workspace_dir, &message.to, &message.from).map_err(
                |e| ProcessError::ChainFailed(format!("workspace creation failed: {}", e)),
            )?
        } else {
            PathBuf::from("/tmp")
        };

        // Create ConversationSession
        let mut conv_session =
            ConversationSession::new(session_id.clone(), "default".to_string(), workdir_path)
                .with_system_prompt("")
                .with_reasoning_level(self.default_reasoning_level);
        self.apply_default_cache_break_thresholds(&mut conv_session);
        // Wire shutdown handle for busy-count tracking.
        if let Some(sh) = self.get_shutdown_handle().await {
            conv_session.set_shutdown_handle(sh);
        }
        // Inject LLM caller and system prompt builder for delegation.
        let agent_hooks = self
            .get_agent_config(&agent_id)
            .await
            .map(|c| c.hooks)
            .unwrap_or_default();
        if let Some(caller) = self.get_llm_caller().await {
            conv_session.set_llm_caller(caller.clone());
            conv_session.init_health_checker(caller, agent_hooks);
        }
        if let Some(builder) = self.get_system_prompt_builder().await {
            conv_session.set_system_prompt_builder(builder);
        }
        conv_session.set_prompt_overrides(self.get_prompt_overrides().await);
        // Inject dynamic prompt builder for per-request dynamic-layer
        // injection (ChannelContext, etc.).
        if let Some(dpb) = self.get_dynamic_prompt_builder().await {
            conv_session.set_dynamic_prompt_builder(dpb);
        }
        // Inject skill listing provider and agent skills.
        self.wire_skill_listing_deps(&mut conv_session, &agent_id)
            .await;
        // Query bootstrap mode from AgentRegistry and cache.
        let bootstrap_mode = self
            .query_agent_bootstrap_mode(&agent_id)
            .await
            .unwrap_or(BootstrapMode::Full);
        conv_session = conv_session.with_bootstrap_mode(bootstrap_mode);
        // Build initial system prompt via session's own builder.
        info!(
            session_id = %session_id,
            agent_id = %agent_id,
            event = "session_injection",
            trigger = "new_session",
            "injecting full session deps for new session"
        );
        conv_session
            .rebuild_system_prompt(&session_id, &agent_id, Some(bootstrap_mode))
            .await;
        // Inject snapshot meta store for persistence.
        self.inject_snapshot_meta_store(&session_id, &mut conv_session)
            .await;
        // Inject checkpoint storage for pending-operation persistence.
        self.inject_checkpoint_storage(&mut conv_session).await;
        // Apply session config (git_status switch).
        if let Some(cfg) = self.get_session_config_for_agent(&agent_id).await {
            conv_session.set_git_status(cfg.is_git_status_enabled);
        }
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
        if let Some(cm) = self.checkpoint_manager.read().await.as_ref() {
            if let Err(e) = cm.save_raw(&cp).await {
                warn!(
                    session_key = %session_key,
                    session_id = %session_id,
                    routing_key = %routing_key,
                    error = %e,
                    "failed to save new session checkpoint"
                );
            }
        }

        info!(
            session_key = %session_key,
            session_id = %session_id,
            routing_key = %routing_key,
            "created new session"
        );
        Ok(session_id)
    }

    /// Bounded poll: wait for a session's checkpoint status to become Archived.
    ///
    /// Polls `cm.load(session_id)` every 500 ms for up to 5 s.
    /// Returns `true` if status reached `Archived`, `false` on timeout.
    ///
    /// Used by both registry-hit migrating handling (Step 1.1) and
    /// registry-miss migrating handling (Step 1.4) to avoid code duplication.
    async fn wait_for_archive_completion(
        cm: &CheckpointManager<dyn PersistenceService>,
        session_id: &str,
    ) -> bool {
        // Immediate check before first poll: if archive completed
        // before we even start sleeping, return right away.
        if let Ok(Some(cp)) = cm.load(session_id).await {
            if cp.status == SessionStatus::Archived {
                return true;
            }
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            match cm.load(session_id).await {
                Ok(Some(refreshed)) if refreshed.status == SessionStatus::Archived => {
                    return true;
                }
                _ if tokio::time::Instant::now() >= deadline => {
                    return false;
                }
                _ => {}
            }
        }
    }

    /// Rebuild system prompt for an archived session that already has a
    /// [`ConversationSession`] in memory (`needs_conv = false`).
    ///
    /// Extracted from Path 2 and Path 3 of [`Self::resolve`] to avoid
    /// duplicating the rebuild logic. Performs the full injection chain
    /// (matching the new session path) so that dynamic prompt builder,
    /// skill listing, snapshot meta store, checkpoint storage, prompt
    /// overrides, and session config are all wired up.
    ///
    /// Lock-range optimised: clones the `Arc` under a read lock then
    /// releases it before acquiring the write lock on the inner session.
    async fn rebuild_archived_session_prompt(
        &self,
        session_id: &str,
        cp: &SessionCheckpoint,
        message: &Message,
    ) {
        let agent_id_for_rebuild = cp.agent_id.clone().unwrap_or_else(|| message.to.clone());
        let bootstrap_mode = self
            .query_agent_bootstrap_mode(&agent_id_for_rebuild)
            .await
            .unwrap_or(BootstrapMode::Full);
        let cs_arc = {
            let cs = self.conversation_sessions.read().await;
            cs.get(session_id).cloned()
        };
        if let Some(cs_arc) = cs_arc {
            let mut cs = cs_arc.write().await;
            // Inject system prompt builder (was already present).
            if let Some(builder) = self.get_system_prompt_builder().await {
                cs.set_system_prompt_builder(builder);
            }
            // Inject prompt overrides (missing — added for parity with new session path).
            cs.set_prompt_overrides(self.get_prompt_overrides().await);
            // Inject dynamic prompt builder for per-request dynamic-layer injection.
            if let Some(dpb) = self.get_dynamic_prompt_builder().await {
                cs.set_dynamic_prompt_builder(dpb);
            }
            // Inject skill listing provider and agent-level skills whitelist.
            self.wire_skill_listing_deps(&mut cs, &agent_id_for_rebuild)
                .await;
            // Cache bootstrap mode on the session (was only queried, not cached).
            *cs = cs.clone().with_bootstrap_mode(bootstrap_mode);
            // Rebuild the system prompt (existing behavior).
            cs.rebuild_system_prompt(session_id, &agent_id_for_rebuild, Some(bootstrap_mode))
                .await;
            // Inject snapshot meta store for persistence.
            self.inject_snapshot_meta_store(session_id, &mut cs).await;
            // Inject checkpoint storage for pending-operation persistence.
            self.inject_checkpoint_storage(&mut cs).await;
            // Apply session config (git_status switch).
            if let Some(cfg) = self
                .get_session_config_for_agent(&agent_id_for_rebuild)
                .await
            {
                cs.set_git_status(cfg.is_git_status_enabled);
            }
        }
    }

    /// Wire skill listing provider and agent-level skills whitelist
    /// into a [`ConversationSession`]. Helper to avoid duplicating
    /// this block across resolve/recovery paths.
    pub(crate) async fn wire_skill_listing_deps(
        &self,
        conv: &mut ConversationSession,
        agent_id: &str,
    ) {
        if let Some(provider) = self.get_skill_listing_provider().await {
            conv.set_skill_listing_provider(provider);
        }
        if let Some(config) = self.get_agent_config(agent_id).await {
            if let Some(skills) = config.effective_skills() {
                conv.set_agent_skills(skills);
            }
        }
    }
}
