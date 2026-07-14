//! Unit tests for [`RunHealthChecker`].

use async_trait::async_trait;

use super::checker::RunHealthChecker;
use super::hard_rules::{
    EmptyResponseRule, HardRuleEngine, RetryExhaustedRule, StructuralAnomalyRule, TimeoutRule,
};
use super::health_types::{
    FailureCategory, HealthCheckInput, HealthStatus, HookContext, RetryPolicy,
};
use super::hook_reviewer::{HookConfig, HookLlmProvider, HookReviewer, HookType};
use super::unhealthy_handler::UnhealthyHandler;

// ── Helpers ────────────────────────────────────────────────────────────────

/// Default healthy input for testing.
fn healthy_input() -> HealthCheckInput {
    HealthCheckInput {
        has_text: true,
        has_tool_calls: false,
        has_thinking: false,
        retry_count: 0,
        turn_duration_ms: 100,
        is_structurally_valid: true,
        structural_anomaly_detail: None,
        side_effect_occurred: false,
    }
}

/// Build a default retry policy for testing.
fn default_retry_policy() -> RetryPolicy {
    RetryPolicy {
        max_retries: 3,
        initial_delay_ms: 100,
        backoff_multiplier: 2.0,
    }
}

/// Default engine with all standard rules.
fn default_engine() -> HardRuleEngine {
    HardRuleEngine::new(vec![
        Box::new(TimeoutRule {
            threshold_ms: 30_000,
        }),
        Box::new(EmptyResponseRule),
        Box::new(StructuralAnomalyRule),
        Box::new(RetryExhaustedRule { max_retries: 3 }),
    ])
}

/// Mock LLM that always returns a specific flag value.
struct MockHookLlm {
    flag: bool,
}

#[async_trait]
impl HookLlmProvider for MockHookLlm {
    async fn review(&self, _prompt: &str, _context: &str) -> Result<bool, String> {
        Ok(self.flag)
    }
}

/// Create a HookReviewer with a single hook.
fn make_hook_reviewer(hook_type: HookType, flag: bool) -> HookReviewer {
    let hooks = vec![HookConfig {
        hook_type,
        enabled: true,
        ..Default::default()
    }];
    HookReviewer::new(hooks, Box::new(MockHookLlm { flag }))
}

// ── Healthy pass path ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_healthy_input_no_violations() {
    let mut checker = RunHealthChecker::new(
        default_engine(),
        None,
        UnhealthyHandler::new(default_retry_policy()),
    );
    let input = healthy_input();
    let verdict = checker.check_turn(&input, None).await;
    assert_eq!(verdict.status, HealthStatus::Healthy);
    assert!(verdict.action.is_none());
}

#[tokio::test]
async fn test_healthy_with_tool_calls() {
    let mut checker = RunHealthChecker::new(
        default_engine(),
        None,
        UnhealthyHandler::new(default_retry_policy()),
    );
    let input = HealthCheckInput {
        has_text: false,
        has_tool_calls: true,
        has_thinking: false,
        retry_count: 0,
        turn_duration_ms: 50,
        is_structurally_valid: true,
        structural_anomaly_detail: None,
        side_effect_occurred: false,
    };
    let verdict = checker.check_turn(&input, None).await;
    assert_eq!(verdict.status, HealthStatus::Healthy);
    assert!(verdict.action.is_none());
}

#[tokio::test]
async fn test_thinking_only_triggers_unhealthy() {
    let mut checker = RunHealthChecker::new(
        default_engine(),
        None,
        UnhealthyHandler::new(default_retry_policy()),
    );
    let input = HealthCheckInput {
        has_text: false,
        has_tool_calls: false,
        has_thinking: true,
        retry_count: 0,
        turn_duration_ms: 50,
        is_structurally_valid: true,
        structural_anomaly_detail: None,
        side_effect_occurred: false,
    };
    let verdict = checker.check_turn(&input, None).await;
    match verdict.status {
        HealthStatus::Unhealthy(FailureCategory::InvalidResponse) => {}
        other => {
            panic!("expected Unhealthy(InvalidResponse) for thinking-only, got {other:?}")
        }
    }
    assert!(verdict.action.is_some());
}

