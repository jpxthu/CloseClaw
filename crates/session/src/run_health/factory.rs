//! Factory for constructing a fully-configured [`RunHealthChecker`].
//!
//! Provides [`create_default_health_checker`] which builds the standard
//! pipeline: 5 hard rules, optional hook reviewer, and unhealthy handler
//! with default retry policy.

use std::sync::Arc;

use closeclaw_common::LlmCaller;

use super::checker::RunHealthChecker;
use super::hard_rules::{
    EmptyResponseRule, HardRuleEngine, RetryExhaustedRule, SideEffectOccurredRule,
    StructuralAnomalyRule, TimeoutRule,
};
use super::health_types::RetryPolicy;
use super::hook_reviewer::{HookConfig, HookReviewer};
use super::llm_caller_hook_provider::LlmCallerHookProvider;
use super::unhealthy_handler::UnhealthyHandler;

/// Threshold for the timeout hard rule (5 minutes).
const TIMEOUT_THRESHOLD_MS: u64 = 300_000;

/// Maximum retries for the retry-exhausted hard rule.
const MAX_RETRIES: u32 = 3;

/// Build a [`RunHealthChecker`] with the standard pipeline configuration.
///
/// # Hard rules
///
/// The default pipeline includes 5 deterministic checks:
/// 1. [`TimeoutRule`] — turn exceeded 5 minutes
/// 2. [`EmptyResponseRule`] — LLM returned no usable content
/// 3. [`StructuralAnomalyRule`] — response structure is malformed
/// 4. [`RetryExhaustedRule`] — retry counter reached maximum
/// 5. [`SideEffectOccurredRule`] — tools executed but response incomplete
///
/// # Hook reviewer
///
/// If an [`LlmCaller`] is provided, a [`HookReviewer`] is constructed
/// wrapping it in a [`LlmCallerHookProvider`]. The caller can supply
/// hook configurations to enable specific quality gates.
pub fn create_default_health_checker(
    llm_caller: Option<Arc<dyn LlmCaller>>,
    hook_configs: Vec<HookConfig>,
) -> RunHealthChecker {
    let hard_rules = HardRuleEngine::new(vec![
        Box::new(TimeoutRule {
            threshold_ms: TIMEOUT_THRESHOLD_MS,
        }),
        Box::new(EmptyResponseRule),
        Box::new(StructuralAnomalyRule),
        Box::new(RetryExhaustedRule {
            max_retries: MAX_RETRIES,
        }),
        Box::new(SideEffectOccurredRule),
    ]);

    let hook_reviewer = llm_caller.map(|caller| {
        let provider = LlmCallerHookProvider::new(caller);
        HookReviewer::new(hook_configs, Box::new(provider))
    });

    let handler = UnhealthyHandler::new(RetryPolicy::default());

    RunHealthChecker::new(hard_rules, hook_reviewer, handler)
}
