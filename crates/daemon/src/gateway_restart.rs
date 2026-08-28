//! Gateway restart orchestration.
//!
//! Manages the state machine for config-triggered gateway restarts:
//! [`RestartState::Idle`] → [`RestartState::Pending`] → [`RestartState::Executing`].
//!
//! Restart-class config changes are collected in the `Pending` state;
//! the actual rebuild happens in a later step once an idle window is found.

use std::fmt;
use tokio::sync::watch;

/// State of the gateway restart lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartState {
    /// No restart pending — system running normally.
    Idle,
    /// A restart has been requested; `changes` lists affected config paths.
    Pending { changes: Vec<String> },
    /// Gateway rebuild is in progress.
    Executing,
}

impl fmt::Display for RestartState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => write!(f, "Idle"),
            Self::Pending { changes } => {
                write!(f, "Pending({})", changes.join(", "))
            }
            Self::Executing => write!(f, "Executing"),
        }
    }
}

/// Handle to the restart-state watch channel.
///
/// Lightweight handle kept on the [`Daemon`] struct.
/// The receiver half is consumed by the watchdog task (spawned later).
pub(crate) struct RestartHandle {
    tx: watch::Sender<RestartState>,
}

impl RestartHandle {
    /// Create a handle in the `Idle` state.
    pub(crate) fn new() -> Self {
        let (tx, _rx) = watch::channel(RestartState::Idle);
        Self { tx }
    }

    /// Current state snapshot.
    #[allow(dead_code)]
    pub(crate) fn state(&self) -> RestartState {
        self.tx.borrow().clone()
    }

    /// Return a **new** receiver that will see future state changes.
    ///
    /// The caller (watchdog task) should `changed().await` in a loop
    /// to react to transitions.
    #[allow(dead_code)]
    pub(crate) fn subscribe(&self) -> watch::Receiver<RestartState> {
        self.tx.subscribe()
    }
}

// ---------------------------------------------------------------------------
// Daemon methods (impl block)
// ---------------------------------------------------------------------------

impl crate::Daemon {
    /// Request a gateway restart for the given change summaries.
    ///
    /// - If currently **Idle**: transitions to `Pending` and returns
    ///   `true` (caller should spawn the watchdog).
    /// - If currently **Pending**: merges the new `changes` into the
    ///   existing list and returns `false` (watchdog already running).
    /// - If currently **Executing** or **Pending with no new changes**:
    ///   returns `false` — no action needed.
    #[allow(dead_code)]
    pub(crate) fn request_gateway_restart(&self, changes: Vec<String>) -> bool {
        let mut current = self.restart_state.tx.borrow().clone();
        match current {
            RestartState::Idle => {
                let new_state = RestartState::Pending { changes };
                let _ = self.restart_state.tx.send(new_state);
                true
            }
            RestartState::Pending {
                changes: ref mut existing,
            } => {
                // Merge: add only non-duplicate entries.
                for c in &changes {
                    if !existing.contains(c) {
                        existing.push(c.clone());
                    }
                }
                let _ = self.restart_state.tx.send(current);
                false
            }
            RestartState::Executing => false,
        }
    }

    /// Cancel a pending restart, returning to `Idle`.
    ///
    /// Returns `true` if a pending restart was cancelled, `false` if
    /// there was nothing to cancel (already Idle or Executing).
    #[allow(dead_code)]
    pub(crate) fn cancel_pending_restart(&self) -> bool {
        let current = self.restart_state.tx.borrow().clone();
        match current {
            RestartState::Pending { .. } => {
                let _ = self.restart_state.tx.send(RestartState::Idle);
                true
            }
            _ => false,
        }
    }

    /// Force an immediate gateway restart (skip idle-window wait).
    ///
    /// If currently **Pending**, transitions directly to `Executing` so
    /// the watchdog proceeds immediately. If currently **Idle**, starts
    /// a fresh restart cycle by going to `Pending` with the given
    /// `changes` and returning `true` (caller should spawn watchdog).
    /// If currently **Executing**, returns `false`.
    ///
    /// Returns `true` if the caller should spawn the watchdog task.
    #[allow(dead_code)]
    pub(crate) fn force_gateway_restart(&self, changes: Vec<String>) -> bool {
        let current = self.restart_state.tx.borrow().clone();
        match current {
            RestartState::Pending { .. } => {
                // Overwrite with provided changes and signal the watchdog.
                let _ = self
                    .restart_state
                    .tx
                    .send(RestartState::Pending { changes });
                // Tell caller to spawn (or re-spawn) the watchdog.
                true
            }
            RestartState::Idle => {
                let _ = self
                    .restart_state
                    .tx
                    .send(RestartState::Pending { changes });
                true
            }
            RestartState::Executing => false,
        }
    }
}
