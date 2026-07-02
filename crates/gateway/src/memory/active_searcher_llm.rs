//! LLM caller trait for the active-searcher pipeline.

use super::active_searcher::ActiveSearcherError;

/// Trait for making LLM completion calls from the active-searcher.
#[async_trait::async_trait]
pub trait LlmCaller: Send + Sync {
    /// Complete a prompt using the LLM.
    #[allow(dead_code)]
    async fn complete(&self, prompt: &str) -> Result<String, ActiveSearcherError>;
}

/// Session roles that should NOT trigger the active-searcher.
const EXCLUDED_ROLES: &[&str] = &["memory-miner", "dreaming"];

/// Check whether the given role should trigger the active searcher.
///
/// Returns `false` for roles that are excluded (e.g. memory-miner, dreaming)
/// to avoid circular memory writes.
pub fn should_trigger_role(role: &str) -> bool {
    !EXCLUDED_ROLES.contains(&role)
}
