//! Step completion hooks — executed after a step transitions to completed.
//!
//! Hooks allow post-step actions (verification, notification, custom scripts)
//! without blocking step completion.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

use crate::engine::StepResult;
use crate::spawn::SpawnAdapter;
use crate::types::VerifyTrigger;

/// Callback type for NotifyHook: receives (step_index, summary).
type NotifyCallback = Box<
    dyn Fn(
            usize,
            String,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>
        + Send
        + Sync,
>;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during hook execution.
#[derive(Debug, Clone, Error)]
pub enum HookError {
    /// Verification agent spawn failed.
    #[error("verification spawn failed: {message}")]
    VerificationSpawnFailed {
        /// Descriptive error message.
        message: String,
    },

    /// Notify callback failed.
    #[error("notify callback failed: {message}")]
    NotifyFailed {
        /// Descriptive error message.
        message: String,
    },

    /// Custom script execution failed.
    #[error("custom hook failed: {message}")]
    CustomFailed {
        /// Descriptive error message.
        message: String,
    },

    /// Custom script timed out.
    #[error("custom hook timed out after {timeout_secs}s")]
    CustomTimeout {
        /// Timeout in seconds.
        timeout_secs: u64,
    },
}

// ---------------------------------------------------------------------------
// HookResult
// ---------------------------------------------------------------------------

/// Result of a hook execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookResult {
    /// Hook succeeded; continue with remaining hooks.
    Continue,
    /// Hook wants to block further processing.
    Block(String),
}

// ---------------------------------------------------------------------------
// StepHook trait
// ---------------------------------------------------------------------------

/// Trait for hooks that execute after a step completes.
#[async_trait]
pub trait StepHook: Send + Sync {
    /// Execute the hook for the given completed step.
    async fn execute(&self, step: &StepResult) -> Result<HookResult, HookError>;
}

// ---------------------------------------------------------------------------
// VerificationHook
// ---------------------------------------------------------------------------

/// Hook that spawns a verification sub-agent after step completion.
pub struct VerificationHook<S> {
    /// Adapter used to spawn the verification agent.
    adapter: S,
}

impl<S> VerificationHook<S> {
    /// Create a new verification hook with the given spawn adapter.
    pub fn new(adapter: S) -> Self {
        Self { adapter }
    }
}

#[async_trait]
impl<S: SpawnAdapter> StepHook for VerificationHook<S> {
    async fn execute(&self, step: &StepResult) -> Result<HookResult, HookError> {
        let task = format!(
            "Verify step {} ({}): {}",
            step.step_index, step.description, step.summary
        );
        let context = format!("changed_files={}", step.changed_files.join(","));

        match self.adapter.spawn_run(&task, &context).await {
            Ok(_result) => Ok(HookResult::Continue),
            Err(e) => Ok(HookResult::Block(format!("verification failed: {e}"))),
        }
    }
}

// ---------------------------------------------------------------------------
// NotifyHook
// ---------------------------------------------------------------------------

/// Hook that notifies external systems about step completion.
pub struct NotifyHook {
    /// Callback function receiving (step_index, summary).
    callback: NotifyCallback,
}

impl NotifyHook {
    /// Create a new notify hook with the given callback.
    pub fn new<F, Fut>(callback: F) -> Self
    where
        F: Fn(usize, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    {
        Self {
            callback: Box::new(move |idx, summary| Box::pin(callback(idx, summary))),
        }
    }
}

#[async_trait]
impl StepHook for NotifyHook {
    async fn execute(&self, step: &StepResult) -> Result<HookResult, HookError> {
        match (self.callback)(step.step_index, step.summary.clone()).await {
            Ok(()) => Ok(HookResult::Continue),
            Err(_) => Ok(HookResult::Continue), // notify failure is non-blocking
        }
    }
}

// ---------------------------------------------------------------------------
// CustomHook
// ---------------------------------------------------------------------------

/// Hook that executes a user-configured shell command.
pub struct CustomHook {
    /// Shell command to execute.
    command: String,
    /// Timeout for the command execution.
    timeout: Duration,
}

impl CustomHook {
    /// Create a new custom hook with the given command and timeout.
    pub fn new(command: String, timeout: Duration) -> Self {
        Self { command, timeout }
    }
}

#[async_trait]
impl StepHook for CustomHook {
    async fn execute(&self, _step: &StepResult) -> Result<HookResult, HookError> {
        let mut child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&self.command)
            .process_group(0)
            .spawn()
            .map_err(|e| HookError::CustomFailed {
                message: format!("failed to spawn command: {e}"),
            })?;

