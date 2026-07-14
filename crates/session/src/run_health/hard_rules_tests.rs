//! Tests for hard-rule engine and individual rules.

use super::hard_rules::*;
use super::health_types::*;

// ─── helpers ────────────────────────────────────────────────

fn input_base() -> HealthCheckInput {
    HealthCheckInput {
        has_text: true,
        has_tool_calls: false,
        has_thinking: false,
        retry_count: 0,
        turn_duration_ms: 5_000,
        is_structurally_valid: true,
        structural_anomaly_detail: None,
        side_effect_occurred: false,
    }
}

// ─── TimeoutRule ────────────────────────────────────────────

#[tokio::test]
async fn timeout_rule_no_trigger_when_within_threshold() {
    let rule = TimeoutRule {
        threshold_ms: 10_000,
    };
    let mut input = input_base();
    input.turn_duration_ms = 5_000;
    assert_eq!(rule.check(&input).await, None);
}

#[tokio::test]
async fn timeout_rule_triggers_when_exceeded() {
    let rule = TimeoutRule {
        threshold_ms: 10_000,
    };
    let mut input = input_base();
    input.turn_duration_ms = 15_000;
    let result = rule.check(&input).await;
    assert_eq!(
        result,
        Some(HardRuleViolation::Timeout {
            elapsed_ms: 15_000,
            threshold_ms: 10_000,
        })
    );
}

#[tokio::test]
async fn timeout_rule_boundary_exact_threshold_not_triggered() {
    let rule = TimeoutRule {
        threshold_ms: 10_000,
    };
    let mut input = input_base();
    input.turn_duration_ms = 10_000;
    assert_eq!(rule.check(&input).await, None);
}

#[tokio::test]
async fn timeout_rule_boundary_one_over_threshold_triggered() {
    let rule = TimeoutRule {
        threshold_ms: 10_000,
    };
    let mut input = input_base();
    input.turn_duration_ms = 10_001;
    assert_eq!(
        rule.check(&input).await,
        Some(HardRuleViolation::Timeout {
            elapsed_ms: 10_001,
            threshold_ms: 10_000,
        })
    );
}

// ─── EmptyResponseRule ──────────────────────────────────────

#[tokio::test]
async fn empty_rule_no_trigger_when_has_text() {
    let rule = EmptyResponseRule;
    let mut input = input_base();
    input.has_text = true;
    assert_eq!(rule.check(&input).await, None);
}

#[tokio::test]
async fn empty_rule_no_trigger_when_has_tool_calls() {
    let rule = EmptyResponseRule;
    let mut input = input_base();
    input.has_text = false;
    input.has_tool_calls = true;
    assert_eq!(rule.check(&input).await, None);
}

#[tokio::test]
async fn empty_rule_triggers_when_no_content() {
    let rule = EmptyResponseRule;
    let mut input = input_base();
    input.has_text = false;
    input.has_tool_calls = false;
    input.has_thinking = false;
    assert_eq!(
        rule.check(&input).await,
        Some(HardRuleViolation::EmptyResponse)
    );
}

#[tokio::test]
async fn empty_rule_only_thinking_triggers() {
    let rule = EmptyResponseRule;
    let mut input = input_base();
    input.has_text = false;
    input.has_tool_calls = false;
    input.has_thinking = true;
    assert_eq!(
        rule.check(&input).await,
        Some(HardRuleViolation::ThinkingOnlyResponse)
    );
}

#[tokio::test]
async fn empty_response_rule_text_with_thinking() {
    let rule = EmptyResponseRule;
    let mut input = input_base();
    input.has_text = true;
    input.has_tool_calls = false;
    input.has_thinking = true;
    assert_eq!(rule.check(&input).await, None);
}

#[tokio::test]
async fn empty_response_rule_tool_calls_with_thinking() {
    let rule = EmptyResponseRule;
    let mut input = input_base();
    input.has_text = false;
    input.has_tool_calls = true;
    input.has_thinking = true;
    assert_eq!(rule.check(&input).await, None);
}

// ─── StructuralAnomalyRule ──────────────────────────────────

