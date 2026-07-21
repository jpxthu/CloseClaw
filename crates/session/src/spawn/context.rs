//! Dependency injection trait for child session creation.
//!
//! Defines [`SpawnCreationContext`], the abstract interface that
//! `create_child_session` uses to interact with the gateway's session
//! state. The gateway implements this trait on `SessionManager`, keeping
//! the session crate decoupled from gateway internals.

use std::sync::Arc;

use crate::llm_session::ConversationSession;
use crate::persistence::ReasoningLevel;
use closeclaw_config::agents::ResolvedAgentConfig;

use closeclaw_common::{LlmCaller, PromptOverrides, ShutdownSignal, SystemPromptBuilder};

/// Abstract interface for querying session state during child session creation.
///
/// The gateway implements this trait on `SessionManager`. Each method
/// corresponds to a specific dependency the creation logic needs, keeping
/// the session crate decoupled from concrete gateway types.
#[async_trait::async_trait]
pub trait SpawnCreationContext: Send + Sync {
    /// Get the parent session's `ConversationSession` for fork mode
    /// (cloning messages) and cancel-token derivation.
    async fn get_parent_conversation_session(
        &self,
        parent_session_id: &str,
    ) -> Option<Arc<tokio::sync::RwLock<ConversationSession>>>;

    /// Load a checkpoint from persistent storage.
    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Option<crate::persistence::SessionCheckpoint>;

    /// Save a checkpoint to persistent storage.
    async fn save_checkpoint(&self, cp: &crate::persistence::SessionCheckpoint);

    /// Look up a resolved agent config by agent ID.
    fn get_agent_config(&self, agent_id: &str) -> Option<ResolvedAgentConfig>;

    /// Get the shutdown signal for busy-count tracking.
    fn shutdown_signal(&self) -> Option<Arc<dyn ShutdownSignal>>;

    /// Get the default reasoning level for new sessions.
    fn default_reasoning_level(&self) -> ReasoningLevel;

    /// Get the LLM caller for delegation.
    fn llm_caller(&self) -> Option<Arc<dyn LlmCaller>>;

    /// Get the system prompt builder.
    fn system_prompt_builder(&self) -> Option<Arc<dyn SystemPromptBuilder>>;

    /// Get the priority prompt overrides.
    fn prompt_overrides(&self) -> Option<PromptOverrides>;

    /// Get the dynamic prompt builder for per-request injection.
    fn dynamic_prompt_builder(&self) -> Option<Arc<dyn closeclaw_common::DynamicPromptBuilder>>;

    /// Get the skill listing provider for per-turn injection.
    fn skill_listing_provider(&self) -> Option<Arc<dyn closeclaw_common::SkillListingProvider>>;

    /// Get the sender/user ID for a session (used for workspace path fallback).
    async fn sender_id(&self, session_id: &str) -> Option<String>;
}
