//! OS-level sandboxing for the Permission Engine.
//!
//! The Permission Engine runs as a separate OS process for security isolation.
//! This module provides:
//! - [`Sandbox`] — subprocess lifecycle management
//! - [`IpcChannel`] — Unix socket communication between host and engine process
//! - [`SecurityPolicy`] — seccomp/landlock enforcement on Linux
//! - [`SandboxRequest`] / [`SandboxResponse`] — serialized IPC messages

mod security;
pub use security::*;

mod ipc;
pub use ipc::*;

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::time::timeout;

use crate::permission::{PermissionEngine, PermissionRequest, PermissionResponse, RuleSet};

/// Maximum time to wait for the engine process to start.
const ENGINE_SPAWN_TIMEOUT_MS: u64 = 5000;

// ---------------------------------------------------------------------------
// Sandbox
// ---------------------------------------------------------------------------

/// Current state of the sandboxed engine subprocess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxState {
    #[default]
    Unstarted,
    Running,
    Crashed {
        exit_code: Option<i32>,
    },
    Shutdown,
}

/// Errors that can occur during sandbox operations.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("IPC error: {0}")]
    Ipc(#[from] std::io::Error),

    #[error("IPC timeout waiting for engine response")]
    IpcTimeout,

    #[error("engine process error: {0}")]
    ProcessError(String),

    #[error("engine is in state {state:?}")]
    InvalidState { state: SandboxState },
}

/// Manages the lifecycle of the sandboxed Permission Engine subprocess.
///
/// `Sandbox` provides:
/// - **Spawn** — starts the engine as a child process
/// - **Restart** — re-spawns after a crash
/// - **Shutdown** — cleanly terminates the engine
/// - **Evaluate** — sends permission requests over IPC
///
/// Communication with the engine uses [`IpcChannel`] (Unix domain socket).
///
/// # Example
///
/// ```ignore
/// let mut sandbox = Sandbox::new("/tmp/closeclaw-engine.sock");
/// sandbox.spawn().await?;
///
/// let resp = sandbox.evaluate(request).await?;
/// sandbox.shutdown().await?;
/// ```
pub struct Sandbox {
    ipc_path: PathBuf,
    policy: SecurityPolicy,
    child: Option<Child>,
    channel: IpcChannel,
    state: RwLock<SandboxState>,
}

impl Sandbox {
    /// Create a new sandbox for the engine at the given IPC socket path.
    pub fn new(ipc_path: impl Into<PathBuf>) -> Self {
        let ipc_path = ipc_path.into();
        Self {
            ipc_path: ipc_path.clone(),
            policy: SecurityPolicy::default_restrictive(),
            child: None,
            channel: IpcChannel::new(ipc_path),
            state: RwLock::new(SandboxState::Unstarted),
        }
    }

    /// Set the security policy for the engine subprocess.
    pub fn with_policy(mut self, policy: SecurityPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Get the current engine state.
    pub async fn state(&self) -> SandboxState {
        *self.state.read().await
    }
}

impl Sandbox {
    /// Spawn the permission engine as a child process.
    ///
    /// The engine binary is the current process itself, started with the
    /// environment variable `SANDBOX_ENGINE=1` and `--engine` flag.
    ///
    /// After spawning, the function waits up to [`ENGINE_SPAWN_TIMEOUT_MS`] for the
    /// engine to become responsive on the IPC socket.
    pub async fn spawn(&mut self) -> Result<(), SandboxError> {
        if *self.state.read().await == SandboxState::Running {
            return Err(SandboxError::InvalidState {
                state: SandboxState::Running,
            });
        }

        self.channel.clean_up();

        let child = self.spawn_child_process().await?;
        self.child = Some(child);

        self.confirm_engine_ready().await?;

        *self.state.write().await = SandboxState::Running;
        Ok(())
    }

    /// Create the engine subprocess and wait for its socket to appear.
    async fn spawn_child_process(&self) -> Result<Child, SandboxError> {
        let mut child = Command::new(std::env::current_exe()?)
            .env("SANDBOX_ENGINE", "1")
            .env("SANDBOX_IPC_PATH", &self.ipc_path)
            .arg("--engine")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| SandboxError::ProcessError(e.to_string()))?;

        // Wait for the socket to appear (engine is ready).
        let deadline = tokio::time::Instant::now() + Duration::from_millis(ENGINE_SPAWN_TIMEOUT_MS);

        let socket_ready = loop {
            if tokio::time::Instant::now() >= deadline {
                break false;
            }
            if self.ipc_path.exists() {
                // Give the engine one more moment to call bind().
                tokio::time::sleep(Duration::from_millis(50)).await;
                break true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };

        if !socket_ready {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SandboxError::ProcessError(
                "engine socket never appeared".to_string(),
            ));
        }