#[tokio::test]
async fn structural_rule_no_trigger_when_valid() {
    let rule = StructuralAnomalyRule;
    let mut input = input_base();
    input.is_structurally_valid = true;
    assert_eq!(rule.check(&input).await, None);
}

#[tokio::test]
async fn structural_rule_triggers_when_invalid_with_detail() {
    let rule = StructuralAnomalyRule;
    let mut input = input_base();
    input.is_structurally_valid = false;
    input.structural_anomaly_detail = Some("missing content field".into());
    assert_eq!(
        rule.check(&input).await,
        Some(HardRuleViolation::StructuralAnomaly {
            detail: "missing content field".into(),
        })
    );
}

#[tokio::test]
async fn structural_rule_triggers_when_invalid_no_detail() {
    let rule = StructuralAnomalyRule;
    let mut input = input_base();
    input.is_structurally_valid = false;
    input.structural_anomaly_detail = None;
    assert_eq!(
        rule.check(&input).await,
        Some(HardRuleViolation::StructuralAnomaly {
            detail: "unknown structural anomaly".into(),
        })
    );
}

// ─── SideEffectOccurredRule ─────────────────────────────────

#[tokio::test]
async fn side_effect_rule_triggers_when_tool_executed_and_interrupted() {
    let rule = SideEffectOccurredRule;
    let mut input = input_base();
    input.side_effect_occurred = true;
    input.has_text = false;
    input.has_tool_calls = false;
    assert_eq!(
        rule.check(&input).await,
        Some(HardRuleViolation::SideEffectOccurred)
    );
}

#[tokio::test]
async fn side_effect_rule_no_trigger_when_response_normal() {
    let rule = SideEffectOccurredRule;
    let mut input = input_base();
    input.side_effect_occurred = true;
    input.has_text = true;
    input.has_tool_calls = false;
    assert_eq!(rule.check(&input).await, None);
}

#[tokio::test]
async fn side_effect_rule_no_trigger_when_no_side_effects() {
    let rule = SideEffectOccurredRule;
    let mut input = input_base();
    input.side_effect_occurred = false;
    input.has_text = false;
    input.has_tool_calls = false;
    // Without side effects, this rule does not trigger;
    // the EmptyResponseRule handles this case instead.
    assert_eq!(rule.check(&input).await, None);
}

#[tokio::test]
async fn side_effect_rule_no_trigger_when_tool_calls_present() {
    let rule = SideEffectOccurredRule;
    let mut input = input_base();
    input.side_effect_occurred = true;
    input.has_text = false;
    input.has_tool_calls = true;
    assert_eq!(rule.check(&input).await, None);
}

// ─── SideEffectOccurredRule in engine ──────────────────────

#[tokio::test]
async fn engine_side_effect_violation_category() {
    let engine = HardRuleEngine::new(vec![Box::new(SideEffectOccurredRule)]);
    let mut input = input_base();
    input.side_effect_occurred = true;
    input.has_text = false;
    input.has_tool_calls = false;
    let output = engine.evaluate(&input).await;
    assert_eq!(
        output.suggested_category,
        Some(FailureCategory::SideEffectOccurred)
    );
    assert_eq!(output.violations.len(), 1);
    assert!(matches!(
        output.violations[0],
        HardRuleViolation::SideEffectOccurred
    ));
}

// ─── RetryExhaustedRule ─────────────────────────────────────

#[tokio::test]
async fn retry_rule_no_trigger_when_under_limit() {
    let rule = RetryExhaustedRule { max_retries: 5 };
    let mut input = input_base();
    input.retry_count = 3;
    assert_eq!(rule.check(&input).await, None);
}

#[tokio::test]
async fn retry_rule_triggers_when_at_limit() {
    let rule = RetryExhaustedRule { max_retries: 5 };
    let mut input = input_base();
    input.retry_count = 5;
    assert_eq!(
        rule.check(&input).await,
        Some(HardRuleViolation::RetryExhausted {
            attempts: 5,
            max_retries: 5,
        })
    );
}

