//! SessionManager - extracted session management from Gateway
//!
//! Responsible for session lifecycle: lookup, creation, restoration.
//! On daemon shutdown, `flush_all()` serializes all active sessions to the persistence backend.

use crate::shutdown_handle::ShutdownHandle;
use crate::sweeper::ActiveSessionQuery;
use crate::{compute_session_key, GatewayConfig, Message, Session};
use async_trait::async_trait;
use closeclaw_common::processor::ProcessError;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_common::tool_session::ToolSession;
use closeclaw_common::IMPlugin;
use closeclaw_common::{
    CacheBreakThresholds, DynamicPromptBuilder, LlmCaller, PromptOverrides,
    SessionActivityDimensions, SkillListingProvider, SkillRegistryQuery, SystemPromptBuilder,
    ToolRegistryQuery,
};
use closeclaw_config::manager::{ConfigManager, ConfigSnapshot};
use closeclaw_config::session::SessionConfigProvider;
use closeclaw_config::ConfigSection;
use closeclaw_session::checkpoint_manager::CheckpointManager;
use closeclaw_session::llm_session::{ChatSession, ConversationSession};
use closeclaw_session::persistence::{
    PendingMessage, PersistenceError, PersistenceService, ReasoningLevel, SessionCheckpoint,
    SessionStatus,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::warn;

mod announce;
mod channel;
pub mod communication;
pub(crate) mod compact;
mod compaction_helpers;
mod consistency_check;
mod key_registry;
mod ops_impl;
mod recovery_injection;
pub mod register_tools;
mod resolve;
mod session_helpers;
mod session_lookup_impl;
mod setters;
mod spawn;
pub mod spawn_adapter;
pub mod spawn_controller;
pub mod spawn_reclaim_gc;
pub mod stop;
mod stop_graceful;
mod sweep_target;
mod yield_timeout;
use closeclaw_session::spawn::SpawnTree;
pub use spawn::{ChildSessionConfig, ChildSessionInfo, ChildSessionStatus, SpawnMode};
pub use spawn_controller::SpawnController;
/// SessionManager holds all session state previously belonging to Gateway.
/// It provides find_or_create to lookup or create a session by channel + message.
pub struct SessionManager {
    /// Active sessions: session_id -> Session
    pub sessions: RwLock<HashMap<String, Session>>,
    /// Persistence coordination layer (cache + storage backend)
    checkpoint_manager: RwLock<Option<Arc<CheckpointManager<dyn PersistenceService>>>>,
    /// IM adapters for sending notifications during restoration
    adapters: RwLock<HashMap<String, Arc<dyn IMPlugin>>>,
    /// Per-session ConversationSession for llm_busy and outbound_pending management
    pub conversation_sessions: RwLock<HashMap<String, Arc<RwLock<ConversationSession>>>>,
    /// Workspace directory for bootstrap file loading (None means no workspace)
    workspace_dir: Option<PathBuf>,
    /// Tool registry for building system prompt ToolsSection
    tool_registry: RwLock<Option<Arc<dyn ToolRegistryQuery>>>,
    /// Skill registry for querying available skills (listing generation)
    skill_registry: RwLock<Option<Arc<dyn SkillRegistryQuery>>>,
    /// Default reasoning level for new sessions
    default_reasoning_level: ReasoningLevel,
    /// Default cache break detection thresholds for new sessions
    default_cache_break_thresholds: std::sync::RwLock<Option<CacheBreakThresholds>>,
    /// Priority prompt overrides (checked at request time, not session creation).
    prompt_overrides: RwLock<Option<PromptOverrides>>,
    /// System prompt builder (trait object) for rebuilding prompts.
    system_prompt_builder: RwLock<Option<Arc<dyn SystemPromptBuilder>>>,
    /// LLM caller injected by Gateway for delegating LLM requests.
    /// Set via [`set_llm_caller`](Self::set_llm_caller) after construction.
    llm_caller: RwLock<Option<Arc<dyn LlmCaller>>>,
    /// Dynamic prompt builder for per-request dynamic-layer injection.
    /// Injected by daemon (composition root) so gateway avoids depending
    /// on `closeclaw-system-prompt` directly.
    dynamic_prompt_builder: RwLock<Option<Arc<dyn DynamicPromptBuilder>>>,
    /// Children tracking table: parent_session_id → list of child sessions.
    pub(crate) children: RwLock<SpawnTree>,
    /// Channel → active session_id mapping.
    /// Updated by `force_new_for_channel` so subsequent `find_or_create`
    /// calls route to the latest session for a channel.
    channel_active_sessions: RwLock<HashMap<String, String>>,
    /// session_key → session_id mapping, rebuilt from persistence at startup.
    /// Updated by `resolve()` on new session creation and by
    /// `force_new_for_channel`.
    key_registry: RwLock<HashMap<String, String>>,
    /// Config manager for looking up agent-level tool/skill filtering.
    config_manager: RwLock<Option<Arc<ConfigManager>>>,
    /// Agent registry for looking up resolved agent configs (design-doc query layer).
    agent_registry: RwLock<Option<Arc<dyn closeclaw_agent::AgentRegistryQuery>>>,
    /// Latest config snapshot; swapped atomically on each hot-reload.
    /// The old snapshot is released when all Arc references are dropped.
    config_snapshot: RwLock<Option<ConfigSnapshot>>,
    /// Shutdown handle for busy-count tracking during drain.
    shutdown_handle: RwLock<Option<Arc<ShutdownHandle>>>,
    /// Pending restore notifications: session_id → (chat_id, Option<custom_message>).
    /// Populated by `try_restore_archived_session_inner` (None → default message)
    /// or by `resolve()` migrating path (Some("⏳ ...")) when a session needs
    /// archiving notification before restore.
    /// Consumed by `take_restore_notification` after Gateway sends the outbound message.
    pending_restore_notifications: RwLock<HashMap<String, (String, Option<String>)>>,
    /// Pending workflow blocked notifications: session_id → notification message.
    /// Populated by `drain_workflow_notification` when a workflow enters blocked state,
    /// consumed by `take_workflow_notification` after Gateway sends it through the outbound chain.
    pending_workflow_notifications: RwLock<HashMap<String, String>>,
    /// Optional callback to invalidate the static-layer section cache.
    /// Injected by the daemon (composition root) so gateway avoids
    /// depending on `closeclaw-system-prompt` directly.
    cache_invalidator: RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Channel sender for notifying the DreamingScheduler about completed
    /// sub-agent sessions, enabling immediate mining (design doc §触发 1).
    mining_notify_tx: std::sync::RwLock<Option<tokio::sync::mpsc::Sender<String>>>,
    /// Background task manager for draining completion notifications
    /// and cleaning up finished task output files.
    task_manager: RwLock<Option<Arc<dyn closeclaw_tasks::TaskManager>>>,
    /// Per-session runtime snapshot managers for transcript rollback.
    /// Per-agent mutexes for serializing resolve() requests.
    /// Keyed by agent_id. Ensures the same agent's lookup/restore/create
    /// operations are serialized while different agents run in parallel.
    agent_locks: Arc<RwLock<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    /// Per-session yield timeout handles (keyed by session_id).
    /// Aborted on normal recovery or timeout.
    yield_timeout_handles: RwLock<HashMap<String, tokio::task::JoinHandle<()>>>,
    /// Per-session yield warning timeout handles (keyed by session_id).
    /// Aborted on normal recovery, timeout, or when the warning fires.
    yield_warning_handles: RwLock<HashMap<String, tokio::task::JoinHandle<()>>>,
    /// Cached config directory path, set by [`set_config_manager`](Self::set_config_manager).
    /// Used by [`SpawnCreationContext::config_dir`] to return a stable `&Path` reference.
    /// Immutable after initialization (set once, read many).
    config_dir: std::sync::OnceLock<PathBuf>,
    /// Skill listing provider for per-turn skill injection.
    /// Injected by daemon (composition root) so each LLM turn can
    /// prepend a tool-role attachment with the agent's skill listing.
    skill_listing_provider: RwLock<Option<Arc<dyn SkillListingProvider>>>,
    /// Output channel for sending LLM responses to the user.
    /// Set via [`set_output_tx`](Self::set_output_tx) after construction.
    /// Used by [`drain_pending_for_session`](super::announce::SessionManager::drain_pending_for_session)
    /// to send responses to the user during yield recovery.
    output_tx: RwLock<Option<crate::OutputTx>>,
    /// Callback for registering session tools into a tool registry.
    /// Injected by daemon (composition root) so that [`register_tools`](Self::register_tools)
    /// can delegate tool creation to the tools crate without gateway depending on it.
    tool_register_fn: RwLock<Option<register_tools::ToolRegisterFn>>,
    /// Back-reference to Gateway for outbound dispatch (Weak to avoid cycle).
    gateway_ref: RwLock<Option<std::sync::Weak<crate::Gateway>>>,
    /// Timestamp (Unix epoch seconds) of the last consistency scan.
    /// `None` means no scan has been performed yet; the first periodic
    /// incremental scan will use 0 (equivalent to full scan) until the
    /// startup full scan sets this value.
    last_consistency_check_time: std::sync::Mutex<Option<i64>>,
    /// Injected SessionConfigProvider for per-agent idle/purge thresholds.
    /// When set, `get_session_config_for_agent` uses this directly instead
    /// of going through ConfigManager, following the layer-2 component
    /// extraction pattern.
    session_config_provider: RwLock<Option<Arc<dyn SessionConfigProvider>>>,
    /// In-memory tracker for active-searcher sub-sessions.
    /// Lightweight identity + lifecycle tracking (Running → Injected /
    /// NoResult / Abandoned), no persistence.
    pub(crate) searcher_sessions:
        Arc<closeclaw_session::active_searcher_session::SearcherSessionTracker>,
}

impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager").finish_non_exhaustive()
    }
}

