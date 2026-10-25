// Topic-split module for `process` tests (issue #4050): each submodule owns
// one theme of the process lifecycle, mirroring the design doc's dimensions
// (alive detection / PID registration / termination / shutdown signals).
// Submodule declarations are `#[cfg(test)]`-gated so `scripts/test-audit.sh`
// mount resolution keeps covering the topic files (no scan blind spot).

#[cfg(test)]
mod alive;
#[cfg(test)]
mod kill_self;
#[cfg(test)]
mod pid_file;
#[cfg(test)]
mod shutdown_signals;
#[cfg(test)]
mod signal;
#[cfg(test)]
mod spawn;
#[cfg(test)]
mod stop_daemon;
#[cfg(test)]
mod support;

pub(crate) use support::{
    in_signal_test_child, kill_self, open_child_subscription, spawn_sleep_child, TestSelfSignal,
};