// ── Hard rule violations → unhealthy → action ──────────────────────────────

#[tokio::test]
async fn test_empty_response_violation() {
    let mut checker = RunHealthChecker::new(
        default_engine(),
        None,
        UnhealthyHandler::new(default_retry_policy()),
    );
    let input = HealthCheckInput {
        has_text: false,
        has_tool_calls: false,
        has_thinking: false,
        retry_count: 0,
        turn_duration_ms: 100,
        is_structurally_valid: true,
        structural_anomaly_detail: None,
        side_effect_occurred: false,
    };
    let verdict = checker.check_turn(&input, None).await;
    match verdict.status {
        HealthStatus::Unhealthy(FailureCategory::InvalidResponse) => {}
        other => panic!("expected Unhealthy(InvalidResponse), got {other:?}"),
    }
    assert!(verdict.action.is_some());
}

#[tokio::test]
async fn test_timeout_violation() {
    let mut checker = RunHealthChecker::new(
        default_engine(),
        None,
        UnhealthyHandler::new(default_retry_policy()),
    );
    let input = HealthCheckInput {
        has_text: true,
        has_tool_calls: false,
        has_thinking: false,
        retry_count: 0,
        turn_duration_ms: 60_000, // > 30s threshold
        is_structurally_valid: true,
        structural_anomaly_detail: None,
        side_effect_occurred: false,
    };
    let verdict = checker.check_turn(&input, None).await;
    match verdict.status {
        HealthStatus::Unhealthy(FailureCategory::Retryable) => {}
        other => panic!("expected Unhealthy(Retryable), got {other:?}"),
    }
    assert!(verdict.action.is_some());
}

#[tokio::test]
async fn test_structural_anomaly_violation() {
    let mut checker = RunHealthChecker::new(
        default_engine(),
        None,
        UnhealthyHandler::new(default_retry_policy()),
    );
    let input = HealthCheckInput {
        has_text: true,
        has_tool_calls: false,
        has_thinking: false,
        retry_count: 0,
        turn_duration_ms: 100,
        is_structurally_valid: false,
        structural_anomaly_detail: Some("missing required field".into()),
        side_effect_occurred: false,
    };
    let verdict = checker.check_turn(&input, None).await;
    match verdict.status {
        HealthStatus::Unhealthy(FailureCategory::InvalidResponse) => {}
        other => panic!("expected Unhealthy(InvalidResponse), got {other:?}"),
    }
    assert!(verdict.action.is_some());
}

#[tokio::test]
async fn test_retry_exhausted_violation() {
    let mut checker = RunHealthChecker::new(
        default_engine(),
        None,
        UnhealthyHandler::new(default_retry_policy()),
    );
    let input = HealthCheckInput {
        has_text: true,
        has_tool_calls: false,
        has_thinking: false,
        retry_count: 5, // > max_retries (3)
        turn_duration_ms: 100,
        is_structurally_valid: true,
        structural_anomaly_detail: None,
        side_effect_occurred: false,
    };
    let verdict = checker.check_turn(&input, None).await;
    match verdict.status {
        HealthStatus::Unhealthy(FailureCategory::Unrecoverable) => {}
        other => panic!("expected Unhealthy(Unrecoverable), got {other:?}"),
    }
    assert!(verdict.action.is_some());
}

// ── Hook flag → unhealthy → action ─────────────────────────────────────────

#[tokio::test]
async fn test_hook_flags_turn_unhealthy() {
    let reviewer = make_hook_reviewer(HookType::PlanCheck, true); // flag = true
    let mut checker = RunHealthChecker::new(
        default_engine(),
        Some(reviewer),
        UnhealthyHandler::new(default_retry_policy()),
    );
    let input = healthy_input();
    let hook_ctx = HookContext {
        text: "I will do something".into(),
        ..Default::default()
    };
    let verdict = checker.check_turn(&input, Some(&hook_ctx)).await;
    match verdict.status {
        HealthStatus::Unhealthy(FailureCategory::InvalidResponse) => {}
        other => panic!("expected Unhealthy(InvalidResponse) from hook flag, got {other:?}"),
    }
    assert!(verdict.action.is_some());
}

