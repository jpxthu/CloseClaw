//! SessionManager - extracted session management from Gateway
//!
//! Responsible for session lifecycle: lookup, creation, restoration.
//! On daemon shutdown, `flush_all()` serializes all active sessions to the persistence backend.

use crate::shutdown_handle::ShutdownHandle;
use crate::{compute_session_key, GatewayConfig, Message, Session};
use closeclaw_common::processor::ProcessError;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_common::IMPlugin;
use closeclaw_common::{
    DynamicPromptBuilder, LlmCaller, PromptOverrides, SkillRegistryQuery, SystemPromptBuilder,
    ToolRegistryQuery,
};
use closeclaw_config::manager::{ConfigManager, ConfigSnapshot};
use closeclaw_config::ConfigSection;
use closeclaw_session::bootstrap::loader::BootstrapMode;
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
mod compaction_helpers;
mod consistency_check;
mod key_registry;
mod recovery_injection;
mod resolve;
mod session_helpers;
mod spawn;
pub mod spawn_adapter;
pub mod spawn_controller;
pub mod stop;
mod stop_graceful;
mod yield_timeout;
use closeclaw_session::run_health::PersistenceMetaStore;
use closeclaw_session::spawn::SpawnTree;
pub use spawn::{ChildSessionInfo, SpawnMode};
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
    /// Skill registry for building system prompt SkillListingSection
    skill_registry: RwLock<Option<Arc<dyn SkillRegistryQuery>>>,
    /// Default reasoning level for new sessions
    default_reasoning_level: ReasoningLevel,
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
    /// Pending restore notifications: session_id → chat_id.
    /// Populated by `try_restore_archived_session_inner` when a session is restored,
    /// consumed by `take_restore_notification` after Gateway sends it through the outbound chain.
    pending_restore_notifications: RwLock<HashMap<String, String>>,
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
    /// Output channel for sending LLM responses to the user.
    /// Set via [`set_output_tx`](Self::set_output_tx) after construction.
    /// Used by [`drain_pending_for_session`](super::announce::SessionManager::drain_pending_for_session)
    /// to send responses to the user during yield recovery.
    output_tx: RwLock<Option<crate::OutputTx>>,
    /// Back-reference to Gateway for outbound dispatch (Weak to avoid cycle).
    gateway_ref: RwLock<Option<std::sync::Weak<crate::Gateway>>>,
    /// Timestamp (Unix epoch seconds) of the last consistency scan.
    /// `None` means no scan has been performed yet; the first periodic
    /// incremental scan will use 0 (equivalent to full scan) until the
    /// startup full scan sets this value.
    last_consistency_check_time: std::sync::Mutex<Option<i64>>,
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
            prompt_overrides: RwLock::new(None),
            system_prompt_builder: RwLock::new(None),
            llm_caller: RwLock::new(None),
            dynamic_prompt_builder: RwLock::new(None),
            children: RwLock::new(SpawnTree::new()),
            channel_active_sessions: RwLock::new(HashMap::new()),
            key_registry: RwLock::new(HashMap::new()),
            config_manager: RwLock::new(None),
            agent_registry: RwLock::new(None),
            config_snapshot: RwLock::new(None),
            shutdown_handle: RwLock::new(None),
            pending_restore_notifications: RwLock::new(HashMap::new()),
            cache_invalidator: RwLock::new(None),
            mining_notify_tx: std::sync::RwLock::new(None),
            task_manager: RwLock::new(None),
            agent_locks: Arc::new(RwLock::new(HashMap::new())),
            yield_timeout_handles: RwLock::new(HashMap::new()),
            output_tx: RwLock::new(None),
            gateway_ref: RwLock::new(None),
            last_consistency_check_time: std::sync::Mutex::new(None),
        }
    }

    /// Set the background task manager for notification drain and cleanup.
    pub async fn set_task_manager(&self, tm: Arc<dyn closeclaw_tasks::TaskManager>) {
        *self.task_manager.write().await = Some(tm);
    }

    /// Set the output channel for sending LLM responses to the user.
    ///
    /// When set, [`drain_pending_for_session`](super::announce::SessionManager::drain_pending_for_session)
    /// will send queued message responses through this channel so the
    /// user sees them after yield recovery.
    pub async fn set_output_tx(&self, tx: crate::OutputTx) {
        *self.output_tx.write().await = Some(tx);
    }

    /// Get a clone of the output channel, if set.
    pub async fn get_output_tx(&self) -> Option<crate::OutputTx> {
        self.output_tx.read().await.clone()
    }

    /// Set the Gateway back-reference for outbound message dispatch.
    pub async fn set_gateway_ref(&self, gw: Arc<crate::Gateway>) {
        *self.gateway_ref.write().await = Some(Arc::downgrade(&gw));
    }

    /// Get a strong reference to the Gateway, if still alive.
    pub async fn get_gateway_ref(&self) -> Option<Arc<crate::Gateway>> {
        self.gateway_ref
            .read()
            .await
            .as_ref()
            .and_then(|w| w.upgrade())
    }

    /// Get a clone of the task manager, if set.
    pub async fn get_task_manager(&self) -> Option<Arc<dyn closeclaw_tasks::TaskManager>> {
        self.task_manager.read().await.clone()
    }

    /// Attach a mining notify channel sender. When a run-mode sub-agent
    /// session completes, the sender emits the session ID so the
    /// DreamingScheduler can trigger mining immediately.
    pub fn set_mining_notify_tx(&self, tx: tokio::sync::mpsc::Sender<String>) {
        *self.mining_notify_tx.write().unwrap() = Some(tx);
    }

    /// Set the config manager for agent-level tool/skill filtering.
    pub async fn set_config_manager(&self, config_manager: Arc<ConfigManager>) {
        *self.config_manager.write().await = Some(config_manager);
    }

    /// Get the config manager reference (if set).
    pub async fn get_config_manager(&self) -> Option<Arc<ConfigManager>> {
        self.config_manager.read().await.clone()
    }

    /// Set the agent registry for resolved config lookups.
    pub async fn set_agent_registry(
        &self,
        agent_registry: Arc<dyn closeclaw_agent::AgentRegistryQuery>,
    ) {
        *self.agent_registry.write().await = Some(agent_registry);
    }

    /// Look up agent config by ID via the config manager.
    pub(crate) async fn get_agent_config(
        &self,
        agent_id: &str,
    ) -> Option<closeclaw_config::agents::ResolvedAgentConfig> {
        let config_manager = self.config_manager.read().await;
        let cm = config_manager.as_ref()?;
        let agents = cm.agents.read().unwrap();
        agents.get(agent_id).cloned()
    }

    /// Query per-agent workspace path via the agent registry.
    /// Falls back to the global workspace_dir if the agent has no
    /// per-agent workspace configured.
    pub(super) async fn query_agent_workspace(&self, agent_id: &str) -> Option<PathBuf> {
        let registry = self.agent_registry.read().await;
        let registry = registry.as_ref()?;
        registry.get_agent_workspace(agent_id).await
    }

    /// Query per-agent bootstrap mode via the agent registry.
    ///
    /// Returns the agent's configured [`BootstrapMode`], or `None` if
    /// the agent has no registry entry (caller should fall back to
    /// a sensible default).
    pub(super) async fn query_agent_bootstrap_mode(&self, agent_id: &str) -> Option<BootstrapMode> {
        let registry = self.agent_registry.read().await;
        let registry = registry.as_ref()?;
        registry.query_bootstrap_mode(agent_id).await
    }

    /// Set priority prompt overrides.
    pub async fn set_prompt_overrides(&self, overrides: Option<PromptOverrides>) {
        *self.prompt_overrides.write().await = overrides;
    }

    /// Get the current priority prompt overrides, if set.
    pub async fn get_prompt_overrides(&self) -> Option<PromptOverrides> {
        self.prompt_overrides.read().await.clone()
    }

    /// Set the system prompt builder.
    pub async fn set_system_prompt_builder(&self, builder: Arc<dyn SystemPromptBuilder>) {
        *self.system_prompt_builder.write().await = Some(builder);
    }

    /// Get the system prompt builder, if set.
    pub async fn get_system_prompt_builder(&self) -> Option<Arc<dyn SystemPromptBuilder>> {
        self.system_prompt_builder.read().await.clone()
    }

    /// Set the LLM caller.
    pub async fn set_llm_caller(&self, caller: Arc<dyn LlmCaller>) {
        *self.llm_caller.write().await = Some(caller);
    }

    /// Get the LLM caller, if set.
    pub async fn get_llm_caller(&self) -> Option<Arc<dyn LlmCaller>> {
        self.llm_caller.read().await.clone()
    }

    /// Inject a [`DynamicPromptBuilder`] into the session manager.
    ///
    /// Called by daemon (composition root) after construction so that
    /// `resolve()` and `force_new_for_channel()` can pass it to every
    /// new [`ConversationSession`].
    pub async fn set_dynamic_prompt_builder(&self, builder: Arc<dyn DynamicPromptBuilder>) {
        *self.dynamic_prompt_builder.write().await = Some(builder);
    }

    /// Get the dynamic prompt builder, if set.
    pub async fn get_dynamic_prompt_builder(&self) -> Option<Arc<dyn DynamicPromptBuilder>> {
        self.dynamic_prompt_builder.read().await.clone()
    }

    /// Initialize the consistency check timestamp after the startup full scan.
    ///
    /// Call this once after `run_consistency_check()` completes at startup
    /// so that subsequent incremental scans only examine records that
    /// changed since this point.
    pub fn initialize_consistency_check_time(&self) {
        let now = chrono::Utc::now().timestamp();
        *self.last_consistency_check_time.lock().unwrap() = Some(now);
    }

    /// Swap in a new config snapshot, releasing the old one.
    ///
    /// The old snapshot's `Arc` reference count decrements; once all
    /// holders release it, the memory is reclaimed automatically.
    pub(crate) async fn swap_config_snapshot(&self, snapshot: ConfigSnapshot) {
        let mut guard = self.config_snapshot.write().await;
        *guard = Some(snapshot);
    }

    /// Get the current config snapshot, if one has been swapped in.
    #[allow(dead_code)] // used in tests
    pub(crate) async fn get_config_snapshot(&self) -> Option<ConfigSnapshot> {
        self.config_snapshot.read().await.clone()
    }

    /// Set the shutdown handle for busy-count tracking.
    pub async fn set_shutdown_handle(&self, handle: Arc<ShutdownHandle>) {
        *self.shutdown_handle.write().await = Some(handle);
    }

    /// Get a clone of the shutdown handle, if set.
    pub(crate) async fn get_shutdown_handle(&self) -> Option<Arc<ShutdownHandle>> {
        self.shutdown_handle.read().await.clone()
    }

    /// Inject a [`PersistenceMetaStore`] into a conversation session's
    /// snapshot manager for metadata persistence.
    ///
    /// When the checkpoint manager (and thus persistence) is available,
    /// this wires the snapshot manager to persist metadata to the
    /// session checkpoint. When persistence is unavailable, the snapshot
    /// manager falls back to in-memory-only mode.
    pub(crate) async fn inject_snapshot_meta_store(
        &self,
        session_id: &str,
        conv: &mut ConversationSession,
    ) {
        if let Some(cm) = self.checkpoint_manager.read().await.as_ref() {
            let meta_store = Arc::new(PersistenceMetaStore::new(
                Arc::clone(cm.storage_arc()),
                session_id.to_string(),
            ));
            conv.set_snapshot_meta_store(meta_store);
        }
    }

    /// Inject persistence service for `persist_pending_checkpoint`.
    pub(crate) async fn inject_checkpoint_storage(&self, conv: &mut ConversationSession) {
        if let Some(cm) = self.checkpoint_manager.read().await.as_ref() {
            conv.set_checkpoint_storage(Arc::clone(cm.storage_arc()));
        }
    }

    /// Register a callback to invalidate the static-layer section cache.
    ///
    /// The daemon (composition root) injects this so that gateway code
    /// can trigger cache invalidation without depending on
    /// `closeclaw-system-prompt` directly.
    pub async fn set_cache_invalidator(&self, invalidator: Arc<dyn Fn() + Send + Sync>) {
        *self.cache_invalidator.write().await = Some(invalidator);
    }

    /// Invoke the registered cache-invalidation callback (if any).
    ///
    /// Called by `/system clear` to invalidate static-layer sections so
    /// the next prompt build regenerates from current state. No-op when
    /// no callback has been registered.
    pub async fn invalidate_static_cache(&self) {
        let guard = self.cache_invalidator.read().await;
        if let Some(cb) = guard.as_ref() {
            cb();
        }
    }

    /// Set the tool registry for building system prompt ToolsSection.
    pub async fn set_tool_registry(&self, registry: Arc<dyn ToolRegistryQuery>) {
        *self.tool_registry.write().await = Some(registry);
    }

    /// Set the skill registry for building system prompt SkillListingSection.
    pub async fn set_skill_registry(&self, registry: Arc<dyn SkillRegistryQuery>) {
        *self.skill_registry.write().await = Some(registry);
    }

    /// Get the current tool registry, if set.
    pub async fn get_tool_registry(&self) -> Option<Arc<dyn ToolRegistryQuery>> {
        self.tool_registry.read().await.clone()
    }

    /// Get the current skill registry, if set.
    pub async fn get_skill_registry(&self) -> Option<Arc<dyn SkillRegistryQuery>> {
        self.skill_registry.read().await.clone()
    }

    /// Register an IM adapter.
    pub async fn register_adapter(&self, name: String, adapter: Arc<dyn IMPlugin>) {
        let mut adapters = self.adapters.write().await;
        adapters.insert(name, adapter);
    }

    /// Set the persistence backend (backward-compatible wrapper).
    ///
    /// Internally creates a [`CheckpointManager`] wrapping the raw storage.
    /// Prefer [`set_checkpoint_manager`](Self::set_checkpoint_manager) when
    /// the caller already has a [`CheckpointManager`].
    pub async fn set_storage(&self, storage: Arc<dyn PersistenceService>) {
        let cm = Arc::new(CheckpointManager::new(storage));
        *self.checkpoint_manager.write().await = Some(cm);
    }

    /// Set the persistence coordination layer directly.
    pub async fn set_checkpoint_manager(&self, cm: Arc<CheckpointManager<dyn PersistenceService>>) {
        *self.checkpoint_manager.write().await = Some(cm);
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
            pending.insert(session_id.to_string(), chat_id);
        }
        result.restored
    }

    /// Take the pending restore notification for a session.
    /// Returns the chat_id if a restore notification is pending for this session.
    pub async fn take_restore_notification(&self, session_id: &str) -> Option<String> {
        let mut pending = self.pending_restore_notifications.write().await;
        pending.remove(session_id)
    }

    /// Update the thread_id in a session's checkpoint.
    /// Delegates to `session_helpers::update_checkpoint_thread_id`.
    async fn update_checkpoint_thread_id(&self, session_id: &str, thread_id: &Option<String>) {
        let cm_guard = self.checkpoint_manager.read().await;
        let Some(cm) = cm_guard.as_ref() else {
            warn!(
                session_id = %session_id,
                "storage not available, skipping thread_id update"
            );
            return;
        };
        session_helpers::update_checkpoint_thread_id(cm.as_ref(), session_id, thread_id).await;
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
            self.update_checkpoint_thread_id(active_id, &message.thread_id)
                .await;
            return Ok(active_id.clone());
        }

        let session_key =
            self.compute_session_key_local(channel, message, account_id, message.timestamp);
        let session_id = self
            .resolve(&session_key, channel, message, account_id)
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
                _ => {
                    // No existing checkpoint — create a fresh one
                    SessionCheckpoint::new(session_id.clone())
                        .with_status(SessionStatus::Active)
                        .with_platform(session.channel.clone())
                        .with_peer_id(session.agent_id.clone())
                        .with_agent_id(session.agent_id.clone())
                        .with_outbound_pending(pending)
                }
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
    pub async fn set_plan_state(&self, session_id: &str, plan_state: closeclaw_common::PlanState) {
        let cm_guard = {
            let guard = self.checkpoint_manager.read().await;
            match guard.as_ref() {
                Some(cm) => Arc::clone(cm),
                None => return,
            }
        };
        let mut cp = match cm_guard.load(session_id).await {
            Ok(Some(cp)) => cp,
            _ => return,
        };
        cp.plan_state = Some(plan_state);
        if let Err(e) = cm_guard.save_raw(&cp).await {
            tracing::warn!(
                session_id = %session_id,
                "failed to save plan_state: {}",
                e
            );
        }
    }

    /// Rebuild the system prompt for a session.
    /// Delegates to `ConversationSession::rebuild_system_prompt` which
    /// uses the session's own builder and overrides.
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

    /// Notify all active sessions that a configuration section has been updated.
    ///
    /// Iterates through all active sessions and rebuilds their system prompt
    /// so the next LLM request picks up the latest config values (tools,
    /// skills, system-prompt sections, etc.). Sessions themselves are not
    /// rebuilt; only the cached system prompt is invalidated.
    ///
    /// Sessions already observe the latest config values through the shared
    /// `Arc<ConfigManager>` reference, so this notification is the only
    /// explicit refresh needed to invalidate derived caches.
    ///
    /// The `snapshot` parameter carries the latest config snapshot for
    /// downstream reference-swap semantics (fully utilised in Step 1.3).
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
use closeclaw_common::SessionLookup;

