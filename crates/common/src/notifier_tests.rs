//! Unit tests for [`PlanStateNotifier`] trait and [`NoopNotifier`] default
//! implementation (Step 1.5 — Gap 1).

use super::notifier::{NoopNotifier, PlanStateNotifier};

#[tokio::test]
async fn test_noop_notifier_on_progress_changed_does_not_panic() {
    let notifier = NoopNotifier;
    // on_progress_changed has a default no-op implementation; calling it
    // should not panic or cause any side effects.
    notifier.on_progress_changed("Step 1/3: completed").await;
}

#[tokio::test]
async fn test_noop_notifier_on_plan_completed_does_not_panic() {
    let notifier = NoopNotifier;
    // on_plan_completed has a default no-op implementation; calling it
    // should not panic or cause any side effects.
    notifier.on_plan_completed().await;
}
