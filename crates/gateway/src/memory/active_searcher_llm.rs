//! LLM caller trait for the active-searcher pipeline.

use super::active_searcher::ActiveSearcherError;

/// Trait for making LLM completion calls from the active-searcher.
#[async_trait::async_trait]
pub trait LlmCaller: Send + Sync {
    /// Complete a prompt using the LLM.
    #[allow(dead_code)]
    async fn complete(&self, prompt: &str) -> Result<String, ActiveSearcherError>;
}

/// Check whether the given agent_id should trigger the active searcher.
///
/// Returns `false` for agents that are excluded (e.g. memory-miner, dreaming).
pub fn should_trigger_role(agent_id: &str) -> bool {
    !agent_id.starts_with("memory-miner") && !agent_id.starts_with("dreaming")
}