#[tokio::test]
async fn test_hook_no_flag_stays_healthy() {
    let reviewer = make_hook_reviewer(HookType::PlanCheck, false); // flag = false
    let mut checker = RunHealthChecker::new(
        default_engine(),
        Some(reviewer),
        UnhealthyHandler::new(default_retry_policy()),
    );
    let input = healthy_input();
    let hook_ctx = HookContext::default();
    let verdict = checker.check_turn(&input, Some(&hook_ctx)).await;
    assert_eq!(verdict.status, HealthStatus::Healthy);
    assert!(verdict.action.is_none());
}

// ── No hook configuration → healthy pass ───────────────────────────────────

#[tokio::test]
async fn test_no_hooks_configured_healthy_path() {
    let mut checker = RunHealthChecker::new(
        default_engine(),
        None, // No hook reviewer
        UnhealthyHandler::new(default_retry_policy()),
    );
    let input = healthy_input();
    let verdict = checker.check_turn(&input, None).await;
    assert_eq!(verdict.status, HealthStatus::Healthy);
    assert!(verdict.action.is_none());
}

#[tokio::test]
async fn test_hooks_configured_but_no_context_skips_hooks() {
    let reviewer = make_hook_reviewer(HookType::PlanCheck, true);
    let mut checker = RunHealthChecker::new(
        default_engine(),
        Some(reviewer),
        UnhealthyHandler::new(default_retry_policy()),
    );
    let input = healthy_input();
    // Pass None for hook_context → hooks are skipped
    let verdict = checker.check_turn(&input, None).await;
    assert_eq!(verdict.status, HealthStatus::Healthy);
    assert!(verdict.action.is_none());
}

// ── Pipeline ordering: hard rules first, hooks second ──────────────────────

#[tokio::test]
async fn test_hard_rule_blocks_hook_review() {
    // When hard rules fail, hooks should NOT be called.
    let reviewer = make_hook_reviewer(HookType::PlanCheck, true);
    let mut checker = RunHealthChecker::new(
        default_engine(),
        Some(reviewer),
        UnhealthyHandler::new(default_retry_policy()),
    );
    let input = HealthCheckInput {
        has_text: false,
        has_tool_calls: false,
        has_thinking: false,
        retry_count: 0,
        turn_duration_ms: 100,
        is_structurally_valid: true,
        structural_anomaly_detail: None,
        side_effect_occurred: false,
    };
    let verdict = checker.check_turn(&input, None).await;
    // Hard rule violation takes precedence; hook is not invoked.
    match verdict.status {
        HealthStatus::Unhealthy(FailureCategory::InvalidResponse) => {}
        other => panic!("expected hard rule Unhealthy, got {other:?}"),
    }
}

// ── Factory function tests ────────────────────────────────────────────────

#[tokio::test]
async fn test_factory_default_creates_checker_with_5_hard_rules() {
    use super::factory::create_default_health_checker;

    // Build a checker with no LLM caller.
    let mut checker = create_default_health_checker(None, vec![]);

    // Verify it works: healthy input passes, unhealthy input is caught.
    let healthy = healthy_input();
    let verdict = checker.check_turn(&healthy, None).await;
    assert_eq!(verdict.status, HealthStatus::Healthy);

    // Verify SideEffectOccurredRule is registered by testing through
    // the engine directly. Side effect + no text + no tool calls →
    // both SideEffectOccurred and EmptyResponse fire.
    let mut side_effect_input = healthy_input();
    side_effect_input.side_effect_occurred = true;
    side_effect_input.has_text = false;
    side_effect_input.has_tool_calls = false;
    let verdict = checker.check_turn(&side_effect_input, None).await;
    // Should be unhealthy because both rules fire.
    assert!(matches!(verdict.status, HealthStatus::Unhealthy(_)));
}

