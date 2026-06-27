//! Active-searcher: background context injection for sessions.
//!
//! This is a minimal stub implementation that provides the types
//! needed by `SessionMessageHandler::maybe_spawn_active_searcher`.

use std::collections::HashSet;
use std::path::Path;

use closeclaw_common::ActiveSearcherOverride;

use super::active_searcher_llm::LlmCaller;

/// Configuration for the active searcher.
#[derive(Debug, Clone)]
pub struct ActiveSearcherConfig {
    /// Model to use for the searcher LLM calls.
    pub model: String,
    /// Number of recent context turns to include.
    pub context_turns: usize,
}

impl ActiveSearcherConfig {
    /// Build a config from agent-level overrides.
    pub fn from_agent_config(
        agent_model: Option<&str>,
        override_config: Option<&ActiveSearcherOverride>,
    ) -> Self {
        let model = override_config
            .and_then(|o| o.model.clone())
            .or_else(|| agent_model.map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        let context_turns = override_config.and_then(|o| o.context_turns).unwrap_or(10);
        Self {
            model,
            context_turns,
        }
    }
}

/// Error type for active-searcher operations.
#[derive(Debug)]
#[allow(dead_code)]
pub enum ActiveSearcherError {
    /// LLM call failed.
    Llm(String),
    /// Database error.
    Database(String),
    /// Other error.
    Other(String),
}

impl std::fmt::Display for ActiveSearcherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Llm(e) => write!(f, "LLM error: {}", e),
            Self::Database(e) => write!(f, "Database error: {}", e),
            Self::Other(e) => write!(f, "Active searcher error: {}", e),
        }
    }
}

impl std::error::Error for ActiveSearcherError {}

/// Active searcher that runs in the background to find relevant context.
pub struct ActiveSearcher {
    config: ActiveSearcherConfig,
    #[allow(dead_code)]
    db_path: std::path::PathBuf,
}

impl ActiveSearcher {
    /// Create a new active searcher with the given database path and config.
    pub fn new(db_path: &Path, config: ActiveSearcherConfig) -> Self {
        Self {
            config,
            db_path: db_path.to_path_buf(),
        }
    }

    /// Get a reference to the configuration.
    pub fn config(&self) -> &ActiveSearcherConfig {
        &self.config
    }

    /// Run the active searcher for the given context.
    ///
    /// This is a minimal stub that always returns `None`.
    pub async fn run<C: LlmCaller>(
        &self,
        _agent_id: &str,
        _role: &str,
        _content: &str,
        _context_messages: &[closeclaw_llm::session::SessionMessage],
        _injected_ids: &HashSet<i64>,
        _caller: &C,
    ) -> Option<closeclaw_llm::session::MemoryInjection> {
        // Stub: no-op in this implementation.
        None
    }
}