#[async_trait::async_trait]
impl SessionLookup for SessionManager {
    async fn get_parent_of(&self, child_id: &str) -> Option<String> {
        SessionManager::get_parent_of(self, child_id).await
    }

    async fn get_chat_id(&self, session_id: &str) -> Option<String> {
        SessionManager::get_chat_id(self, session_id).await
    }

    async fn push_pending_message(
        &self,
        session_id: &str,
        msg: PendingMessage,
    ) -> Result<(), String> {
        SessionManager::push_pending_message(self, session_id, msg).await
    }

    async fn get_plan_state(&self, session_id: &str) -> Option<closeclaw_common::PlanState> {
        SessionManager::get_plan_state(self, session_id).await
    }

    async fn set_plan_state(&self, session_id: &str, plan_state: closeclaw_common::PlanState) {
        SessionManager::set_plan_state(self, session_id, plan_state).await;
    }

    async fn set_session_mode(&self, session_id: &str, mode: closeclaw_common::SessionMode) {
        if let Some(cs) = self.get_conversation_session(session_id).await {
            cs.write().await.set_session_mode(mode);
        }
    }
}
#[cfg(test)]
mod announce_dedup_tests;
#[cfg(test)]
mod announce_drain_outbound_tests;
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
mod rebuild_spawn_tree_tests;
#[cfg(test)]
mod recovery_injection_tests;
#[cfg(test)]
mod resolve_archived_recovery_tests;
#[cfg(test)]
mod resolve_checkpoint_status_tests;
#[cfg(test)]
mod resolve_registry_tests;
#[cfg(test)]
mod resolve_tests;
#[cfg(test)]
mod self_heal_tests;
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
mod spawn_mode_inherit_tests;
#[cfg(test)]
mod spawn_tests;
#[cfg(test)]
mod spawn_tree_tests;
#[cfg(test)]
mod stop_tests;
#[cfg(test)]
pub(crate) mod test_helpers;
#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
mod yield_recovery_tests;
#[cfg(test)]
mod yield_timeout_tests;
