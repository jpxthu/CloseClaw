//! Sandbox engine subprocess entry point.
//!
//! When `SANDBOX_ENGINE=1` is set, the binary acts as a permission engine
//! subprocess instead of the normal CLI. This module encapsulates the
//! environment-variable branching logic so that the detection logic is
//! independently testable without mutating the process environment.

use std::path::PathBuf;

use closeclaw_permission::{sandbox::run_engine_subprocess, RuleSet};

/// Core detection logic — pure function, no side effects.
///
/// Given explicit env-var values, returns:
/// - `Some(Ok((ipc_path, rules)))` when engine mode should activate
/// - `Some(Err(..))` when engine mode was requested but misconfigured
/// - `None` when engine mode is not active (normal CLI flow)
pub fn detect_engine_mode_inner(
    engine_flag: Option<&str>,
    ipc_path: Option<&str>,
) -> Option<Result<(PathBuf, RuleSet), anyhow::Error>> {
    if engine_flag != Some("1") {
        return None;
    }
    let path_str = match ipc_path {
        Some(p) if !p.is_empty() => p,
        _ => {
            return Some(Err(anyhow::anyhow!(
                "SANDBOX_IPC_PATH must be set when SANDBOX_ENGINE=1"
            )));
        }
    };
    let rules = RuleSet::default();
    Some(Ok((PathBuf::from(path_str), rules)))
}

#[cfg(test)]
#[path = "sandbox_engine_tests.rs"]
mod sandbox_engine_tests;

/// Try to run as sandbox engine subprocess.
///
/// Returns `Some(Ok(()))` if the process entered engine mode and completed,
/// or `Some(Err(..))` if engine mode was entered but failed.
/// Returns `None` if `SANDBOX_ENGINE` is not set, meaning normal CLI flow.
pub async fn try_run_engine_subprocess() -> Option<anyhow::Result<()>> {
    let engine_flag = std::env::var("SANDBOX_ENGINE").ok();
    let ipc_path = std::env::var("SANDBOX_IPC_PATH").ok();

    match detect_engine_mode_inner(engine_flag.as_deref(), ipc_path.as_deref()) {
        None => None,
        Some(Err(e)) => Some(Err(e)),
        Some(Ok((path, rules))) => Some(run_engine_subprocess(path, rules).await),
    }
}
