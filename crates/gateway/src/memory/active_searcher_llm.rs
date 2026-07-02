//! LLM caller trait for the active-searcher pipeline.
//!
//! Re-exports the canonical trait from `closeclaw-memory`.

pub use closeclaw_memory::active_searcher_llm::LlmCaller;

/// Session roles that should NOT trigger the active-searcher.
const EXCLUDED_ROLES: &[&str] = &["memory-miner", "dreaming"];

/// Check whether the given role should trigger the active searcher.
///
/// Returns `false` for roles that are excluded (e.g. memory-miner, dreaming)
/// to avoid circular memory writes.
pub fn should_trigger_role(role: &str) -> bool {
    !EXCLUDED_ROLES.contains(&role)
}