#[tokio::test]
async fn retry_rule_triggers_when_over_limit() {
    let rule = RetryExhaustedRule { max_retries: 5 };
    let mut input = input_base();
    input.retry_count = 7;
    assert_eq!(
        rule.check(&input).await,
        Some(HardRuleViolation::RetryExhausted {
            attempts: 7,
            max_retries: 5,
        })
    );
}

// ─── HardRuleEngine ─────────────────────────────────────────

#[tokio::test]
async fn engine_no_violations_returns_healthy() {
    let engine = HardRuleEngine::new(vec![
        Box::new(TimeoutRule {
            threshold_ms: 10_000,
        }),
        Box::new(EmptyResponseRule),
        Box::new(StructuralAnomalyRule),
        Box::new(RetryExhaustedRule { max_retries: 5 }),
    ]);
    let input = input_base();
    let output = engine.evaluate(&input).await;
    assert_eq!(output.status, HealthStatus::Healthy);
    assert!(output.violations.is_empty());
    assert!(output.suggested_category.is_none());
}

#[tokio::test]
async fn engine_captures_multiple_violations() {
    let engine = HardRuleEngine::new(vec![
        Box::new(TimeoutRule {
            threshold_ms: 1_000,
        }),
        Box::new(EmptyResponseRule),
        Box::new(StructuralAnomalyRule),
        Box::new(RetryExhaustedRule { max_retries: 1 }),
    ]);
    let mut input = input_base();
    input.turn_duration_ms = 5_000;
    input.has_text = false;
    input.has_tool_calls = false;
    input.has_thinking = false;
    input.is_structurally_valid = false;
    input.retry_count = 2;
    let output = engine.evaluate(&input).await;
    assert_eq!(output.violations.len(), 4);
    assert!(matches!(output.status, HealthStatus::Unhealthy(_)));
    assert!(output.suggested_category.is_some());
}

#[tokio::test]
async fn engine_first_violation_drives_category() {
    // TimeoutRule first → category should be Retryable
    let engine = HardRuleEngine::new(vec![
        Box::new(TimeoutRule {
            threshold_ms: 1_000,
        }),
        Box::new(EmptyResponseRule),
    ]);
    let mut input = input_base();
    input.turn_duration_ms = 5_000;
    input.has_text = false;
    input.has_tool_calls = false;
    input.has_thinking = false;
    let output = engine.evaluate(&input).await;
    assert_eq!(output.suggested_category, Some(FailureCategory::Retryable));
}

#[tokio::test]
async fn engine_rule_order_does_not_affect_violation_set() {
    // Two engines with reversed rule order should produce the same
    // violation set (order within vec may differ, so compare as sets).
    let input = {
        let mut i = input_base();
        i.turn_duration_ms = 5_000;
        i.has_text = false;
        i
    };

    let engine_a = HardRuleEngine::new(vec![
        Box::new(TimeoutRule {
            threshold_ms: 1_000,
        }),
        Box::new(EmptyResponseRule),
    ]);
    let engine_b = HardRuleEngine::new(vec![
        Box::new(EmptyResponseRule),
        Box::new(TimeoutRule {
            threshold_ms: 1_000,
        }),
    ]);

    let out_a = engine_a.evaluate(&input).await;
    let out_b = engine_b.evaluate(&input).await;

    assert_eq!(out_a.violations.len(), out_b.violations.len());
    // Both should contain exactly one Timeout and one EmptyResponse
    let has_timeout_a = out_a
        .violations
        .iter()
        .any(|v| matches!(v, HardRuleViolation::Timeout { .. }));
    let has_empty_a = out_a
        .violations
        .iter()
        .any(|v| matches!(v, HardRuleViolation::EmptyResponse));
    let has_timeout_b = out_b
        .violations
        .iter()
        .any(|v| matches!(v, HardRuleViolation::Timeout { .. }));
    let has_empty_b = out_b
        .violations
        .iter()
        .any(|v| matches!(v, HardRuleViolation::EmptyResponse));
    assert!(has_timeout_a && has_empty_a);
    assert!(has_timeout_b && has_empty_b);
}