        Ok(child)
    }

    /// Confirm the engine is responsive by sending a Ping and awaiting Pong.
    async fn confirm_engine_ready(&self) -> Result<(), SandboxError> {
        match timeout(
            Duration::from_millis(IPC_TIMEOUT_MS),
            self.channel.call(&SandboxRequest::Ping),
        )
        .await
        {
            Ok(Ok(SandboxResponse::Pong)) => Ok(()),
            Ok(Ok(other)) => Err(SandboxError::ProcessError(format!(
                "unexpected response to Ping: {:?}",
                other
            ))),
            Ok(Err(e)) => Err(SandboxError::Ipc(e)),
            Err(_) => Err(SandboxError::IpcTimeout),
        }
    }
}

impl Sandbox {
    /// Restart the engine process (shutdown + spawn).
    pub async fn restart(&mut self) -> Result<(), SandboxError> {
        self.shutdown().await;
        self.spawn().await
    }

    /// Shutdown the engine subprocess.
    pub async fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.channel.clean_up();
        *self.state.write().await = SandboxState::Shutdown;
    }

    /// Evaluate a permission request by sending it to the engine over IPC.
    pub async fn evaluate(
        &self,
        request: PermissionRequest,
    ) -> Result<PermissionResponse, SandboxError> {
        let state = *self.state.read().await;
        if state != SandboxState::Running {
            return Err(SandboxError::InvalidState { state });
        }

        let resp = timeout(
            Duration::from_millis(IPC_TIMEOUT_MS),
            self.channel.call(&SandboxRequest::Evaluate { request }),
        )
        .await
        .map_err(|_| SandboxError::IpcTimeout)?
        .map_err(SandboxError::Ipc)?;

        match resp {
            SandboxResponse::PermissionResponse(r) => Ok(r),
            SandboxResponse::Error { message } => Err(SandboxError::ProcessError(message)),
            other => Err(SandboxError::ProcessError(format!(
                "unexpected response: {:?}",
                other
            ))),
        }
    }

    /// Reload the ruleset in the running engine.
    pub async fn reload_rules(&self, rules: RuleSet) -> Result<(), SandboxError> {
        let state = *self.state.read().await;
        if state != SandboxState::Running {
            return Err(SandboxError::InvalidState { state });
        }

        let resp = timeout(
            Duration::from_millis(IPC_TIMEOUT_MS),
            self.channel.call(&SandboxRequest::ReloadRules { rules }),
        )
        .await
        .map_err(|_| SandboxError::IpcTimeout)?
        .map_err(SandboxError::Ipc)?;

        match resp {
            SandboxResponse::RulesReloaded => Ok(()),
            SandboxResponse::Error { message } => Err(SandboxError::ProcessError(message)),
            other => Err(SandboxError::ProcessError(format!(
                "unexpected response: {:?}",
                other
            ))),
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
        }
    }
}

// ---------------------------------------------------------------------------
// Engine subprocess entry point
// ---------------------------------------------------------------------------

/// Run the engine in subprocess mode.
///
/// Called when the binary is started with `SANDBOX_ENGINE=1` env var.
/// Creates a [`PermissionEngine`], binds the IPC socket, and serves requests.
pub async fn run_engine_subprocess(ipc_path: PathBuf, rules: RuleSet) -> anyhow::Result<()> {
    let engine = Arc::new(PermissionEngine::new(rules));
    let channel = IpcChannel::new(ipc_path);

    // Apply security policy as early as possible.
    let policy = SecurityPolicy::default_restrictive();
    policy.apply()?;

    channel.serve(engine).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_state_default() {
        let state = SandboxState::default();
        assert_eq!(state, SandboxState::Unstarted);
    }

    #[test]
    fn test_sandbox_error_display() {
        let ipc_err = SandboxError::IpcTimeout;
        let ipc_msg = ipc_err.to_string();
        assert!(
            ipc_msg.contains("timeout") || ipc_msg.contains("IPC"),
            "IpcTimeout display should contain 'timeout' or 'IPC': {}",
            ipc_msg
        );

        let process_err = SandboxError::ProcessError("engine died".to_string());
        let process_msg = process_err.to_string();
        assert!(
            process_msg.contains("engine died"),
            "ProcessError display should contain the message: {}",
            process_msg
        );

        let invalid_state = SandboxError::InvalidState {
            state: SandboxState::Running,
        };
        let invalid_msg = invalid_state.to_string();
        assert!(
            invalid_msg.contains("Running"),
            "InvalidState display should contain the state: {}",
            invalid_msg
        );
    }
}
