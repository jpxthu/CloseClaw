//! Failure-category handler for unhealthy session turns.
//!
//! Maps [`FailureCategory`] + backoff state to a concrete
//! [`RecoverableAction`] that the session execution loop can act on.
//! Maintains per-session retry state via [`BackoffCounter`].

use super::health_types::{FailureCategory, HealthCheckOutput, RecoverableAction, RetryPolicy};

/// Exponential backoff state machine for retryable failures.
///
/// Tracks how many retries have been attempted and computes the
/// delay before the next attempt using the configured [`RetryPolicy`].
#[derive(Debug, Clone)]
pub struct BackoffCounter {
    /// Number of consecutive retry attempts so far.
    attempt_count: u32,
    /// Retry policy parameters.
    policy: RetryPolicy,
}

impl BackoffCounter {
    /// Create a new backoff counter with the given policy.
    pub fn new(policy: RetryPolicy) -> Self {
        Self {
            attempt_count: 0,
            policy,
        }
    }

    /// Increment the attempt counter by one.
    pub fn increment(&mut self) {
        self.attempt_count += 1;
    }

    /// Compute the delay in milliseconds for the next retry.
    ///
    /// Uses exponential backoff: `initial_delay_ms * backoff_multiplier ^ attempt_count`.
    /// Returns `None` if the counter is already exhausted.
    pub fn next_delay(&self) -> Option<u64> {
        if self.is_exhausted() {
            return None;
        }
        let delay = self.policy.initial_delay_ms as f64
            * self
                .policy
                .backoff_multiplier
                .powi(self.attempt_count as i32);
        Some(delay as u64)
    }

    /// Returns `true` if the maximum number of retries has been reached.
    pub fn is_exhausted(&self) -> bool {
        self.attempt_count >= self.policy.max_retries
    }

    /// Reset the counter to zero (e.g. after a successful recovery).
    pub fn reset(&mut self) {
        self.attempt_count = 0;
    }

    /// Returns the current attempt count.
    pub fn attempt_count(&self) -> u32 {
        self.attempt_count
    }
}

/// Default retry instruction injected into the LLM when the response
/// is invalid and the handler decides to retry.
const RETRY_INSTRUCTION: &str =
    "Your previous response was invalid. Please provide a complete, well-structured response.";

/// Default message shown to the user when a retryable failure
/// exhausts all retries.
const RETRY_EXHAUSTED_MESSAGE: &str =
    "The operation failed after multiple retries. Please try again later.";

/// Default message shown to the user when an invalid response
/// exhausts retry instruction attempts.
const INVALID_RESPONSE_EXHAUSTED_MESSAGE: &str =
    "The assistant produced invalid responses repeatedly. Please rephrase your request.";

/// Handler that converts an unhealthy [`HealthCheckOutput`] into a
/// concrete [`RecoverableAction`], maintaining backoff state across
/// calls.
pub struct UnhealthyHandler {
    /// Backoff state for retryable failures.
    retry_backoff: BackoffCounter,
    /// Backoff state for invalid-response retries (counts how many
    /// retry instructions have been injected).
    invalid_backoff: BackoffCounter,
}

impl UnhealthyHandler {
    /// Create a new handler with the given retry policy.
    pub fn new(policy: RetryPolicy) -> Self {
        Self {
            retry_backoff: BackoffCounter::new(policy.clone()),
            invalid_backoff: BackoffCounter::new(policy),
        }
    }

    /// Produce a recovery action based on the health check output.
    ///
    /// The caller should pass the output from [`HardRuleEngine::evaluate`].
    /// If the output indicates `Healthy`, returns `None`.
    pub fn handle(&mut self, output: &HealthCheckOutput) -> Option<RecoverableAction> {
        let category = output.suggested_category.as_ref()?;

        match category {
            FailureCategory::Retryable => {
                self.retry_backoff.increment();

                if self.retry_backoff.is_exhausted() {
                    Some(RecoverableAction::NotifyUser {
                        message: RETRY_EXHAUSTED_MESSAGE.into(),
                    })
                } else {
                    let delay_ms = self.retry_backoff.next_delay().unwrap_or(0);
                    Some(RecoverableAction::Retry {
                        delay_ms,
                        instruction: None,
                    })
                }
            }
            FailureCategory::InvalidResponse => {
                self.invalid_backoff.increment();

                if self.invalid_backoff.is_exhausted() {
                    Some(RecoverableAction::NotifyUser {
                        message: INVALID_RESPONSE_EXHAUSTED_MESSAGE.into(),
                    })
                } else {
                    let delay_ms = self.invalid_backoff.next_delay().unwrap_or(0);
                    Some(RecoverableAction::Retry {
                        delay_ms,
                        instruction: Some(RETRY_INSTRUCTION.into()),
                    })
                }
            }
            FailureCategory::Unrecoverable => Some(RecoverableAction::NotifyUser {
                message: format!(
                    "Unrecoverable error: {}",
                    output
                        .violations
                        .first()
                        .map(|v| format!("{v:?}"))
                        .unwrap_or_else(|| "unknown error".into())
                ),
            }),
            FailureCategory::SideEffectOccurred => Some(RecoverableAction::NotifyUser {
                message: "Side effects detected".to_string()
                    + " — user verification"
                    + " required."
                    + " No rollback will"
                    + " be performed.",
            }),
        }
    }

    /// Reset both retry counters (e.g. after a turn succeeds).
    pub fn reset(&mut self) {
        self.retry_backoff.reset();
        self.invalid_backoff.reset();
    }

    /// Returns a reference to the retry backoff counter.
    pub fn retry_backoff(&self) -> &BackoffCounter {
        &self.retry_backoff
    }

    /// Returns a reference to the invalid-response backoff counter.
    pub fn invalid_backoff(&self) -> &BackoffCounter {
        &self.invalid_backoff
    }
}
