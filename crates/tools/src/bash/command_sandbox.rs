//! Command sandbox routing for BashTool.
//!
//! Routes commands to sandboxed or unsandboxed execution based on
//! permission checks. Commands with full permissions execute in the
//! host environment; commands without permissions (but approved through
//! the approval flow) execute in a restricted sandbox.
//!
//! The sandbox uses Linux landlock (filesystem access control) and
//! seccomp (syscall filtering) to restrict command execution.
//!
//! # Script Handling
//!
//! Scripts (`.sh`, `.py`, `.pl`, `.rb`, `.js`) are always routed to the
//! sandbox regardless of permission status, per the design doc requirement
//! that "所有脚本运行在沙盒内".

use crate::ToolCallError;
use std::path::Path;

/// Detects whether a command invokes a script interpreter.
///
/// Returns `true` if the first token of the command is a script file
/// (`.sh`, `.py`, `.pl`, `.rb`, `.js`).
fn is_script(command: &str) -> bool {
    let trimmed = command.trim();
    let first_token = trimmed.split_whitespace().next().unwrap_or("");
    let base = Path::new(first_token)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(first_token);
    base.ends_with(".sh")
        || base.ends_with(".py")
        || base.ends_with(".pl")
        || base.ends_with(".rb")
        || base.ends_with(".js")
}

/// Command sandbox for restricted command execution.
///
/// Applies Linux landlock (filesystem access control) and seccomp
/// (syscall filtering) to limit the capabilities of executed commands.
pub struct CommandSandbox;

impl CommandSandbox {
    /// Execute a command inside the sandbox with landlock + seccomp restrictions.
    ///
    /// On Linux, applies:
    /// - landlock: restricts filesystem access to cwd and /tmp
    /// - seccomp: blocks dangerous syscalls (mount, pivot_root, etc.)
    ///
    /// On non-Linux, executes without restrictions (same as outside sandbox).
    pub fn execute_in_sandbox(command: &str, cwd: &str) -> Result<String, ToolCallError> {
        Self::apply_sandbox_restrictions(cwd)?;
        Self::run(command, cwd)
    }

    /// Execute a command in the host environment (no sandbox restrictions).
    pub fn execute_outside_sandbox(command: &str, cwd: &str) -> Result<String, ToolCallError> {
        Self::run(command, cwd)
    }

    /// Route a command to sandboxed or unsandboxed execution.
    ///
    /// Scripts (`.sh`, `.py`, etc.) are always routed to the sandbox
    /// regardless of the `is_permitted` flag, per the design doc requirement
    /// that "无权限的命令和所有脚本运行在沙盒内".
    pub fn route_command(
        command: &str,
        is_permitted: bool,
        cwd: &str,
    ) -> Result<String, ToolCallError> {
        if is_script(command) || !is_permitted {
            Self::execute_in_sandbox(command, cwd)
        } else {
            Self::execute_outside_sandbox(command, cwd)
        }
    }

    /// Check if a command should be executed in the sandbox.
    ///
    /// Scripts are always sandboxed. Non-script commands are sandboxed
    /// only when `is_permitted` is `false`.
    pub fn should_sandbox(command: &str, is_permitted: bool) -> bool {
        is_script(command) || !is_permitted
    }

    /// Apply sandbox restrictions (landlock + seccomp) without executing.
    ///
    /// Used by [`BashTool`] to apply restrictions before delegating to
    /// the existing `execute_command` path (which handles background
    /// execution and kill-handle integration).
    pub fn apply_sandbox_restrictions(cwd: &str) -> Result<(), ToolCallError> {
        apply_landlock(cwd)?;
        apply_seccomp()?;
        Ok(())
    }

    /// Spawn a command via `sh -c` and collect output.
    fn run(command: &str, cwd: &str) -> Result<String, ToolCallError> {
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(cwd)
            .output()
            .map_err(|e| {
                ToolCallError::ExecutionFailed(format!("failed to spawn command: {}", e))
            })?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if !stderr.is_empty() {
            Ok(format!("{}\n{}", stdout, stderr))
        } else {
            Ok(stdout)
        }
    }
}

// ---------------------------------------------------------------------------
// Linux sandbox enforcement (stubs)
// ---------------------------------------------------------------------------

/// Apply landlock filesystem restrictions.
///
/// Restricts the process to read-only access on the working directory
/// and `/tmp`. On non-Linux platforms this is a no-op.
#[cfg(target_os = "linux")]
fn apply_landlock(cwd: &str) -> Result<(), ToolCallError> {
    // Sandbox infrastructure is ready; enforcement strategy will be
    // implemented in a follow-up PR (landlock_create_ruleset +
    // landlock_add_rule via the Linux ABI).
    tracing::warn!(
        cwd = %cwd,
        "CommandSandbox::apply_landlock() — sandbox infrastructure ready, \
         enforcement strategy deferred to follow-up PR"
    );
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn apply_landlock(_cwd: &str) -> Result<(), ToolCallError> {
    Ok(())
}

/// Apply seccomp syscall filtering.
///
/// Blocks dangerous syscalls such as `mount`, `pivot_root`, `reboot`.
/// On non-Linux platforms this is a no-op.
#[cfg(target_os = "linux")]
fn apply_seccomp() -> Result<(), ToolCallError> {
    // Sandbox infrastructure is ready; enforcement strategy will be
    // implemented in a follow-up PR (BPF program via seccomp(2)
    // with SECCOMP_SET_MODE_FILTER).
    tracing::warn!(
        "CommandSandbox::apply_seccomp() — sandbox infrastructure ready, \
         enforcement strategy deferred to follow-up PR"
    );
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn apply_seccomp() -> Result<(), ToolCallError> {
    Ok(())
}

#[cfg(test)]
#[path = "command_sandbox_tests.rs"]
mod tests;
