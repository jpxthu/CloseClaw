//! Hard-rule health detection for session turns.
//!
//! Each rule is a deterministic, LLM-free check that inspects the
//! turn snapshot (`HealthCheckInput`) and returns a violation if
//! the session is unhealthy. The [`HardRuleEngine`] aggregates all
//! rule results into a single verdict.

use async_trait::async_trait;

use super::health_types::{
    FailureCategory, HardRuleViolation, HealthCheckInput, HealthCheckOutput, HealthStatus,
};

/// A single hard-rule check.
///
/// Implementations must be deterministic and not depend on any
/// external state beyond the provided input snapshot.
#[async_trait]
pub trait HardRule: Send + Sync {
    /// Evaluate the rule against the given turn snapshot.
    ///
    /// Returns `Some(violation)` if the rule is violated, `None` otherwise.
    async fn check(&self, input: &HealthCheckInput) -> Option<HardRuleViolation>;
}

/// Rule: turn elapsed time exceeded the configured threshold.
pub struct TimeoutRule {
    /// Threshold in milliseconds.
    pub threshold_ms: u64,
}

#[async_trait]
impl HardRule for TimeoutRule {
    async fn check(&self, input: &HealthCheckInput) -> Option<HardRuleViolation> {
        if input.turn_duration_ms > self.threshold_ms {
            Some(HardRuleViolation::Timeout {
                elapsed_ms: input.turn_duration_ms,
                threshold_ms: self.threshold_ms,
            })
        } else {
            None
        }
    }
}

/// Rule: LLM returned no usable content.
///
/// A response is considered empty when it contains no text, no tool
/// calls, AND no thinking output.
pub struct EmptyResponseRule;

#[async_trait]
impl HardRule for EmptyResponseRule {
    async fn check(&self, input: &HealthCheckInput) -> Option<HardRuleViolation> {
        if !input.has_text && !input.has_tool_calls && !input.has_thinking {
            Some(HardRuleViolation::EmptyResponse)
        } else {
            None
        }
    }
}

/// Rule: response structure is malformed or missing required fields.
pub struct StructuralAnomalyRule;

#[async_trait]
impl HardRule for StructuralAnomalyRule {
    async fn check(&self, input: &HealthCheckInput) -> Option<HardRuleViolation> {
        if !input.is_structurally_valid {
            Some(HardRuleViolation::StructuralAnomaly {
                detail: input
                    .structural_anomaly_detail
                    .clone()
                    .unwrap_or_else(|| "unknown structural anomaly".into()),
            })
        } else {
            None
        }
    }
}

/// Rule: retry counter has reached or exceeded the configured maximum.
pub struct RetryExhaustedRule {
    /// Maximum number of allowed retries.
    pub max_retries: u32,
}

#[async_trait]
impl HardRule for RetryExhaustedRule {
    async fn check(&self, input: &HealthCheckInput) -> Option<HardRuleViolation> {
        if input.retry_count >= self.max_retries {
            Some(HardRuleViolation::RetryExhausted {
                attempts: input.retry_count,
                max_retries: self.max_retries,
            })
        } else {
            None
        }
    }
}

/// Engine that aggregates multiple hard rules.
///
/// Holds a list of rule implementations and evaluates them
/// sequentially, collecting all violations.
pub struct HardRuleEngine {
    rules: Vec<Box<dyn HardRule>>,
}

impl HardRuleEngine {
    /// Create a new engine with the given rule set.
    pub fn new(rules: Vec<Box<dyn HardRule>>) -> Self {
        Self { rules }
    }

    /// Evaluate all rules against the turn snapshot.
    ///
    /// Returns a [`HealthCheckOutput`] containing the aggregate status,
    /// all detected violations, and a suggested failure category
    /// derived from the most severe violation.
    pub async fn evaluate(&self, input: &HealthCheckInput) -> HealthCheckOutput {
        let mut violations = Vec::new();

        for rule in &self.rules {
            if let Some(violation) = rule.check(input).await {
                violations.push(violation);
            }
        }

        if violations.is_empty() {
            HealthCheckOutput {
                status: HealthStatus::Healthy,
                violations: Vec::new(),
                suggested_category: None,
            }
        } else {
            // Use the first violation's category as the suggested category.
            // If multiple violations exist, they are all recorded but the
            // first one drives the recovery strategy.
            let suggested_category = violations.first().map(FailureCategory::from);

            HealthCheckOutput {
                status: HealthStatus::Unhealthy(
                    suggested_category
                        .clone()
                        .unwrap_or(FailureCategory::Unrecoverable),
                ),
                violations,
                suggested_category,
            }
        }
    }
}
