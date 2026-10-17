//! Loop-event await and shutdown epilogue for the DreamingScheduler.
//!
//! Houses the `select!`-based [`DreamingScheduler::next_event`] event
//! classifier, the shared [`DreamingScheduler::finish_shutdown`]
//! epilogue, and the `classify_config_recv` pure function they rely on.

use closeclaw_config::{ConfigChangeEvent, ConfigSection};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{error, info, warn};

use super::{
    wait_for_active_task, ConfigRecvOutcome, DreamingScheduler, DreamingSchedulerError, LoopEvent,
    ShutdownCause,
};

/// Classify one config broadcast receive result.
///
/// Pure function, kept private and unit-testable (issue #3577): the
/// four outcomes map to reload / ignore / warn-and-continue /
/// terminate respectively.
fn classify_config_recv(result: Result<ConfigChangeEvent, RecvError>) -> ConfigRecvOutcome {
    match result {
        Ok(ConfigChangeEvent::Reloaded {
            section: ConfigSection::Memory,
            ..
        }) => ConfigRecvOutcome::MemoryReload,
        Ok(_) => ConfigRecvOutcome::Ignore,
        Err(RecvError::Lagged(n)) => ConfigRecvOutcome::Lagged(n),
        Err(RecvError::Closed) => ConfigRecvOutcome::Closed,
    }
}

impl DreamingScheduler {
    /// Await the next loop event (shared by the unified scheduling loop).
    ///
    /// Races the four branches previously duplicated across the two
    /// per-strategy scheduling loops, unified in [`Self::run_loop`] —
    /// shutdown watch, config broadcast, mining-notify channel and the
    /// `next_fire` timer — and classifies the winner as a
    /// [`LoopEvent`]. Only Memory-section reloads surface as
    /// [`LoopEvent::ConfigReload`]; other sections are consumed,
    /// ignored and polling continues. `Err(RecvError::Lagged)` logs a
    /// warning (with the number of lost events) and keeps polling;
    /// `Err(RecvError::Closed)` logs an error and returns
    /// [`LoopEvent::ConfigClosed`] so the caller exits via the shutdown
    /// path (issue #3577).
    ///
    /// Cancellation accounting matches the pre-refactor `select!`:
    /// broadcast `recv()`, watch `changed()` and `sleep_until` are all
    /// cancel-safe, so discarding the losing futures per iteration
    /// loses no signal; the `next_fire` deadline is rebuilt unchanged.
    pub(super) async fn next_event(
        &mut self,
        shutdown: &mut watch::Receiver<()>,
        next_fire: Instant,
    ) -> LoopEvent {
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    return LoopEvent::Shutdown;
                }
                result = self.config_rx.recv() => {
                    match classify_config_recv(result) {
                        ConfigRecvOutcome::MemoryReload => {
                            return LoopEvent::ConfigReload;
                        }
                        ConfigRecvOutcome::Ignore => {}
                        ConfigRecvOutcome::Lagged(n) => {
                            warn!(
                                lost = n,
                                "config broadcast receiver lagged, events lost; continuing"
                            );
                        }
                        ConfigRecvOutcome::Closed => {
                            error!(
                                "config broadcast channel closed, stopping dreaming scheduler"
                            );
                            return LoopEvent::ConfigClosed;
                        }
                    }
                }
                Some(session_id) = async {
                    match &mut self.mining_notify_rx {
                        Some(rx) => rx.recv().await,
                        None => None,
                    }
                } => {
                    return LoopEvent::MiningNotify(session_id);
                }
                _ = tokio::time::sleep_until(next_fire) => {
                    return LoopEvent::Fire;
                }
            }
        }
    }

    /// Shared shutdown epilogue: log the exit cause truthfully and give
    /// an in-flight cycle the grace budget to finish. Termination causes
    /// are the shutdown watch ([`ShutdownCause::Signal`]) and the closed
    /// config broadcast ([`ShutdownCause::ConfigClosed`], issue #3577) —
    /// the latter gets its own message, not the shutdown-signal text.
    /// The caller breaks its loop afterwards.
    pub(super) async fn finish_shutdown(
        active_task: &mut Option<JoinHandle<Result<(), DreamingSchedulerError>>>,
        cause: ShutdownCause,
    ) {
        match cause {
            ShutdownCause::Signal => {
                info!("DreamingScheduler received shutdown signal, exiting");
            }
            ShutdownCause::ConfigClosed => {
                info!("DreamingScheduler config broadcast closed, exiting");
            }
        }
        if let Some(handle) = active_task.take() {
            wait_for_active_task(handle).await;
        }
    }
}

// ── Config recv classification tests ──────────────────────────────────

/// Child test module (separate file, same pattern as the config crate's
/// `events.rs` → `events_tests.rs`): needs `super::*` access to the
/// private `classify_config_recv` and the shared `ConfigRecvOutcome`,
/// which a sibling module cannot reach.
#[cfg(test)]
#[path = "../dreaming_scheduler_classify_tests.rs"]
mod classify_tests;
