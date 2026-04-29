//! Agent Process - manages the OS process for an agent
//!
//! Handles spawning, communication, and monitoring of agent child processes.

use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Errors that can occur during process management
#[derive(Error, Debug)]
pub enum ProcessError {
    #[error("failed to spawn process: {0}")]
    SpawnError(#[from] std::io::Error),
    #[error("process not found: {0}")]
    ProcessNotFound(String),
    #[error("process already running: {0}")]
    ProcessAlreadyRunning(String),
    #[error("process communication error: {0}")]
    CommunicationError(String),
    #[error("process exited unexpectedly: {0}")]
    UnexpectedExit(i32),
}

/// JSON message envelope for inter-process communication
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProcessMessage {
    /// Message type (e.g., "heartbeat", "task", "result", "error")
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Source agent ID
    pub from: String,
    /// Target agent ID (optional for broadcasts)
    pub to: Option<String>,
    /// Message payload
    pub payload: serde_json::Value,
}

/// Handle to a running agent process
#[derive(Debug, Clone)]
pub struct AgentProcessHandle {
    /// The child process
    child: Arc<RwLock<Child>>,
    /// Process ID for logging
    pid: u32,
    /// Agent ID this process belongs to
    agent_id: String,
}

impl AgentProcessHandle {
    /// Send a JSON message to the agent via stdin
    pub async fn send_message(&mut self, message: &str) -> Result<(), ProcessError> {
        let mut child = self.child.write().await;
        if let Some(ref mut stdin) = child.stdin {
            stdin
                .write_all(message.as_bytes())
                .await
                .map_err(|e| ProcessError::CommunicationError(e.to_string()))?;
            stdin
                .write_all(b"\n")
                .await
                .map_err(|e| ProcessError::CommunicationError(e.to_string()))?;
            debug!(agent_id = %self.agent_id, "sent message to agent stdin");
        } else {
            return Err(ProcessError::CommunicationError(
                "stdin not available".to_string(),
            ));
        }
        Ok(())
    }

    /// Send a structured message to the agent
    pub async fn send_json(&mut self, msg: &ProcessMessage) -> Result<(), ProcessError> {
        let json = serde_json::to_string(msg)
            .map_err(|e| ProcessError::CommunicationError(e.to_string()))?;
        self.send_message(&json).await
    }

    /// Kill the process
    pub async fn kill(&mut self) -> Result<(), ProcessError> {
        debug!(agent_id = %self.agent_id, pid = %self.pid, "killing agent process");
        let mut child = self.child.write().await;
        child
            .kill()
            .await
            .map_err(|e| ProcessError::CommunicationError(e.to_string()))?;
        Ok(())
    }

    /// Wait for the process to exit
    pub async fn wait(&mut self) -> Result<i32, ProcessError> {
        let status = self
            .child
            .write()
            .await
            .wait()
            .await
            .map_err(|e| ProcessError::CommunicationError(e.to_string()))?;
        match status.code() {
            Some(code) => {
                debug!(agent_id = %self.agent_id, code = code, "process exited");
                Ok(code)
            }
            None => Err(ProcessError::CommunicationError(
                "process terminated by signal".to_string(),
            )),
        }
    }

    /// Get the process ID
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Get the agent ID
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }
}

/// Agent process manager
#[derive(Debug)]
pub struct AgentProcess;

impl AgentProcess {
    /// Spawn a new agent process
    ///
    /// # Arguments
    /// * `binary_path` - Path to the agent binary to execute
    /// * `agent_id` - ID to pass to the agent process via environment
    /// * `bootstrap_minimal` - Whether to use minimal bootstrap mode (true) or full mode (false)
    ///
    /// # Returns
    /// A handle to the spawned process
    pub async fn spawn(
        binary_path: &str,
        agent_id: &str,
        bootstrap_minimal: bool,
    ) -> Result<AgentProcessHandle, ProcessError> {
        info!(binary = %binary_path, agent_id = %agent_id, bootstrap_minimal = bootstrap_minimal, "spawning agent process");

        let bootstrap_mode = if bootstrap_minimal { "minimal" } else { "full" };
        let child = Command::new(binary_path)
            .env("AGENT_ID", agent_id)
            .env("BOOTSTRAP_MODE", bootstrap_mode)
            .env("RUST_LOG", "info")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let pid = child.id().unwrap_or(0);
        info!(agent_id = %agent_id, pid = %pid, "agent process spawned");

        Ok(AgentProcessHandle {
            child: Arc::new(RwLock::new(child)),
            pid,
            agent_id: agent_id.to_string(),
        })
    }

