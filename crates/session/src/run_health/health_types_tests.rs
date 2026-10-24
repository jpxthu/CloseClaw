use super::health_types::*;

#[test]
fn test_timeout_maps_to_retryable() {
    let violation = HardRuleViolation::Timeout {
        elapsed_ms: 30_000,
        threshold_ms: 10_000,
    };
    let category: FailureCategory = (&violation).into();
    assert_eq!(category, FailureCategory::Retryable);
}

#[test]
fn test_empty_response_maps_to_invalid_response() {
    let violation = HardRuleViolation::EmptyResponse;
    let category: FailureCategory = (&violation).into();
    assert_eq!(category, FailureCategory::InvalidResponse);
}

#[test]
fn test_structural_anomaly_maps_to_invalid_response() {
    let violation = HardRuleViolation::StructuralAnomaly {
        detail: "missing content field".into(),
    };
    let category: FailureCategory = (&violation).into();
    assert_eq!(category, FailureCategory::InvalidResponse);
}

#[test]
fn test_retry_exhausted_maps_to_unrecoverable() {
    let violation = HardRuleViolation::RetryExhausted {
        attempts: 5,
        max_retries: 5,
    };
    let category: FailureCategory = (&violation).into();
    assert_eq!(category, FailureCategory::Unrecoverable);
}

#[test]
fn test_health_check_output_default_is_empty() {
    let output = HealthCheckOutput {
        status: HealthStatus::Healthy,
        violations: vec![],
        suggested_category: None,
    };
    assert_eq!(output.status, HealthStatus::Healthy);
    assert!(output.violations.is_empty());
    assert!(output.suggested_category.is_none());
}

#[test]
fn test_side_effect_occurred_maps_to_side_effect_occurred() {
    let violation = HardRuleViolation::SideEffectOccurred;
    let category: FailureCategory = (&violation).into();
    assert_eq!(category, FailureCategory::SideEffectOccurred);
}
