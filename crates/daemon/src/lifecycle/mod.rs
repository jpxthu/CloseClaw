//! Daemon lifecycle: start, run, and shutdown phases.
//!
//! Runtime and bootstrap implementations live in submodules; this file
//! only holds module declarations and re-exports (CONTRIBUTING.md 模块规范).

mod bg_task_helpers;
mod bootstrap;
mod plugin_init;
mod run;

#[cfg(test)]
pub(crate) use bg_task_helpers::{BG_TASK_ABORT_GRACE_SECS, BG_TASK_JOIN_TIMEOUT_SECS};
#[cfg(test)]
pub(crate) use run::resolve_pid_file_path;
pub(crate) use run::TaskStopStatus;
