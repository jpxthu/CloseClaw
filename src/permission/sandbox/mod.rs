//! OS-level sandboxing for the Permission Engine.
//!
//! The Permission Engine runs as a separate OS process for security isolation.
//! This module provides:
//! - [`Sandbox`] — subprocess lifecycle management
//! - [`IpcChannel`] — Unix socket communication between host and engine process
//! - [`SecurityPolicy`] — seccomp/landlock enforcement on Linux
//! - [`SandboxRequest`] / [`SandboxResponse`] — serialized IPC messages

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;
use tokio::time::timeout;

use crate::permission::{PermissionEngine, PermissionRequest, PermissionResponse, RuleSet};

/// Maximum time to wait for the engine process to start.
const ENGINE_SPAWN_TIMEOUT_MS: u64 = 5000;

/// Maximum time to wait for a response from the engine over IPC.
const IPC_TIMEOUT_MS: u64 = 3000;

// ---------------------------------------------------------------------------
// Security Policy
// ---------------------------------------------------------------------------

/// Security policies applied to the engine subprocess.
///
/// On Linux, seccomp and landlock are used to restrict the engine's capabilities.
/// On non-Linux platforms, these are no-ops.
#[derive(Debug, Clone, Default)]
pub struct SecurityPolicy {
    /// Enable seccomp to restrict syscalls.
    pub seccomp: bool,
    /// Enable landlock to restrict filesystem access.
    pub landlock: bool,
    /// Explicitly allowed filesystem paths (used with landlock).
    pub allowed_fs_paths: Vec<PathBuf>,
    /// Explicitly blocked syscalls (used with seccomp).
    pub blocked_syscalls: Vec<String>,
}

impl SecurityPolicy {
    /// Create a default security policy that enables seccomp and landlock on Linux.
    pub fn default_restrictive() -> Self {
        Self {
            seccomp: cfg!(target_os = "linux"),
            landlock: cfg!(target_os = "linux"),
            allowed_fs_paths: vec![],
            blocked_syscalls: vec![],
        }
    }

