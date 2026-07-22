//! Setter and getter methods for [`SessionManager`].
//!
//! Extracted from `session_manager.rs` to keep file size under the
//! 1000-line limit while preserving full doc comments.

use super::register_tools;
use super::SessionManager;
use crate::shutdown_handle::ShutdownHandle;
use closeclaw_common::IMPlugin;
use closeclaw_common::{
    DynamicPromptBuilder, LlmCaller, PromptOverrides, SkillListingProvider, SkillRegistryQuery,
    SystemPromptBuilder, ToolRegistryQuery,
};
use closeclaw_config::manager::{ConfigManager, ConfigSnapshot};
use closeclaw_session::bootstrap::loader::BootstrapMode;
use closeclaw_session::checkpoint_manager::CheckpointManager;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::PersistenceService;
use closeclaw_session::run_health::PersistenceMetaStore;
use std::path::PathBuf;
use std::sync::Arc;

impl SessionManager {
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

    /// Inject the tool-register callback.
    ///
    /// Called by daemon (composition root) so that [`register_tools`](Self::register_tools)
    /// can delegate to the tools crate without a direct dependency.
    pub async fn set_tool_register_fn(&self, func: register_tools::ToolRegisterFn) {
        register_tools::set_tool_register_fn(self, func).await;
    }

    /// Register session tools into the given tool registry.
    ///
    /// Delegates to the callback set via [`set_tool_register_fn`](Self::set_tool_register_fn).
    /// If no callback has been registered, this is a no-op with a warning log.
    pub async fn register_tools(
        &self,
        registry: &dyn closeclaw_common::ToolRegistry,
    ) -> Result<(), closeclaw_common::ToolRegistrarError> {
        register_tools::register_tools(self, registry).await
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

    /// Set the skill listing provider for per-turn injection.
    pub async fn set_skill_listing_provider(&self, provider: Arc<dyn SkillListingProvider>) {
        *self.skill_listing_provider.write().await = Some(provider);
    }

    /// Get the skill listing provider, if set.
    pub async fn get_skill_listing_provider(&self) -> Option<Arc<dyn SkillListingProvider>> {
        self.skill_listing_provider.read().await.clone()
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
}
