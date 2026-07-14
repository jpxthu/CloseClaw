//! Run health checker — orchestrates the full health-check pipeline.
//!
//! Combines hard-rule evaluation, hook review, and unhealthy handling
//! into a single [`RunHealthChecker::check_turn`] entry point that
//! the session execution loop calls at turn boundaries.
//!
//! Design reference: `docs/design/session/run-health.md`.

use super::hard_rules::HardRuleEngine;
use super::health_types::{HealthCheckInput, HealthStatus, HookContext, RecoverableAction};
use super::hook_reviewer::HookReviewer;
use super::unhealthy_handler::UnhealthyHandler;

/// Result of a full health-check pipeline run.
///
/// Contains the overall status, the suggested recovery action (if any),
/// and a flag indicating whether any hook flagged the turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunHealthVerdict {
    /// Overall health status after hard rules and hook review.
    pub status: HealthStatus,
    /// Suggested recovery action, if the turn is unhealthy.
    pub action: Option<RecoverableAction>,
}

/// Orchestrates the run-health pipeline for a single session turn.
///
/// Holds the three components of the pipeline and executes them
/// in order: hard-rule evaluation → hook review → unhealthy handling.
pub struct RunHealthChecker {
    hard_rules: HardRuleEngine,
    hook_reviewer: Option<HookReviewer>,
    unhealthy_handler: UnhealthyHandler,
}

impl RunHealthChecker {
    /// Create a new checker with the given components.
    pub fn new(
        hard_rules: HardRuleEngine,
        hook_reviewer: Option<HookReviewer>,
        unhealthy_handler: UnhealthyHandler,
    ) -> Self {
        Self {
            hard_rules,
            hook_reviewer,
            unhealthy_handler,
        }
    }

    /// Run the full health-check pipeline for a single turn.
    ///
    /// # Flow
    ///
    /// 1. **Hard-rule evaluation** — deterministic checks (timeout,
    ///    empty response, structural anomaly, retry exhaustion).
    /// 2. **Hook review** — LLM-based quality gates (only if hard
    ///    rules passed and hooks are configured).
    /// 3. **Unhealthy handling** — maps any violation/flag to a
    ///    [`RecoverableAction`].
    pub async fn check_turn(
        &mut self,
        input: &HealthCheckInput,
        hook_context: Option<&HookContext>,
    ) -> RunHealthVerdict {
        // 1. Hard-rule evaluation.
        let hard_output = self.hard_rules.evaluate(input).await;

        // If hard rules are healthy, optionally run hook review.
        if hard_output.status == HealthStatus::Healthy {
            if let Some(ref reviewer) = self.hook_reviewer {
                if let Some(ctx) = hook_context {
                    let hook_verdicts = reviewer.review(ctx).await;
                    let any_flagged = hook_verdicts.iter().any(|v| v.flag);
                    if any_flagged {
                        // Build a synthetic unhealthy output for the handler.
                        let flag_reasons: Vec<String> = hook_verdicts
                            .iter()
                            .filter(|v| v.flag)
                            .map(|v| v.reason.clone())
                            .collect();
                        let synthetic_output = super::health_types::HealthCheckOutput {
                            status: HealthStatus::Unhealthy(
                                super::health_types::FailureCategory::InvalidResponse,
                            ),
                            violations: vec![
                                super::health_types::HardRuleViolation::StructuralAnomaly {
                                    detail: format!("hooks flagged: {}", flag_reasons.join("; ")),
                                },
                            ],
                            suggested_category: Some(
                                super::health_types::FailureCategory::InvalidResponse,
                            ),
                        };
                        if let Some(action) = self.unhealthy_handler.handle(&synthetic_output) {
                            return RunHealthVerdict {
                                status: HealthStatus::Unhealthy(
                                    super::health_types::FailureCategory::InvalidResponse,
                                ),
                                action: Some(action),
                            };
                        }
                    }
                }
            }
            // Healthy path — no action.
            return RunHealthVerdict {
                status: HealthStatus::Healthy,
                action: None,
            };
        }

        // 2. Hard rules found violations — map to recovery action.
        let action = self.unhealthy_handler.handle(&hard_output);
        RunHealthVerdict {
            status: hard_output.status,
            action,
        }
    }
}