impl SessionManager {
    /// Create a new SessionManager with the given config, optional storage,
    /// workspace directory and bootstrap mode.
    pub fn new(
        _config: &GatewayConfig,
        storage: Option<Arc<dyn PersistenceService>>,
        workspace_dir: Option<PathBuf>,
        default_reasoning_level: ReasoningLevel,
    ) -> Self {
        let cm = storage.map(|s| Arc::new(CheckpointManager::new(s)));
        Self {
            sessions: RwLock::new(HashMap::new()),
            checkpoint_manager: RwLock::new(cm),
            adapters: RwLock::new(HashMap::new()),
            conversation_sessions: RwLock::new(HashMap::new()),
            workspace_dir,
            tool_registry: RwLock::new(None),
            skill_registry: RwLock::new(None),
            default_reasoning_level,
            default_cache_break_thresholds: std::sync::RwLock::new(None),
            prompt_overrides: RwLock::new(None),
            system_prompt_builder: RwLock::new(None),
            llm_caller: RwLock::new(None),
            dynamic_prompt_builder: RwLock::new(None),
            children: RwLock::new(SpawnTree::new()),
            channel_active_sessions: RwLock::new(HashMap::new()),
            key_registry: RwLock::new(HashMap::new()),
            config_manager: RwLock::new(None),
            config_dir: std::sync::OnceLock::new(),
            agent_registry: RwLock::new(None),
            config_snapshot: RwLock::new(None),
            shutdown_handle: RwLock::new(None),
            pending_restore_notifications: RwLock::new(HashMap::new()),
            pending_workflow_notifications: RwLock::new(HashMap::new()),
            cache_invalidator: RwLock::new(None),
            mining_notify_tx: std::sync::RwLock::new(None),
            task_manager: RwLock::new(None),
            agent_locks: Arc::new(RwLock::new(HashMap::new())),
            skill_listing_provider: RwLock::new(None),
            yield_timeout_handles: RwLock::new(HashMap::new()),
            yield_warning_handles: RwLock::new(HashMap::new()),
            output_tx: RwLock::new(None),
            tool_register_fn: RwLock::new(None),
            gateway_ref: RwLock::new(None),
            last_consistency_check_time: std::sync::Mutex::new(None),
            session_config_provider: RwLock::new(None),
            searcher_sessions: Arc::new(
                closeclaw_session::active_searcher_session::SearcherSessionTracker::new(),
            ),
        }
    }

