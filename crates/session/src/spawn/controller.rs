//! SpawnController — validates spawn requests against agent configuration.
//!
//! This module defines the `SpawnController` struct and the `SpawnContext`
//! trait used for dependency injection. The controller validates depth,
//! concurrency, whitelist, and permission checks before allowing a spawn.
//!
//! Implementation will be migrated from `closeclaw-gateway` in Step 1.2.

use std::sync::Arc;

use super::error::SpawnError;
use super::types::SpawnValidationResult;

/// Dependency injection trait for querying active child session counts.
///
/// Implemented by `SessionManager` on the gateway side to decouple
/// `SpawnController` from the concrete SessionManager type.
#[async_trait::async_trait]
#[allow(dead_code)] // Trait methods used in Step 1.2 implementation
pub trait SpawnContext: Send + Sync {
    /// Return the number of active (non-completed) child sessions
    /// for the given parent session.
    fn active_children_count(&self, parent_session_id: &str) -> usize;

    /// Look up the agent ID associated with a session.
    fn chat_id(&self, session_id: &str) -> Option<String>;

    /// Look up the sender/user ID associated with a session.
    fn sender_id(&self, session_id: &str) -> Option<String>;

    /// Get the effective max spawn depth budget for a session.
    fn effective_max_spawn_depth(&self, session_id: &str) -> Option<u32>;
}

/// Validates spawn requests and computes effective child configuration.
#[allow(dead_code)] // Fields used in Step 1.2 implementation
pub struct SpawnController {
    config_manager: Arc<closeclaw_config::ConfigManager>,
    context: Box<dyn SpawnContext>,
}

impl SpawnController {
    #[allow(dead_code)] // Used in Step 1.2
    pub fn new(
        config_manager: Arc<closeclaw_config::ConfigManager>,
        context: Box<dyn SpawnContext>,
    ) -> Self {
        Self {
            config_manager,
            context,
        }
    }

    /// Validate a spawn request.
    ///
    /// Returns a [`SpawnValidationResult`] with the target agent's resolved
    /// config and the effective max spawn depth for the child, or a
    /// [`SpawnError`] on failure.
    ///
    /// Implementation to be added in Step 1.2.
    pub async fn validate(
        &self,
        _parent_session_id: &str,
        _target_agent_id: Option<&str>,
    ) -> Result<SpawnValidationResult, SpawnError> {
        todo!("validate — implementation migrated in Step 1.2")
    }
}