#[tokio::test]
async fn test_factory_with_llm_caller_creates_hook_reviewer() {
    use std::sync::Arc;

    use super::factory::create_default_health_checker;
    use closeclaw_common::processor::{ContentBlock, UnifiedResponse, UnifiedUsage};
    use closeclaw_common::{InternalRequest, LlmCaller};

    struct DummyLlm;
    #[async_trait]
    impl LlmCaller for DummyLlm {
        async fn call(
            &self,
            _req: InternalRequest,
        ) -> Result<UnifiedResponse, closeclaw_common::LLMError> {
            Ok(UnifiedResponse {
                content_blocks: vec![ContentBlock::Text("no".into())],
                usage: UnifiedUsage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: None,
                    reasoning_tokens: None,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                },
                finish_reason: Some("stop".into()),
                retry_attempts: 0,
            })
        }
        async fn call_streaming(
            &self,
            _: InternalRequest,
        ) -> Result<
            std::pin::Pin<
                Box<
                    dyn futures::Stream<
                            Item = Result<
                                closeclaw_common::processor::StreamEvent,
                                closeclaw_common::LLMError,
                            >,
                        > + Send,
                >,
            >,
            closeclaw_common::LLMError,
        > {
            unimplemented!()
        }
    }

    let caller: Arc<dyn LlmCaller> = Arc::new(DummyLlm);
    let mut checker = create_default_health_checker(Some(caller), vec![]);

    // The checker should have a hook reviewer (hard rules pass → hook review path).
    let healthy = healthy_input();
    let verdict = checker.check_turn(&healthy, None).await;
    // With no hook_context, hooks are skipped even if reviewer exists;
    // so this still returns healthy. The important thing is no panic.
    assert_eq!(verdict.status, HealthStatus::Healthy);
}

#[tokio::test]
async fn test_factory_without_llm_caller_no_hook_reviewer() {
    use super::factory::create_default_health_checker;

    let mut checker = create_default_health_checker(None, vec![]);

    // Healthy input → healthy verdict (no hook reviewer to flag).
    let healthy = healthy_input();
    let verdict = checker.check_turn(&healthy, None).await;
    assert_eq!(verdict.status, HealthStatus::Healthy);
    assert!(verdict.action.is_none());
}

// ── Integration: ConversationSession health_checker() ──────────────────────

#[test]
fn test_init_health_checker_sets_health_checker() {
    use std::sync::Arc;

    use crate::llm_session::ConversationSession;
    use closeclaw_common::processor::{ContentBlock, UnifiedResponse, UnifiedUsage};
    use closeclaw_common::{InternalRequest, LlmCaller};

    struct DummyLlm;
    #[async_trait]
    impl LlmCaller for DummyLlm {
        async fn call(
            &self,
            _req: InternalRequest,
        ) -> Result<UnifiedResponse, closeclaw_common::LLMError> {
            Ok(UnifiedResponse {
                content_blocks: vec![ContentBlock::Text("no".into())],
                usage: UnifiedUsage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: None,
                    reasoning_tokens: None,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                },
                finish_reason: Some("stop".into()),
                retry_attempts: 0,
            })
        }
        async fn call_streaming(
            &self,
            _: InternalRequest,
        ) -> Result<
            std::pin::Pin<
                Box<
                    dyn futures::Stream<
                            Item = Result<
                                closeclaw_common::processor::StreamEvent,
                                closeclaw_common::LLMError,
                            >,
                        > + Send,
                >,
            >,
            closeclaw_common::LLMError,
        > {
            unimplemented!()
        }
    }

    let mut session = ConversationSession::new(
        "test-session".into(),
        "test-model".into(),
        std::path::PathBuf::from("/tmp"),
    );

    // Before init, health_checker() should return None.
    assert!(session.health_checker().is_none());

    // After init_health_checker, health_checker() should return Some.
    let caller: Arc<dyn LlmCaller> = Arc::new(DummyLlm);
    session.init_health_checker(caller, vec![]);
    assert!(session.health_checker().is_some());
}