    /// Compute session key from channel, message and optional account_id.
    fn compute_session_key_local(
        &self,
        channel: &str,
        message: &Message,
        account_id: Option<&str>,
        timestamp_ms: i64,
    ) -> String {
        compute_session_key(
            channel,
            &message.from,
            &message.to,
            account_id,
            timestamp_ms,
        )
    }

    /// Strip the `{timestamp_ms}-` prefix from a session key, returning
    /// only the sha256 hash portion.
    ///
    /// Given a key in the format `{ts}-{sha256_hex}`, this returns
    /// `{sha256_hex}`.  Used by both `resolve` and `rebuild_key_registry`.
    #[allow(dead_code)]
    pub(crate) fn strip_timestamp_from_session_key(key: &str) -> &str {
        key.find('-').map(|i| &key[i + 1..]).unwrap_or(key)
    }

    /// Compute a stable routing key from message routing fields.
    ///
    /// Format: `sha256("{account_id}:{channel}:{from}:{to}")`.
    /// This is stable across timestamps and matches the format used by
    /// `rebuild_key_registry()`.
    pub(crate) fn compute_routing_key(
        channel: &str,
        message: &Message,
        account_id: Option<&str>,
    ) -> String {
        let acc = account_id.unwrap_or("default");
        let routing_fields = format!("{}:{}:{}:{}", acc, channel, message.from, message.to);
        let hash = Sha256::digest(routing_fields.as_bytes());
        format!("{:x}", hash)
    }