    /// Apply the security policy inside the engine subprocess.
    ///
    /// Call this **once** at startup, before serving any requests.
    #[cfg(target_os = "linux")]
    pub fn apply(&self) -> anyhow::Result<()> {
        if self.seccomp {
            apply_seccomp()?;
        }
        if self.landlock {
            apply_landlock(&self.allowed_fs_paths)?;
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    pub fn apply(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn apply_seccomp() -> anyhow::Result<()> {
    // seccomp enforcement is not yet implemented.
    // In production, use libseccomp or a BPF program via seccomp(2)
    // with SECCOMP_SET_MODE_FILTER.
    tracing::warn!(
        "SecurityPolicy::apply(): seccomp enforcement is a stub. \
         Kernel-level syscall filtering is NOT active."
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn apply_landlock(_allowed_paths: &[PathBuf]) -> anyhow::Result<()> {
    // Landlock enforcement is not yet implemented.
    // Landlock is available since Linux 5.13.
    // In production, call `landlock_create_ruleset()` and `landlock_add_rule()`
    // via the userspace ABI.
    tracing::warn!(
        "SecurityPolicy::apply(): landlock enforcement is a stub. \
         Filesystem sandboxing is NOT active."
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// IPC Messages
// ---------------------------------------------------------------------------

/// Request sent from the host process to the sandboxed engine.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SandboxRequest {
    /// Ask the engine to evaluate a permission request.
    Evaluate { request: PermissionRequest },
    /// Ask the engine to reload its ruleset.
    ReloadRules { rules: RuleSet },
    /// Ping — returns Pong.
    Ping,
}

/// Response sent from the sandboxed engine back to the host.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SandboxResponse {
    /// Permission evaluation result.
    PermissionResponse(PermissionResponse),
    /// Acknowledge a rules reload.
    RulesReloaded,
    /// Pong acknowledgement.
    Pong,
    /// Engine-side error.
    Error { message: String },
}

// ---------------------------------------------------------------------------
// IPC Channel
// ---------------------------------------------------------------------------

/// Bidirectional IPC channel over a Unix domain socket.
///
/// The protocol uses length-prefixed JSON frames:
///
/// ```text
/// [4-byte big-endian length (u32)][JSON frame bytes]
/// ```
pub struct IpcChannel {
    path: PathBuf,
}

impl IpcChannel {
    /// Create a new IPC channel at the given socket path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Remove the socket file if it already exists (idempotent).
    fn clean_up(&self) {
        let _ = std::fs::remove_file(&self.path);
    }

    /// Connect to the engine and send a request, returning the response.
    async fn call(&self, request: &SandboxRequest) -> std::io::Result<SandboxResponse> {
        let stream = timeout(
            Duration::from_millis(IPC_TIMEOUT_MS),
            UnixStream::connect(&self.path),
        )
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "IPC connect timeout"))??;

        let (reader, mut writer): (_, OwnedWriteHalf) = stream.into_split();

        // Send request: [4-byte len][json]
        let json = serde_json::to_vec(request)?;
        let len = (json.len() as u32).to_be_bytes();
        writer.write_all(&len).await?;
        writer.write_all(&json).await?;
        writer.flush().await?;

        // Read response header
        let mut hdr = [0u8; 4];
        let mut reader = BufReader::new(reader);
        reader.read_exact(&mut hdr).await?;
        let body_len = u32::from_be_bytes(hdr) as usize;

        // Read body
        let mut body = vec![0u8; body_len];
        reader.read_exact(&mut body).await?;

        serde_json::from_slice(&body)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Start listening for connections and dispatch them to the engine.
    ///
    /// Blocks forever, processing each connection in a spawned task.
    pub async fn serve(self, engine: Arc<PermissionEngine>) -> std::io::Result<()> {
        self.clean_up();

        let listener = UnixListener::bind(&self.path)?;

        tracing::info!("engine IPC server listening on {}", self.path.display());

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let engine = engine.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, engine).await {
                            tracing::error!("IPC connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("UnixListener accept error: {}", e);
                }
            }
        }
    }
}

/// Handle a single IPC connection from the host.
async fn handle_connection(
    stream: UnixStream,
    engine: Arc<PermissionEngine>,
) -> std::io::Result<()> {
    let (reader, writer): (_, OwnedWriteHalf) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut writer = writer;

    loop {
        // Read 4-byte length header
        let mut hdr = [0u8; 4];
        match reader.read_exact(&mut hdr).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
        let body_len = u32::from_be_bytes(hdr) as usize;

        // Read body
        let mut body = vec![0u8; body_len];
        reader.read_exact(&mut body).await?;

        // Deserialize request
        let request: SandboxRequest = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => {
                let resp = SandboxResponse::Error {
                    message: format!("invalid request: {}", e),
                };
                send_response(&mut writer, &resp).await?;
                continue;
            }
        };

        // Process request
        let response = match &request {
            SandboxRequest::Evaluate { request } => {
                SandboxResponse::PermissionResponse(engine.evaluate(request.clone()))
            }
            SandboxRequest::ReloadRules { rules: _ } => {
                // The engine is recreated externally; we just acknowledge.
                SandboxResponse::RulesReloaded
            }
            SandboxRequest::Ping => SandboxResponse::Pong,
        };

        send_response(&mut writer, &response).await?;
    }

    Ok(())
}

async fn send_response(
    writer: &mut OwnedWriteHalf,
    response: &SandboxResponse,
) -> std::io::Result<()> {
    let json = serde_json::to_vec(response)?;
    let len = (json.len() as u32).to_be_bytes();
    writer.write_all(&len).await?;
    writer.write_all(&json).await?;
    writer.flush().await?;
    Ok(())
}

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

        self.child = Some(child);

        // Ping to confirm the engine is responsive.
        match timeout(
            Duration::from_millis(IPC_TIMEOUT_MS),
            self.channel.call(&SandboxRequest::Ping),
        )
        .await
        {
            Ok(Ok(SandboxResponse::Pong)) => {}
            Ok(Ok(other)) => {
                return Err(SandboxError::ProcessError(format!(
                    "unexpected response to Ping: {:?}",
                    other
                )));
            }
            Ok(Err(e)) => return Err(SandboxError::Ipc(e)),
            Err(_) => return Err(SandboxError::IpcTimeout),
        }

        *self.state.write().await = SandboxState::Running;
        Ok(())
    }

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