        match tokio::time::timeout(self.timeout, child.wait()).await {
            Ok(Ok(_)) | Ok(Err(_)) => Ok(HookResult::Continue),
            Err(_elapsed) => {
                if let Some(pid) = child.id() {
                    // PIDs exceeding i32::MAX cannot exist on Unix (see
                    // closeclaw_platform::process for the same guard); a cast
                    // would wrap such a pid to a negative value, and negating
                    // that would turn it positive again (2^32 - pid), making
                    // the kill target an unrelated single process rather than
                    // the hook's group — skip kill instead.
                    if let Ok(pid_i32) = i32::try_from(pid) {
                        // SAFETY: kill(2) with a pid argument performs no memory
                        // access. The child was spawned as its own process group
                        // leader (`process_group(0)`), so a negative pid targets
                        // the whole hook process tree; a stale pid (PID reuse
                        // race) at worst hits an unrelated process or fails
                        // with ESRCH.
                        if unsafe { libc::kill(-pid_i32, libc::SIGKILL) } == 0 {
                            // Reap only on confirmed delivery: waiting on a
                            // child we failed to signal could block long past
                            // the timeout (non-blocking contract).
                            let _ = child.wait().await;
                        } else {
                            tracing::warn!(
                                pid,
                                error = %std::io::Error::last_os_error(),
                                "failed to kill timed-out hook process group"
                            );
                        }
                    } else {
                        tracing::warn!(pid, "hook pid exceeds i32::MAX, skipping kill");
                    }
                }
                Ok(HookResult::Continue) // timeout is non-blocking per plan
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HookRunner
// ---------------------------------------------------------------------------

/// Manages a list of hooks and runs them after step completion
/// based on the configured trigger policy.
pub struct HookRunner {
    /// Registered hooks to execute.
    hooks: Vec<Box<dyn StepHook>>,
    /// Trigger policy controlling when hooks execute.
    trigger: VerifyTrigger,
}

impl HookRunner {
    /// Create a new hook runner with the given trigger policy.
    pub fn new(trigger: VerifyTrigger) -> Self {
        Self {
            hooks: Vec::new(),
            trigger,
        }
    }

    /// Register a hook.
    pub fn register(&mut self, hook: Box<dyn StepHook>) {
        self.hooks.push(hook);
    }

    /// Determine whether hooks should run for the given step.
    pub fn should_run(&self, step: &StepResult) -> bool {
        match self.trigger {
            VerifyTrigger::Never => false,
            VerifyTrigger::NonTrivial => !step.changed_files.is_empty(),
            VerifyTrigger::Always => true,
        }
    }

    /// Run all registered hooks for the given step, serially.
    ///
    /// Returns the first `Block` result encountered, or `Continue` if
    /// all hooks pass. Hook failures are logged as warnings and do not
    /// block step completion.
    pub async fn run_hooks(&self, step: &StepResult) -> HookResult {
        if !self.should_run(step) {
            return HookResult::Continue;
        }

        for hook in &self.hooks {
            match hook.execute(step).await {
                Ok(result) => {
                    if matches!(result, HookResult::Block(_)) {
                        return result;
                    }
                }
                Err(e) => {
                    tracing::warn!(step_index = step.step_index, error = %e, "hook execution failed");
                }
            }
        }

        HookResult::Continue
    }
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

impl fmt::Display for HookResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HookResult::Continue => write!(f, "continue"),
            HookResult::Block(reason) => write!(f, "block: {reason}"),
        }
    }
}