    /// Attempt to restore an archived session.
    /// Returns true iff restoration was attempted and succeeded.
    async fn try_restore_archived_session(&self, session_id: &str, channel: &str) -> bool {
        let storage_arc = {
            let cm_guard = self.checkpoint_manager.read().await;
            match cm_guard.as_ref() {
                Some(cm) => Arc::clone(cm.storage_arc()),
                None => return false,
            }
        };
        let result =
            session_helpers::try_restore_archived_session_inner(&storage_arc, session_id, channel)
                .await;
        // Store the notification chat_id for Gateway to send via outbound chain.
        if let Some(chat_id) = result.notification_chat_id {
            let mut pending = self.pending_restore_notifications.write().await;
            pending.insert(session_id.to_string(), (chat_id, None));
        }
        result.restored
    }

    /// Take the pending restore notification for a session.
    /// Returns `(chat_id, Option<custom_message>)` if a restore notification
    /// is pending. When `custom_message` is `None`, the caller uses the
    /// default "正在恢复会话..." text; otherwise the custom message is used.
    pub async fn take_restore_notification(
        &self,
        session_id: &str,
    ) -> Option<(String, Option<String>)> {
        let mut pending = self.pending_restore_notifications.write().await;
        pending.remove(session_id)
    }

    /// Queue a workflow blocked notification for a session.
    /// The notification is consumed by [`take_workflow_notification`] after
    /// the Gateway sends it through the outbound chain.
    pub async fn queue_workflow_notification(&self, session_id: &str, message: String) {
        let mut pending = self.pending_workflow_notifications.write().await;
        pending.insert(session_id.to_string(), message);
    }

    /// Take the pending workflow blocked notification for a session.
    /// Returns the notification message text if one is pending.
    pub async fn take_workflow_notification(&self, session_id: &str) -> Option<String> {
        let mut pending = self.pending_workflow_notifications.write().await;
        pending.remove(session_id)
    }

    /// Update thread_id and reply_ref in a session's checkpoint.
    async fn update_checkpoint_fields(
        &self,
        session_id: &str,
        thread_id: &Option<String>,
        reply_ref: &Option<String>,
    ) {
        if let Some(cm) = self.checkpoint_manager.read().await.as_ref() {
            session_helpers::update_checkpoint_fields(
                cm.as_ref(),
                session_id,
                thread_id,
                reply_ref,
            )
            .await;
        }
    }

    /// Update `last_user_activity_at` and `last_message_at` for a user message.
    ///
    /// Called from the message handler when a user message arrives.
    /// Updates both timestamps to now and persists the checkpoint.
    pub async fn update_checkpoint_user_activity(&self, session_id: &str) {
        let cm_guard = self.checkpoint_manager.read().await;
        let Some(cm) = cm_guard.as_ref() else {
            warn!(
                session_id = %session_id,
                "storage not available, skipping user activity update"
            );
            return;
        };
        session_helpers::update_checkpoint_user_activity(cm.as_ref(), session_id).await;
    }

