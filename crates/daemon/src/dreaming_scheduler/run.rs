//! Scheduler run loop for the DreamingScheduler.
//!
//! Houses the public [`DreamingScheduler::run`] entry point: it picks a
//! [`FirePolicy`] — cron-based wall-clock scheduling when a valid
//! expression is available, the fixed-interval fallback otherwise — and
//! hands it to the single parameterized scheduling loop
//! [`DreamingScheduler::run_loop`].

use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::info;

use super::cron::{cron_next_fire, parse_cron_schedule};
use super::{DreamingScheduler, DreamingSchedulerError, LoopEvent, ShutdownCause};

/// Per-loop scheduling strategy (issue #3606, #3609): how `next_fire`
/// is re-armed after each [`LoopEvent::Fire`], and which handler a
/// [`LoopEvent::ConfigReload`] dispatches to. The initial arm is not
/// policy work: it is derived once in [`DreamingScheduler::run`] from
/// the schedule parsed there.
///
/// The loop skeleton itself — Shutdown / ConfigClosed / MiningNotify
/// arms and the cycle spawning — is shared verbatim by both variants;
/// only these difference points are parameterized. The policy is fixed
/// once at startup for the loop's lifetime: there is deliberately no
/// runtime fixed→cron switch.
#[derive(Debug, Clone, Copy)]
enum FirePolicy {
    /// Cron-based (issue #3608): every re-arm re-derives the instant
    /// from the *current* `schedule` — falling back to
    /// [`DreamingScheduler::fallback_next_fire`] when it is missing or
    /// invalid — so a reloaded schedule keeps governing all subsequent
    /// Fires. `ConfigReload` goes through
    /// [`DreamingScheduler::handle_cron_config_reload`].
    Cron,
    /// Fixed interval: every arm is `now + interval`. `ConfigReload`
    /// goes through [`DreamingScheduler::handle_config_change`].
    FixedInterval(tokio::time::Duration),
}

impl FirePolicy {
    /// Compute the next fire instant per this policy when re-arming
    /// after a `Fire`: cron re-parses the current `schedule`,
    /// fixed-interval uses `now + interval`. The initial arm instead
    /// comes from [`DreamingScheduler::run`], which already parsed the
    /// startup schedule.
    fn next_fire(&self, scheduler: &DreamingScheduler) -> tokio::time::Instant {
        match self {
            FirePolicy::Cron => scheduler.next_fire_from_current_schedule(),
            FirePolicy::FixedInterval(interval) => tokio::time::Instant::now() + *interval,
        }
    }

    /// Apply a `ConfigReload` event per this policy.
    async fn on_config_reload(
        &self,
        scheduler: &mut DreamingScheduler,
        next_fire: &mut tokio::time::Instant,
    ) {
        match self {
            FirePolicy::Cron => scheduler.handle_cron_config_reload(next_fire).await,
            FirePolicy::FixedInterval(_) => scheduler.handle_config_change().await,
        }
    }
}

impl DreamingScheduler {
    /// Run the scheduler loop until `shutdown` signal is received.
    ///
    /// Uses cron-based scheduling when a valid expression is available;
    /// falls back to a fixed interval otherwise. Each cycle: dreaming
    /// pipeline first, then mining scan.
    pub async fn run(&mut self, mut shutdown: watch::Receiver<()>) {
        let cron_schedule = self.schedule.as_deref().and_then(parse_cron_schedule);

        let (policy, next_fire) = if let Some(ref sched) = cron_schedule {
            info!(
                schedule = %sched.display(),
                "DreamingScheduler started with cron schedule"
            );
            (FirePolicy::Cron, cron_next_fire(sched))
        } else {
            let interval_secs = self.config.dreaming_interval_secs();
            info!(
                "DreamingScheduler started with fixed interval {}s",
                interval_secs
            );
            let interval = tokio::time::Duration::from_secs(interval_secs);
            (
                FirePolicy::FixedInterval(interval),
                tokio::time::Instant::now() + interval,
            )
        };

        self.run_loop(&mut shutdown, policy, next_fire).await;
    }

    /// Unified scheduling loop (issue #3606, #3609): one skeleton serves
    /// both strategies — the Shutdown / ConfigClosed / MiningNotify arms
    /// are shared verbatim — and the per-strategy difference points
    /// (reload handler selection, Fire re-arm) live in [`FirePolicy`].
    /// The initial `next_fire` is passed in by [`DreamingScheduler::run`],
    /// derived from the schedule parsed there without a re-parse.
    async fn run_loop(
        &mut self,
        shutdown: &mut watch::Receiver<()>,
        policy: FirePolicy,
        mut next_fire: tokio::time::Instant,
    ) {
        let mut active_task: Option<JoinHandle<Result<(), DreamingSchedulerError>>> = None;

        loop {
            match self.next_event(shutdown, next_fire).await {
                LoopEvent::Shutdown => {
                    Self::finish_shutdown(&mut active_task, ShutdownCause::Signal).await;
                    break;
                }
                LoopEvent::ConfigClosed => {
                    Self::finish_shutdown(&mut active_task, ShutdownCause::ConfigClosed).await;
                    break;
                }
                LoopEvent::ConfigReload => policy.on_config_reload(self, &mut next_fire).await,
                LoopEvent::MiningNotify(session_id) => {
                    self.handle_mining_notify(&session_id).await;
                }
                LoopEvent::Fire => {
                    self.spawn_or_skip_cycle(&mut active_task).await;
                    next_fire = policy.next_fire(self);
                }
            }
        }
    }

    /// Fixed-interval fallback instant: `now + dreaming_interval_secs`.
    ///
    /// Single definition of the fallback the cron scheduling degrades to
    /// when the schedule is missing or invalid — shared verbatim by
    /// [`DreamingScheduler::next_fire_from_current_schedule`] and the
    /// fallback branch of
    /// [`DreamingScheduler::handle_cron_config_reload`] so the same
    /// semantics cannot drift apart across two copies.
    pub(super) fn fallback_next_fire(&self) -> tokio::time::Instant {
        tokio::time::Instant::now()
            + tokio::time::Duration::from_secs(self.config.dreaming_interval_secs())
    }

    /// Recompute the next fire instant from the current `schedule`
    /// (issue #3608) when re-arming after a `Fire`: re-parse the
    /// expression so a reloaded schedule keeps governing every re-arm,
    /// falling back to [`DreamingScheduler::fallback_next_fire`] when it
    /// is missing or invalid.
    fn next_fire_from_current_schedule(&self) -> tokio::time::Instant {
        match self.schedule.as_deref().and_then(parse_cron_schedule) {
            Some(sched) => cron_next_fire(&sched),
            None => self.fallback_next_fire(),
        }
    }
}