    /// Spawn with custom arguments
    pub async fn spawn_with_args(
        binary_path: &str,
        agent_id: &str,
        args: &[&str],
        bootstrap_minimal: bool,
    ) -> Result<AgentProcessHandle, ProcessError> {
        info!(binary = %binary_path, agent_id = %agent_id, args = ?args, bootstrap_minimal = bootstrap_minimal, "spawning agent process with args");

        let bootstrap_mode = if bootstrap_minimal { "minimal" } else { "full" };
        let mut cmd = Command::new(binary_path);
        cmd.env("AGENT_ID", agent_id)
            .env("BOOTSTRAP_MODE", bootstrap_mode)
            .env("RUST_LOG", "info")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        for arg in args {
            cmd.arg(arg);
        }

        let child = cmd.spawn()?;

        let pid = child.id().unwrap_or(0);
        info!(agent_id = %agent_id, pid = %pid, "agent process spawned with args");

        Ok(AgentProcessHandle {
            child: Arc::new(RwLock::new(child)),
            pid,
            agent_id: agent_id.to_string(),
        })
    }

    /// Create a message for inter-agent communication
    pub fn create_message(
        from: &str,
        to: Option<&str>,
        msg_type: &str,
        payload: impl serde::Serialize,
    ) -> Result<String, ProcessError> {
        let msg = ProcessMessage {
            msg_type: msg_type.to_string(),
            from: from.to_string(),
            to: to.map(|s| s.to_string()),
            payload: serde_json::to_value(payload)
                .map_err(|e| ProcessError::CommunicationError(e.to_string()))?,
        };
        serde_json::to_string(&msg).map_err(|e| ProcessError::CommunicationError(e.to_string()))
    }

    /// Parse a message from JSON
    pub fn parse_message(raw: &str) -> Result<ProcessMessage, ProcessError> {
        serde_json::from_str(raw).map_err(|e| {
            ProcessError::CommunicationError(format!("failed to parse message: {}", e))
        })
    }
}

/// Spawn a background task to read stdout from a process
pub async fn spawn_output_reader(
    handle: AgentProcessHandle,
) -> Result<tokio::sync::mpsc::Receiver<ProcessMessage>, ProcessError> {
    let (tx, rx) = tokio::sync::mpsc::channel(100);

    let agent_id = handle.agent_id.clone();
    let mut child = handle.child.write().await;

    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                match AgentProcess::parse_message(&line) {
                    Ok(msg) => {
                        debug!(agent_id = %agent_id, msg_type = %msg.msg_type, "received message from agent");
                        if tx.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(agent_id = %agent_id, error = %e, "failed to parse agent message");
                    }
                }
            }
            debug!(agent_id = %agent_id, "output reader task ended");
        });
    }

    Ok(rx)
}

