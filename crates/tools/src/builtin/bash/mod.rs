//! Built-in BashTool - provides shell command execution capability for agents.
//! Implements timeout control, output truncation with head-preservation,
//! output persistence, and command classification.
//!
//! Step 1.4 of issue #858 added a kill-handle integration path: foreground
//! processes register a [`BashKillHandle`] on the owning
//! `ConversationSession`, background tasks register a
//! [`BackgroundKillHandle`]. The actual `KillHandle` adapter types and
//! the output-processing helpers (`process_output`, `build_result`,
//! etc.) live in the sibling module [`super::bash_kill`] to keep this
//! file under the CONTRIBUTING.md 500-line hard cap.

mod exec;
mod prompt;
mod tool;
mod trust;

pub use tool::BashTool;

pub(crate) use exec::ForegroundOutcome;

#[cfg(test)]
pub(crate) use super::bash_kill::{build_auto_background_result, build_background_result};
#[cfg(test)]
pub(crate) use crate::permission_check::PermDeps;
#[cfg(test)]
pub(crate) use crate::{Tool, ToolCallError, ToolContext};
#[cfg(test)]
pub(crate) use exec::{
    auto_backgroundize_excluded, auto_backgroundize_foreground, backgroundize_child,
    execute_command, execute_foreground_command, handle_foreground_result, handle_timeout_expiry,
    parse_timeout, resolve_cwd, spawn_sh_command, BashExecCtx, ChildHandles, ForegroundContext,
    AUTO_BG_TIMEOUT_CAP_MS, AUTO_BG_TIMEOUT_MS,
};
#[cfg(test)]
pub(crate) use std::sync::{Arc, Mutex};
#[cfg(test)]
pub(crate) use std::time::Duration;

#[cfg(test)]
#[path = "../bash_approval_tests.rs"]
mod approval_tests;

#[cfg(test)]
#[path = "../bash_backgroundize_tests.rs"]
mod backgroundize_tests;

#[cfg(test)]
#[path = "../bash_gap_tests.rs"]
mod gap_tests;

#[cfg(test)]
#[path = "../bash_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../bash_timeout_tests.rs"]
mod timeout_tests;