    /// Find or create a session for the given channel and message.
    ///
    /// 1. Compute session_id from channel + message + account_id
    /// 2. If session exists in active table → return it
    /// 3. If not, try to restore from archived storage
    /// 4. If restoration succeeds → return restored session
    /// 5. Otherwise → create and register a new session
    ///
    /// **Caller responsibility**: `message.to` is passed as the `agent_id`
    /// parameter to [`resolve`](Self::resolve). Callers with access to
    /// `bot_agent_bindings` (e.g. Gateway) must call
    /// [`Gateway::resolve_agent_id`] before constructing the `Message` so
    /// that `message.to` carries the resolved agent_id per the design doc:
    /// "Gateway 根据配置定义的机器人→Agent 绑定确定对应的 Agent".
    ///
    /// Returns the session_id string on success.
    pub async fn find_or_create(
        &self,
        channel: &str,
        message: &Message,
        account_id: Option<&str>,
    ) -> Result<String, ProcessError> {
        // Channel-level override: if a channel has an active session
        // (e.g. from /new), route directly to it.
        let channel_override = {
            let channel_map = self.channel_active_sessions.read().await;
            if let Some(active_id) = channel_map.get(channel) {
                let sessions = self.sessions.read().await;
                if sessions.contains_key(active_id) {
                    Some(active_id.clone())
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(active_id) = &channel_override {
            self.update_checkpoint_fields(active_id, &message.thread_id, &message.reply_ref)
                .await;
            return Ok(active_id.clone());
        }

        let session_key =
            self.compute_session_key_local(channel, message, account_id, message.timestamp);
        let session_id = self
            .resolve(&session_key, channel, message, account_id, &message.to)
            .await?;
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

    /// Get all active sessions.
    pub async fn get_all_sessions(&self) -> Vec<Session> {
        let sessions = self.sessions.read().await;
        sessions.values().cloned().collect()
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

    /// Check whether the model for the given session supports image input.
    /// Fail-open on any lookup failure.
    pub async fn session_model_supports_images(&self, session_id: &str) -> bool {
        let aid = match self.get_chat_id(session_id).await {
            Some(id) => id,
            None => return true,
        };
        let cfg = match self.get_agent_config(&aid).await {
            Some(c) => c,
            None => return true,
        };
        let name = cfg.model.map(|m| m.primary).unwrap_or_default();
        if name.is_empty() {
            return true;
        }
        crate::media_routing::model_supports_images(&name)
    }

    /// Force a WAL checkpoint via the persistence backend.
    ///
    /// Should be called after `flush_all` in Phase 4 to ensure all
    /// session writes are durable on disk.
    pub async fn sync_storage(&self) -> Result<(), PersistenceError> {
        let cm = self.checkpoint_manager.read().await;
        let Some(cm) = cm.as_ref() else {
            return Ok(());
        };
        cm.storage().sync().await
    }

    /// Explicitly close the storage backend and release resources.
    ///
    /// Called during Phase 6 of daemon shutdown. Releases persistent
    /// connections or file handles held by the storage backend.
    pub async fn close_storage(&self) -> Result<(), PersistenceError> {
        let cm = self.checkpoint_manager.read().await;
        let Some(cm) = cm.as_ref() else {
            return Ok(());
        };
        cm.storage().close().await
    }

    /// Flush all active sessions to persistence.
    /// Returns the number of sessions successfully saved.
    pub async fn flush_all(&self, _mode: ShutdownMode) -> Result<usize, PersistenceError> {
        let cm = self.checkpoint_manager.read().await;
        let Some(cm) = cm.as_ref() else {
            return Ok(0);
        };
        let sessions = self.sessions.read().await;
        // Collect session ids first to avoid holding sessions lock across I/O
        let session_ids: Vec<String> = sessions.keys().cloned().collect();
        drop(sessions);

        // Collect pending messages and system_appends using async RwLock read.
        let mut pending_map: HashMap<String, Vec<PendingMessage>> = HashMap::new();
        let mut appends_map: HashMap<String, Vec<String>> = HashMap::new();
        {
            let conv_sessions = self.conversation_sessions.read().await;
            for sid in &session_ids {
                if let Some(cs) = conv_sessions.get(sid) {
                    let cs = cs.read().await;
                    pending_map.insert(sid.clone(), cs.get_pending_messages());
                    appends_map.insert(sid.clone(), cs.user_system_appends().to_vec());
                }
            }
        } // Drop conv_sessions read lock before checkpoint persistence
        let sessions = self.sessions.read().await;
        let mut saved = 0;
        for (session_id, session) in sessions.iter() {
            let pending = pending_map.get(session_id).cloned().unwrap_or_default();
            // Load existing checkpoint to preserve fields like thread_id (Bug #904).
            let mut cp = match cm.load(session_id).await {
                Ok(Some(mut cp)) => {
                    // Update fields from active session state
                    cp.status = SessionStatus::Active;
                    cp.platform = Some(session.channel.clone());
                    cp.peer_id = Some(session.agent_id.clone());
                    cp.agent_id = Some(session.agent_id.clone());
                    cp.outbound_pending = pending;
                    cp
                }
                // No existing checkpoint — create a fresh one
                _ => SessionCheckpoint::new(session_id.clone())
                    .with_status(SessionStatus::Active)
                    .with_platform(session.channel.clone())
                    .with_peer_id(session.agent_id.clone())
                    .with_agent_id(session.agent_id.clone())
                    .with_outbound_pending(pending),
            };
            // Sync per-session append-section list from ConversationSession
            // (issue #860: archived session restore preserves append content).
            if let Some(appends) = appends_map.get(session_id) {
                cp.system_appends = appends.clone();
            }
            if cm.save_raw(&cp).await.is_ok() {
                saved += 1;
            } else {
                warn!(session_id = %session_id, "failed to save session checkpoint");
            }
        }
        drop(sessions);

        // Phase 4 cleanup: remove sessions from tracking tables after
        // all checkpoint persistence is complete. This ensures the
        // "fallback persistence" path actually finds sessions that
        // were stopped in Phase 2 but not yet removed from tracking.
        for session_id in &session_ids {
            self.remove_session(session_id).await;
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

    /// Return the four-dimensional activity state of a session.
    ///
    /// Looks up the session in the in-memory `conversation_sessions` table
    /// and delegates to [`ConversationSession::activity_dimensions`] which
    /// reports the four independent activity dimensions:
    /// 1. **llm_active** — LLM in Requesting/Receiving state
    /// 2. **foreground_tool_active** — tool pending or executing in foreground
    /// 3. **background_tool_active** — tool executing in background
    /// 4. **child_active** — at least one child session is Running
    ///
    /// Returns a zeroed [`SessionActivityDimensions`] (all `false`) if the
    /// session does not exist in the in-memory table — a session that has
    /// been flushed or archived has no active work.
    pub async fn activity_dimensions(&self, session_id: &str) -> SessionActivityDimensions {
        let conv_sessions = self.conversation_sessions.read().await;
        match conv_sessions.get(session_id) {
            Some(cs) => {
                let cs = cs.read().await;
                cs.activity_dimensions()
            }
            None => SessionActivityDimensions::default(),
        }
    }

    /// Look up an existing session_id from routing fields without creating
    /// a new session. Returns `None` if no matching session exists.
    ///
    /// This avoids the side effects of [`resolve`] (session creation,
    /// checkpoint persistence, etc.) and is suitable for lightweight
    /// polling (e.g. streaming-completion wait).
    pub async fn lookup_session_from_routing(
        &self,
        channel: &str,
        from: &str,
        to: &str,
        account_id: Option<&str>,
    ) -> Option<String> {
        let msg = Message {
            id: String::new(),
            from: from.to_string(),
            to: to.to_string(),
            content: String::new(),
            channel: channel.to_string(),
            timestamp: 0,
            metadata: std::collections::HashMap::new(),
            thread_id: None,
            reply_ref: None,
            platform: None,
            dsl_result: None,
            content_blocks: None,
        };
        let routing_key = Self::compute_routing_key(channel, &msg, account_id);
        let session_id = {
            let registry = self.key_registry.read().await;
            registry.get(&routing_key).cloned()
        }?;
        // Verify the session is still active (exists in sessions map).
        let exists = {
            let sessions = self.sessions.read().await;
            sessions.contains_key(&session_id)
        };
        if exists {
            Some(session_id)
        } else {
            None
        }
    }
    /// Returns `true` if the session is in active Waiting (yielding).
    pub async fn is_session_yielding(&self, sid: &str) -> bool {
        if let Some(cs) = self.get_conversation_session(sid).await {
            return cs.read().await.is_waiting();
        }
        false
    }
    /// Trigger manual backgrounding for all foreground commands in a session.
    ///
    /// Sends a signal to every active foreground command (e.g. `BashTool`)
    /// telling it to move to background immediately. The command will be
    /// handed off to `bg_manager` and return a `ToolResult` with
    /// `backgroundedByUser: true`.
    ///
    /// Returns `Ok(true)` if the signal was fired.
    /// Returns `Err` if the session was not found or the lock could not
    /// be acquired.
    pub async fn trigger_manual_background(&self, session_id: &str) -> Result<bool, String> {
        let conv_sessions = self.conversation_sessions.read().await;
        let cs = conv_sessions
            .get(session_id)
            .ok_or_else(|| format!("session not found: {}", session_id))?;
        let cs = cs.read().await;
        cs.trigger_manual_background();
        Ok(true)
    }

    /// Push a pending message onto the queue for a given session.
    ///
    /// After pushing to the unified queue, persists the checkpoint so
    /// that the outbound message is registered as a pending operation.
    /// This ensures crash recovery can detect in-flight outbound messages.
    ///
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
        // Persist checkpoint so the outbound message is registered as a
        // pending operation (active write mode per design doc).
        cs.persist_pending_checkpoint()
            .await
            .map_err(|e| format!("checkpoint persist failed: {}", e))?;
        Ok(())
    }

    /// Set the active workflow run for a session and persist the checkpoint.
    pub async fn set_workflow_run(
        &self,
        session_id: &str,
        run: Option<closeclaw_workflow::run::WorkflowRun>,
    ) -> Result<(), String> {
        let conv_sessions = self.conversation_sessions.read().await;
        let cs = conv_sessions
            .get(session_id)
            .ok_or_else(|| format!("session not found: {}", session_id))?;
        let mut cs = cs.write().await;
        cs.set_workflow_run(run);
        cs.persist_pending_checkpoint()
            .await
            .map_err(|e| format!("checkpoint persist failed: {}", e))?;
        Ok(())
    }

    /// Pop the oldest pending message for a given session.
    /// Returns None if the session does not exist or the queue is empty.
    pub async fn pop_pending_message(&self, session_id: &str) -> Option<PendingMessage> {
        let conv_sessions = self.conversation_sessions.read().await;
        let cs = conv_sessions.get(session_id)?;
        let mut cs = cs.write().await;
        cs.pop_pending()
    }

    /// Get the thread_id for a session from its checkpoint.
    /// Returns None if the session has no thread_id or the storage is not available.
    pub async fn get_thread_id(&self, session_id: &str) -> Option<String> {
        let cm = self.checkpoint_manager.read().await;
        let cm = cm.as_ref()?;
        match cm.load(session_id).await {
            Ok(Some(cp)) => cp.thread_id,
            _ => None,
        }
    }

    pub async fn get_reply_ref(&self, session_id: &str) -> Option<String> {
        let cm = self.checkpoint_manager.read().await;
        let cm = cm.as_ref()?;
        match cm.load(session_id).await {
            Ok(Some(cp)) => cp.reply_ref,
            _ => None,
        }
    }

    /// Get the sender_id (user ID) for a session from its checkpoint.
    pub async fn get_sender_id(&self, session_id: &str) -> Option<String> {
        let cm = self.checkpoint_manager.read().await;
        let cm = cm.as_ref()?;
        match cm.load(session_id).await {
            Ok(Some(cp)) => cp.sender_id,
            _ => None,
        }
    }

    /// Get the plan_state from the session checkpoint.
    /// Returns None if the session has no plan_state or storage is unavailable.
    pub async fn get_plan_state(&self, session_id: &str) -> Option<closeclaw_common::PlanState> {
        let cm = self.checkpoint_manager.read().await;
        let cm = cm.as_ref()?;
        match cm.load(session_id).await {
            Ok(Some(cp)) => cp.plan_state,
            _ => None,
        }
    }
    /// Set plan_state on the session checkpoint.
    /// Writes the checkpoint first; updates ConversationSession on success.
    pub async fn set_plan_state(&self, session_id: &str, plan_state: closeclaw_common::PlanState) {
        // Write checkpoint first; skip ConversationSession if checkpoint fails.
        let plan_file_path = if plan_state.plan_file_path.is_empty() {
            None
        } else {
            Some(plan_state.plan_file_path.clone())
        };
        let cm = {
            let guard = self.checkpoint_manager.read().await;
            match guard.as_ref() {
                Some(cm) => Arc::clone(cm),
                None => return,
            }
        };
        let mut cp = match cm.load(session_id).await {
            Ok(Some(cp)) => cp,
            _ => return,
        };
        cp.plan_state = Some(plan_state);
        if let Err(e) = cm.save_raw(&cp).await {
            tracing::warn!(session_id = %session_id, "failed to save plan_state: {}", e);
            return;
        }
        // Sync plan_file_path to ConversationSession for Auto Mode injection.
        if let Some(cs) = self.get_conversation_session(session_id).await {
            let mut cs = cs.write().await;
            cs.set_plan_file_path(plan_file_path);
        }
    }
    /// Clear plan state when exiting Plan Mode (e.g. `/mode normal`, `/auto`).
    pub async fn clear_plan_state(&self, session_id: &str) {
        // Clone Arc before dropping the guard to avoid holding it across await.
        let cm = {
            let guard = self.checkpoint_manager.read().await;
            guard.as_ref().map(Arc::clone)
        };
        if let Some(cm) = cm {
            if session_helpers::clear_plan_state_checkpoint(&cm, session_id)
                .await
                .is_err()
            {
                warn!(session_id = %session_id, "failed to clear plan_state checkpoint");
                return;
            }
        }
        if let Some(cs) = self.get_conversation_session(session_id).await {
            let mut cs = cs.write().await;
            cs.set_plan_file_path(None);
        }
    }
    /// Rebuild the system prompt for a session, delegating to
    /// `ConversationSession::rebuild_system_prompt`.
    pub async fn rebuild_system_prompt_for_session(&self, session_id: &str) {
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
        let mut cs = cs.write().await;
        let cached_mode = cs.bootstrap_mode();
        cs.rebuild_system_prompt(session_id, &agent_id, Some(cached_mode))
            .await;
    }

    /// Notify all active sessions that a configuration section has been
    /// updated. Rebuilds each session's cached system prompt so the next
    /// LLM request picks up the latest config values.
    pub async fn notify_config_changed(&self, section: ConfigSection, snapshot: ConfigSnapshot) {
        tracing::info!(
            section = %section,
            snapshot_sections = snapshot.len(),
            "session_manager: config change notification received"
        );
        // Swap in the new config snapshot (old one auto-released).
        self.swap_config_snapshot(snapshot).await;
        let session_ids: Vec<String> = {
            let sessions = self.sessions.read().await;
            sessions.keys().cloned().collect()
        };
        for session_id in &session_ids {
            tracing::debug!(
                session_id = %session_id,
                section = %section,
                "rebuilding system prompt for session after config change"
            );
            self.rebuild_system_prompt_for_session(session_id).await;
        }
        tracing::info!(
            section = %section,
            session_count = session_ids.len(),
            "session_manager: config change notification sent to sessions"
        );
    }
}

#[async_trait]
impl ActiveSessionQuery for SessionManager {
    /// Return the four-dimensional activity state of the session.
    ///
    /// Delegates to [`SessionManager::activity_dimensions`].
    async fn activity_dimensions(&self, session_id: &str) -> SessionActivityDimensions {
        self.activity_dimensions(session_id).await
    }
}

#[cfg(test)]
impl SessionManager {
    /// Initialize `config_dir` from a temporary directory.
    ///
    /// Used by tests that create `SessionManager` without a real
    /// `ConfigManager`, so `SpawnCreationContext::config_dir()` does
    /// not panic.
    pub fn set_config_dir_for_testing(&self, config_dir: &std::path::Path) {
        let _ = self.config_dir.set(config_dir.to_path_buf());
    }
}

#[cfg(test)]
mod active_write_outbound_tests;
#[cfg(test)]
mod announce_dedup_tests;
#[cfg(test)]
mod announce_drain_outbound_tests;
#[cfg(test)]
mod announce_prefix_tests;
#[cfg(test)]
mod announce_priority_tests;
#[cfg(test)]
mod announce_reclaim_tests;
#[cfg(test)]
mod announce_tests;
#[cfg(test)]
mod bug904_tests;
#[cfg(test)]
mod consistency_check_tests;
#[cfg(test)]
mod flush_tests;
#[cfg(test)]
mod gap3_graceful_timeout_tests;
#[cfg(test)]
mod gap3_priority_injection_tests;
#[cfg(test)]
mod gap3_status_text_tests;
#[cfg(test)]
mod gap3_termination_notification_tests;
#[cfg(test)]
mod graceful_stop_tests;
#[cfg(test)]
mod is_active_tests;
#[cfg(test)]
mod key_registry_tests;
#[cfg(test)]
mod rebuild_spawn_tree_tests;
#[cfg(test)]
mod recovery_injection_tests;
#[cfg(test)]
mod register_tools_tests;
#[cfg(test)]
mod resolve_agent_id_tests;
#[cfg(test)]
mod resolve_archived_recovery_tests;
#[cfg(test)]
mod resolve_checkpoint_status_tests;
#[cfg(test)]
mod resolve_miss_migrating_tests;
#[cfg(test)]
mod resolve_rebuild_tests;
#[cfg(test)]
mod resolve_registry_migrating_tests;
#[cfg(test)]
mod resolve_registry_tests;
#[cfg(test)]
mod resolve_tests;
#[cfg(test)]
mod self_heal_tests;
#[cfg(test)]
mod session_helpers_tests;
#[cfg(test)]
mod setter_tests;
#[cfg(test)]
mod spawn_cascade_tests;
#[cfg(test)]
mod spawn_child_state_tests;
#[cfg(test)]
mod spawn_controller_boundary_tests;
#[cfg(test)]
mod spawn_controller_budget_tests;
#[cfg(test)]
mod spawn_controller_permission_tests;
#[cfg(test)]
mod spawn_controller_step14_tests;
#[cfg(test)]
mod spawn_controller_step15_tests;
#[cfg(test)]
mod spawn_controller_tests;
#[cfg(test)]
mod spawn_kill_persist_tests;
#[cfg(test)]
mod spawn_label_tests;
#[cfg(test)]
mod spawn_mode_inherit_tests;
#[cfg(test)]
mod spawn_reclaim_gc_tests;
#[cfg(test)]
mod spawn_tests;
#[cfg(test)]
mod spawn_tree_tests;
#[cfg(test)]
mod stop_queue_clear_tests;
#[cfg(test)]
mod stop_tests;
#[cfg(test)]
mod sweep_target_tests;
#[cfg(test)]
pub(crate) mod test_helpers;
#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
mod yield_recovery_tests;
#[cfg(test)]
mod yield_timeout_cyclic_tests;
#[cfg(test)]
mod yield_timeout_step13_tests;
#[cfg(test)]
mod yield_timeout_step16_tests;
#[cfg(test)]
mod yield_timeout_tests;
