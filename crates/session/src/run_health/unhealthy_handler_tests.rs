//! Tests for unhealthy handler and backoff counter.

use super::health_types::*;
use super::unhealthy_handler::*;

// ─── helpers ────────────────────────────────────────────────

fn policy(max_retries: u32, initial_delay_ms: u64, backoff_multiplier: f64) -> RetryPolicy {
    RetryPolicy {
        max_retries,
        initial_delay_ms,
        backoff_multiplier,
    }
}

fn output_with_category(category: FailureCategory) -> HealthCheckOutput {
    HealthCheckOutput {
        status: HealthStatus::Unhealthy(category.clone()),
        violations: vec![HardRuleViolation::Timeout {
            elapsed_ms: 15_000,
            threshold_ms: 10_000,
        }],
        suggested_category: Some(category),
    }
}

fn healthy_output() -> HealthCheckOutput {
    HealthCheckOutput {
        status: HealthStatus::Healthy,
        violations: vec![],
        suggested_category: None,
    }
}

// ─── BackoffCounter ─────────────────────────────────────────

#[test]
fn backoff_counter_initial_state() {
    let bc = BackoffCounter::new(policy(3, 100, 2.0));
    assert_eq!(bc.attempt_count(), 0);
    assert!(!bc.is_exhausted());
    assert_eq!(bc.next_delay(), Some(100));
}

#[test]
fn backoff_counter_increment() {
    let mut bc = BackoffCounter::new(policy(3, 100, 2.0));
    bc.increment();
    assert_eq!(bc.attempt_count(), 1);
    assert!(!bc.is_exhausted());
}

#[test]
fn backoff_counter_delay_calculation() {
    let mut bc = BackoffCounter::new(policy(5, 100, 2.0));
    // attempt 0: 100 * 2^0 = 100
    assert_eq!(bc.next_delay(), Some(100));
    bc.increment();
    // attempt 1: 100 * 2^1 = 200
    assert_eq!(bc.next_delay(), Some(200));
    bc.increment();
    // attempt 2: 100 * 2^2 = 400
    assert_eq!(bc.next_delay(), Some(400));
}

#[test]
fn backoff_counter_is_exhausted() {
    let mut bc = BackoffCounter::new(policy(3, 100, 2.0));
    bc.increment();
    bc.increment();
    assert!(!bc.is_exhausted()); // count=2, max=3
    bc.increment();
    assert!(bc.is_exhausted()); // count=3, max=3
}

#[test]
fn backoff_counter_exhausted_returns_none_delay() {
    let mut bc = BackoffCounter::new(policy(2, 100, 2.0));
    bc.increment();
    bc.increment();
    assert!(bc.is_exhausted());
    assert_eq!(bc.next_delay(), None);
}

#[test]
fn backoff_counter_reset() {
    let mut bc = BackoffCounter::new(policy(3, 100, 2.0));
    bc.increment();
    bc.increment();
    assert_eq!(bc.attempt_count(), 2);
    bc.reset();
    assert_eq!(bc.attempt_count(), 0);
    assert!(!bc.is_exhausted());
    assert_eq!(bc.next_delay(), Some(100));
}

// ─── UnhealthyHandler ───────────────────────────────────────

#[test]
fn handler_healthy_returns_none() {
    let mut handler = UnhealthyHandler::new(policy(3, 100, 2.0));
    let action = handler.handle(&healthy_output());
    assert!(action.is_none());
}

#[test]
fn handler_retryable_retries_with_delay() {
    let mut handler = UnhealthyHandler::new(policy(3, 100, 2.0));
    let action = handler
        .handle(&output_with_category(FailureCategory::Retryable))
        .unwrap();
    assert!(matches!(
        action,
        RecoverableAction::Retry {
            delay_ms: 200,
            instruction: None,
        }
    ));
}

#[test]
fn handler_retryable_exhausted_notifies_user() {
    let mut handler = UnhealthyHandler::new(policy(2, 100, 2.0));
    handler.handle(&output_with_category(FailureCategory::Retryable));
    handler.handle(&output_with_category(FailureCategory::Retryable));
    let action = handler
        .handle(&output_with_category(FailureCategory::Retryable))
        .unwrap();
    assert!(matches!(action, RecoverableAction::NotifyUser { .. }));
}

