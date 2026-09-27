//! Daemon lifecycle: start, run, and shutdown phases.
//!
//! Runtime and bootstrap implementations live in submodules; this file
//! only holds module declarations and re-exports (CONTRIBUTING.md 模块规范).

mod bg_task_helpers;
mod bootstrap;
mod plugin_init;
mod run;

pub(crate) use run::TaskStopStatus;