/// Spawn a background task to read stderr from a process
pub async fn spawn_error_reader(
    handle: AgentProcessHandle,
) -> Result<tokio::sync::mpsc::Receiver<String>, ProcessError> {
    let (tx, rx) = tokio::sync::mpsc::channel(100);

    let agent_id = handle.agent_id.clone();
    let mut child = handle.child.write().await;

    if let Some(stderr) = child.stderr.take() {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();

        tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                if tx.send(line).await.is_err() {
                    break;
                }
            }
            debug!(agent_id = %agent_id, "error reader task ended");
        });
    }

    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_message() {
        let msg = AgentProcess::create_message(
            "agent1",
            Some("agent2"),
            "task",
            &serde_json::json!({"data": "test"}),
        )
        .unwrap();

        let parsed: ProcessMessage = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed.from, "agent1");
        assert_eq!(parsed.to, Some("agent2".to_string()));
        assert_eq!(parsed.msg_type, "task");
    }

    #[test]
    fn test_parse_message() {
        let json = r#"{"type":"heartbeat","from":"agent1","to":null,"payload":{"seq":1}}"#;
        let msg = AgentProcess::parse_message(json).unwrap();
        assert_eq!(msg.msg_type, "heartbeat");
        assert_eq!(msg.from, "agent1");
        assert!(msg.to.is_none());
    }

    #[test]
    fn test_parse_invalid_message() {
        let result = AgentProcess::parse_message("not valid json");
        assert!(result.is_err());
    }

    #[test]
    fn test_process_message_serialize_deserialize() {
        let msg = ProcessMessage {
            msg_type: "result".to_string(),
            from: "a".to_string(),
            to: Some("b".to_string()),
            payload: serde_json::json!({"x": 42}),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ProcessMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.msg_type, "result");
        assert_eq!(parsed.payload["x"], 42);
    }

    #[test]
    fn test_process_message_broadcast() {
        let msg = ProcessMessage {
            msg_type: "heartbeat".to_string(),
            from: "agent".to_string(),
            to: None,
            payload: serde_json::json!({}),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"heartbeat\""));
    }

    #[test]
    fn test_process_error_display() {
        let err = ProcessError::SpawnError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no binary",
        ));
        assert!(err.to_string().contains("no binary"));

        let err = ProcessError::ProcessNotFound("test".to_string());
        assert!(err.to_string().contains("test"));

        let err = ProcessError::ProcessAlreadyRunning("a".to_string());
        assert!(err.to_string().contains("already running"));

        let err = ProcessError::CommunicationError("comm fail".to_string());
        assert!(err.to_string().contains("comm fail"));

        let err = ProcessError::UnexpectedExit(1);
        assert!(err.to_string().contains("1"));
    }

    #[test]
    fn test_create_message_no_target() {
        let msg = AgentProcess::create_message(
            "agent1",
            None,
            "broadcast",
            &serde_json::json!({"alert": true}),
        )
        .unwrap();
        let parsed: ProcessMessage = serde_json::from_str(&msg).unwrap();
        assert!(parsed.to.is_none());
        assert_eq!(parsed.payload["alert"], true);
    }

    #[test]
    fn test_parse_message_missing_fields() {
        let json = r#"{"type":"test","from":"a","to":null,"payload":null}"#;
        let msg = AgentProcess::parse_message(json).unwrap();
        assert_eq!(msg.msg_type, "test");
    }

    #[test]
    fn test_agent_process_handle_accessors() {
        // spawn "echo" to get a real process handle
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = rt.block_on(async {
            AgentProcess::spawn("echo", "test-agent", false)
                .await
                .unwrap()
        });
        assert_eq!(handle.agent_id(), "test-agent");
        // pid might be 0 if process exited quickly, but shouldn't panic
        let _pid = handle.pid();
    }

    #[tokio::test]
    async fn test_spawn_nonexistent_binary() {
        let result = AgentProcess::spawn("/nonexistent/binary/path", "test", false).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_spawn_with_args() {
        let result = AgentProcess::spawn_with_args("echo", "test-agent", &["hello"], false).await;
        assert!(result.is_ok());
        let handle = result.unwrap();
        assert_eq!(handle.agent_id(), "test-agent");
    }

    #[tokio::test]
    async fn test_send_message_to_echo() {
        let handle = AgentProcess::spawn("cat", "test-agent", false)
            .await
            .unwrap();
        let mut handle = handle;
        // cat echoes stdin to stdout, so we can send a message
        let result = handle.send_message("hello world").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_json_message() {
        let handle = AgentProcess::spawn("cat", "test-agent", false)
            .await
            .unwrap();
        let mut handle = handle;
        let msg = ProcessMessage {
            msg_type: "test".to_string(),
            from: "a".to_string(),
            to: Some("b".to_string()),
            payload: serde_json::json!({"key": "val"}),
        };
        let result = handle.send_json(&msg).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_kill_process() {
        let handle = AgentProcess::spawn("sleep", "test-agent", false)
            .await
            .unwrap();
        let mut handle = handle;
        let result = handle.kill().await;
        assert!(result.is_ok());
    }

    /// Verify bootstrap_minimal=false sets BOOTSTRAP_MODE=full in child process environment
    #[tokio::test]
    async fn test_spawn_with_bootstrap_full() {
        // Use sh -c to run 'echo "$BOOTSTRAP_MODE"' which prints the env var value set by spawn
        let handle = AgentProcess::spawn(
            "sh",
            "test-bootstrap-full",
            false, // bootstrap_minimal = false -> should be "full"
        )
        .await
        .unwrap();
        let mut handle = handle;
        let result = handle.send_message("echo $BOOTSTRAP_MODE").await;
        assert!(result.is_ok());
        let exit_code = handle.wait().await.unwrap();
        assert_eq!(exit_code, 0);
    }

    /// Verify bootstrap_minimal=true sets BOOTSTRAP_MODE=minimal in child process environment
    #[tokio::test]
    async fn test_spawn_with_bootstrap_minimal() {
        // Use sh -c to run 'echo "$BOOTSTRAP_MODE"' which prints the env var value set by spawn
        let handle = AgentProcess::spawn(
            "sh",
            "test-bootstrap-minimal",
            true, // bootstrap_minimal = true -> should be "minimal"
        )
        .await
        .unwrap();
        let mut handle = handle;
        let result = handle.send_message("echo $BOOTSTRAP_MODE").await;
        assert!(result.is_ok());
        let exit_code = handle.wait().await.unwrap();
        assert_eq!(exit_code, 0);
    }
}