#[test]
fn handler_invalid_response_retries_with_instruction() {
    let mut handler = UnhealthyHandler::new(policy(3, 100, 2.0));
    let action = handler
        .handle(&output_with_category(FailureCategory::InvalidResponse))
        .unwrap();
    match action {
        RecoverableAction::Retry {
            delay_ms: _,
            instruction,
        } => {
            assert!(instruction.is_some());
            assert!(instruction.unwrap().contains("invalid"));
        }
        _ => panic!("Expected Retry with instruction"),
    }
}

#[test]
fn handler_invalid_response_exhausted_notifies_user() {
    let mut handler = UnhealthyHandler::new(policy(2, 100, 2.0));
    handler.handle(&output_with_category(FailureCategory::InvalidResponse));
    handler.handle(&output_with_category(FailureCategory::InvalidResponse));
    let action = handler
        .handle(&output_with_category(FailureCategory::InvalidResponse))
        .unwrap();
    assert!(matches!(action, RecoverableAction::NotifyUser { .. }));
}

#[test]
fn handler_unrecoverable_immediately_notifies_user() {
    let mut handler = UnhealthyHandler::new(policy(5, 100, 2.0));
    let action = handler
        .handle(&output_with_category(FailureCategory::Unrecoverable))
        .unwrap();
    assert!(matches!(action, RecoverableAction::NotifyUser { .. }));
}

#[test]
fn handler_side_effect_notifies_user_for_verification() {
    let mut handler = UnhealthyHandler::new(policy(3, 100, 2.0));
    let action = handler
        .handle(&output_with_category(FailureCategory::SideEffectOccurred))
        .unwrap();
    match action {
        RecoverableAction::NotifyUser { message } => {
            assert!(message.contains("user verification"));
            assert!(message.contains("No rollback"));
        }
        _ => panic!("Expected NotifyUser, got {action:?}"),
    }
}

#[test]
fn handler_retryable_exhaustion_escalates_to_unrecoverable() {
    // After exhausting retries, next retryable call yields NotifyUser
    let mut handler = UnhealthyHandler::new(policy(1, 100, 2.0));
    // First call: count becomes 1 = max, so exhausted → NotifyUser
    let action = handler
        .handle(&output_with_category(FailureCategory::Retryable))
        .unwrap();
    assert!(matches!(action, RecoverableAction::NotifyUser { .. }));
}

#[test]
fn handler_consecutive_retries_independent_backoff_counters() {
    let mut handler = UnhealthyHandler::new(policy(3, 100, 2.0));

    // Exhaust retry backoff
    handler.handle(&output_with_category(FailureCategory::Retryable));
    handler.handle(&output_with_category(FailureCategory::Retryable));
    handler.handle(&output_with_category(FailureCategory::Retryable));

    // Invalid response backoff should still be fresh
    let action = handler
        .handle(&output_with_category(FailureCategory::InvalidResponse))
        .unwrap();
    match action {
        RecoverableAction::Retry {
            delay_ms: _,
            instruction,
        } => {
            assert!(instruction.is_some());
        }
        _ => panic!("Expected Retry with instruction"),
    }
}

#[test]
fn handler_reset_restores_both_counters() {
    let mut handler = UnhealthyHandler::new(policy(2, 100, 2.0));

    // Exhaust retry backoff
    handler.handle(&output_with_category(FailureCategory::Retryable));
    handler.handle(&output_with_category(FailureCategory::Retryable));

    // Reset
    handler.reset();

    // Should be able to retry again
    let action = handler
        .handle(&output_with_category(FailureCategory::Retryable))
        .unwrap();
    assert!(matches!(
        action,
        RecoverableAction::Retry {
            delay_ms: 200,
            instruction: None,
        }
    ));
}

// ─── State transition: healthy → unhealthy → retry → healthy ──

#[test]
fn full_cycle_healthy_unhealthy_retry_healthy() {
    let mut handler = UnhealthyHandler::new(policy(3, 100, 2.0));

    // Step 1: Healthy → None
    assert!(handler.handle(&healthy_output()).is_none());

    // Step 2: Unhealthy (retryable) → Retry
    let action = handler
        .handle(&output_with_category(FailureCategory::Retryable))
        .unwrap();
    assert!(matches!(action, RecoverableAction::Retry { .. }));

    // Step 3: Reset (simulating recovery) → back to healthy state
    handler.reset();
    assert!(handler.handle(&healthy_output()).is_none());
}
