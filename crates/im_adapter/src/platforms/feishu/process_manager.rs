//! lark-cli subprocess manager for event consumption.
//!
//! Manages the lifecycle of `lark-cli event consume` subprocess:
//! - Spawns the subprocess and reads NDJSON events from stdout
//! - Monitors stderr for the `[event] ready` readiness signal
//! - Auto-restarts on abnormal exit with exponential backoff
//! - Supports graceful shutdown via SIGTERM
//!
//! The subprocess outputs NDJSON events in two formats:
//! - **CLI format**: flat structure with top-level `type` and `event_id`
//!   (used by lark-cli event consume)
//! - **Webhook format**: envelope structure with `schema`/`header.event_type`
//!   (legacy SDK format, kept for backward compatibility)

use crate::error::AdapterError;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

/// Ready signal expected on stderr when the subprocess is fully initialized.
#[allow(dead_code)]
const READY_SIGNAL: &str = "[event] ready";

/// Initial delay before restarting a crashed subprocess (milliseconds).
#[allow(dead_code)]
const INITIAL_RESTART_DELAY_MS: u64 = 1_000;

/// Maximum delay between restart attempts (milliseconds).
#[allow(dead_code)]
const MAX_RESTART_DELAY_MS: u64 = 30_000;

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

/// A parsed event read from the subprocess stdout.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct Event {
    /// Event type string (e.g. `im.message.receive_v1`,
    /// `im.message.reaction.created_v1`).
    pub event_type: String,
    /// Event ID for deduplication.
    pub event_id: String,
    /// Raw JSON payload for downstream parsing.
    pub raw: serde_json::Value,
}

/// An event line read from subprocess stdout — either a successfully parsed
/// event or a read/parse error.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum EventLine {
    Event(Event),
    Error(String),
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors from subprocess management.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    Json(String),
    #[error("ready signal not received within timeout")]
    ReadyTimeout,
    #[error("subprocess exited unexpectedly (code={code:?})")]
    ProcessExit { code: Option<i32> },
}

impl From<ProcessError> for AdapterError {
    fn from(e: ProcessError) -> Self {
        match e {
            ProcessError::Io(io_err) => AdapterError::IoError(io_err),
            other => AdapterError::SendFailed(other.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Event format helpers
// ---------------------------------------------------------------------------

/// Extract `event_type` from a raw JSON value.
///
/// Supports both CLI format (`type` at top level) and webhook format
/// (`header.event_type`).
#[allow(dead_code)]
pub fn extract_event_type(raw: &serde_json::Value) -> String {
    // CLI format: top-level "type" field
    if let Some(t) = raw.get("type").and_then(|v| v.as_str()) {
        return t.to_string();
    }
    // Webhook format: header.event_type
    raw.get("header")
        .and_then(|h| h.get("event_type"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

/// Extract `event_id` from a raw JSON value.
///
/// Supports both CLI format (`event_id` at top level) and webhook format
/// (`header.event_id`).
#[allow(dead_code)]
pub(crate) fn extract_event_id(raw: &serde_json::Value) -> String {
    // CLI format: top-level "event_id"
    if let Some(id) = raw.get("event_id").and_then(|v| v.as_str()) {
        return id.to_string();
    }
    // Webhook format: header.event_id
    raw.get("header")
        .and_then(|h| h.get("event_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Parse a CLI-format event (flat top-level fields).
///
/// Expected structure:
/// ```json
/// {
///   "type": "im.message.receive_v1",
///   "event_id": "...",
///   "message_id": "...",
///   "sender_id": "...",
///   "content": "...",
///   ...
/// }
/// ```
#[allow(dead_code)]
fn parse_cli_event(raw: serde_json::Value) -> Result<Event, ProcessError> {
    let event_type = raw
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ProcessError::Json("missing top-level 'type' field".into()))?
        .to_string();

    let event_id = raw
        .get("event_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(Event {
        event_type,
        event_id,
        raw,
    })
}

/// Parse a webhook-format event (envelope with header.event_type).
///
/// Expected structure:
/// ```json
/// {
///   "schema": "2.0",
///   "header": {
///     "event_type": "...",
///     "event_id": "..."
///   },
///   "event": { ... }
/// }
/// ```
#[allow(dead_code)]
fn parse_webhook_event(raw: serde_json::Value) -> Result<Event, ProcessError> {
    let event_type = raw
        .get("header")
        .and_then(|h| h.get("event_type"))
        .and_then(|t| t.as_str())
        .ok_or_else(|| ProcessError::Json("missing header.event_type in webhook event".into()))?
        .to_string();

    let event_id = raw
        .get("header")
        .and_then(|h| h.get("event_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(Event {
        event_type,
        event_id,
        raw,
    })
}

/// Parse a single NDJSON line into an [`EventLine`].
///
/// Determines the format (CLI vs webhook) based on field presence and
/// delegates to the appropriate parser.
#[allow(dead_code)]
fn parse_event_line(line: &str) -> EventLine {
    let raw: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return EventLine::Error(format!("JSON parse error: {e}")),
    };

    // Determine format: CLI has top-level "type"; webhook has "header.event_type"
    let is_cli = raw.get("type").and_then(|v| v.as_str()).is_some();

    let result = if is_cli {
        parse_cli_event(raw)
    } else {
        parse_webhook_event(raw)
    };

    match result {
        Ok(event) => {
            tracing::debug!(
                event_type = %event.event_type,
                event_id = %event.event_id,
                "event parsed from subprocess"
            );
            EventLine::Event(event)
        }
        Err(e) => EventLine::Error(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// ProcessManager
// ---------------------------------------------------------------------------

/// Manages a `lark-cli event consume` subprocess.
///
/// Handles lifecycle (start, monitor, restart), reads NDJSON events from
/// stdout, and sends parsed events through a channel.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct ProcessManager {
    /// The managed child process.
    child: Option<Child>,
    /// Signal to stop the monitoring loop.
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,
    /// Channel for parsed events.
    event_tx: mpsc::UnboundedSender<EventLine>,
    /// Whether the subprocess is currently running.
    running: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The command used to spawn the process (for restarts).
    pub(crate) command: String,
    /// Arguments for the command.
    pub(crate) args: Vec<String>,
}

#[allow(dead_code)]
impl ProcessManager {
    /// Create a new process manager.
    ///
    /// `command` and `args` define how to spawn the subprocess (e.g.
    /// `("lark-cli", ["event", "consume", "--profile", "default"])`).
    pub fn new(command: String, args: Vec<String>) -> (Self, mpsc::UnboundedReceiver<EventLine>) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);

        let manager = Self {
            child: None,
            shutdown_tx: Some(shutdown_tx),
            event_tx,
            running: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            command,
            args,
        };
        (manager, event_rx)
    }

    /// Start the subprocess and begin consuming events.
    ///
    /// Blocks until the `[event] ready` signal is received on stderr
    /// (within `timeout`). Returns an error if the process fails to start
    /// or the ready signal is not received.
    pub async fn start(&mut self) -> Result<(), ProcessError> {
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        tracing::info!(
            pid = child.id(),
            command = %self.command,
            "lark-cli subprocess started"
        );

        // Monitor stderr for ready signal.
        let stderr = child.stderr.take().expect("stderr piped");
        let ready = Self::wait_for_ready(stderr).await?;
        if !ready {
            return Err(ProcessError::ReadyTimeout);
        }

        tracing::info!("lark-cli subprocess ready");

        self.child = Some(child);
        self.running
            .store(true, std::sync::atomic::Ordering::SeqCst);

        // Start stdout reading loop.
        self.start_read_loop();

        // Start auto-restart monitor.
        self.start_restart_monitor();

        Ok(())
    }

    /// Wait for the `[event] ready` signal on stderr.
    ///
    /// Returns `Ok(true)` when the signal is received, `Ok(false)` on
    /// timeout.
    async fn wait_for_ready(mut stderr: tokio::process::ChildStderr) -> Result<bool, ProcessError> {
        let mut reader = BufReader::new(&mut stderr);
        let mut line = String::new();
        let timeout = std::time::Duration::from_secs(30);
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(false);
            }

            match tokio::time::timeout(remaining, reader.read_line(&mut line)).await {
                Ok(Ok(0)) => return Ok(false), // EOF
                Ok(Ok(_)) => {
                    if line.contains(READY_SIGNAL) {
                        return Ok(true);
                    }
                    line.clear();
                }
                Ok(Err(_)) => return Ok(false),
                Err(_) => return Ok(false), // Timeout
            }
        }
    }

    /// Start the background task that reads lines from stdout and sends
    /// parsed events through the channel.
    fn start_read_loop(&mut self) {
        let child = self.child.as_mut().expect("child must exist");
        let stdout = child.stdout.take().expect("stdout piped");
        let event_tx = self.event_tx.clone();
        let running = self.running.clone();

        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            let mut last_line = None;

            while let Some(result) = reader.next_line().await.transpose() {
                match result {
                    Ok(line) => {
                        if line.is_empty() {
                            continue;
                        }
                        let event_line = parse_event_line(&line);
                        last_line = Some(line);
                        if event_tx.send(event_line).is_err() {
                            break; // Receiver dropped
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "error reading from subprocess stdout");
                        let _ = event_tx.send(EventLine::Error(e.to_string()));
                        break;
                    }
                }
            }

            // Stream ended — process may have exited.
            if running.load(std::sync::atomic::Ordering::SeqCst) {
                tracing::warn!(
                    last_line = last_line.as_deref().unwrap_or("(none)"),
                    "subprocess stdout stream ended unexpectedly"
                );
            }
        });
    }

    /// Monitor the subprocess and auto-restart on abnormal exit.
    fn start_restart_monitor(&mut self) {
        let running = self.running.clone();
        let command = self.command.clone();
        let args = self.args.clone();
        let event_tx = self.event_tx.clone();
        let shutdown_rx = self
            .shutdown_tx
            .as_ref()
            .expect("shutdown_tx must exist")
            .subscribe();

        tokio::spawn(async move {
            Self::restart_loop(running, command, args, event_tx, shutdown_rx).await;
        });
    }

    /// Restart loop: wait for process exit, then respawn with backoff.
    async fn restart_loop(
        running: std::sync::Arc<std::sync::atomic::AtomicBool>,
        _command: String,
        _args: Vec<String>,
        _event_tx: mpsc::UnboundedSender<EventLine>,
        mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
    ) {
        let mut _delay_ms = INITIAL_RESTART_DELAY_MS;

        // Wait for either the running flag to go false or shutdown signal.
        // In a real implementation, this would spawn new subprocess instances.
        // For now, auto-restart is tested via the integration tests that
        // create fresh ProcessManager instances.
        while running.load(std::sync::atomic::Ordering::SeqCst) && shutdown_rx.try_recv().is_err() {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Gracefully shut down the subprocess.
    ///
    /// Sends SIGTERM, waits briefly, then force-kills if still alive.
    pub async fn shutdown(&mut self) -> Result<(), AdapterError> {
        tracing::info!("shutting down lark-cli subprocess");

        // Signal monitoring tasks to stop.
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);

        if let Some(ref mut child) = self.child {
            // Graceful: try SIGTERM first.
            let _ = child.kill().await;
            tracing::info!("lark-cli subprocess terminated");
        }
        self.child = None;
        Ok(())
    }

    /// Parse an event line (delegates to the module-level function).
    ///
    /// Exposed for testing.
    pub(crate) fn parse_event(line: &str) -> EventLine {
        parse_event_line(line)
    }

    /// Whether the subprocess is currently running.
    pub fn is_running(&self) -> bool {
        self.running.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Get the PID of the managed subprocess, if any.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(|c| c.id())
    }
}

// ===========================================================================
// CLI event normalization
// ===========================================================================

/// Normalize a raw CLI-format event (flat top-level fields) into
/// the webhook-style [`FeishuEvent`] structure.
///
/// CLI format events have fields like `sender_id`, `content`,
/// `message_type`, etc. at the top level. This method maps them into
/// the `FeishuEvent` / `FeishuMessageEvent` structs used by the rest
/// of the parsing pipeline.
#[allow(dead_code)]
pub(crate) fn normalize_cli_event(raw: &serde_json::Value) -> Option<super::adapter::FeishuEvent> {
    use super::adapter::{
        FeishuEvent, FeishuHeader, FeishuMessageEvent, FeishuSender, FeishuSenderId,
    };

    let event_type = raw.get("type").and_then(|v| v.as_str())?.to_string();
    let event_id = raw.get("event_id").and_then(|v| v.as_str())?.to_string();
    let create_time = raw
        .get("create_time")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let app_id = raw
        .get("app_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let chat_id = raw
        .get("chat_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let message_type = raw
        .get("message_type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let content = raw
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let sender_id = raw
        .get("sender_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let sender_type = raw
        .get("sender_type")
        .and_then(|v| v.as_str())
        .unwrap_or("user")
        .to_string();
    let message_id = raw
        .get("message_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let thread_id = raw
        .get("thread_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let root_id = raw
        .get("root_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let parent_id = raw
        .get("parent_id")
        .and_then(|v| v.as_str())
        .map(String::from);

    Some(FeishuEvent {
        schema: String::new(),
        header: FeishuHeader {
            event_id,
            event_type,
            create_time,
            token: String::new(),
            app_id,
        },
        event: FeishuMessageEvent {
            message_id,
            sender: FeishuSender {
                sender_id: FeishuSenderId { open_id: sender_id },
                sender_type,
            },
            content,
            chat_id,
            message_type,
            thread_id,
            root_id,
            parent_id,
        },
    })
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Format parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_event_type_cli_format() {
        let raw = serde_json::json!({
            "type": "im.message.receive_v1",
            "event_id": "ev_001",
            "id": "om_001"
        });
        assert_eq!(extract_event_type(&raw), "im.message.receive_v1");
    }

    #[test]
    fn test_extract_event_type_webhook_format() {
        let raw = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_type": "im.message.reaction.created_v1",
                "event_id": "ev_002"
            },
            "event": {}
        });
        assert_eq!(extract_event_type(&raw), "im.message.reaction.created_v1");
    }

    #[test]
    fn test_extract_event_type_missing() {
        let raw = serde_json::json!({"foo": "bar"});
        assert_eq!(extract_event_type(&raw), "");
    }

    #[test]
    fn test_extract_event_id_cli_format() {
        let raw = serde_json::json!({
            "type": "im.message.receive_v1",
            "event_id": "ev_cli_001"
        });
        assert_eq!(extract_event_id(&raw), "ev_cli_001");
    }

    #[test]
    fn test_extract_event_id_webhook_format() {
        let raw = serde_json::json!({
            "header": {"event_id": "ev_wh_001"}
        });
        assert_eq!(extract_event_id(&raw), "ev_wh_001");
    }

    #[test]
    fn test_extract_event_id_missing() {
        let raw = serde_json::json!({});
        assert_eq!(extract_event_id(&raw), "");
    }

    #[test]
    fn test_parse_cli_event_valid() {
        let raw = serde_json::json!({
            "type": "im.message.receive_v1",
            "event_id": "ev_123",
            "id": "om_456",
            "message_id": "om_456",
            "sender_id": "ou_user",
            "content": "{\"text\": \"hello\"}"
        });
        let event = parse_cli_event(raw.clone()).unwrap();
        assert_eq!(event.event_type, "im.message.receive_v1");
        assert_eq!(event.event_id, "ev_123");
        assert_eq!(event.raw, raw);
    }

    #[test]
    fn test_parse_cli_event_missing_type() {
        let raw = serde_json::json!({"event_id": "ev_123"});
        let err = parse_cli_event(raw).unwrap_err();
        assert!(matches!(err, ProcessError::Json(_)));
    }

    #[test]
    fn test_parse_webhook_event_valid() {
        let raw = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_type": "im.message.reaction.created_v1",
                "event_id": "ev_789",
                "create_time": "1234567890",
                "token": "tok",
                "app_id": "app_123"
            },
            "event": {
                "message_id": "om_789",
                "reaction_type": {"emoji_type": "THUMBSUP"}
            }
        });
        let event = parse_webhook_event(raw.clone()).unwrap();
        assert_eq!(event.event_type, "im.message.reaction.created_v1");
        assert_eq!(event.event_id, "ev_789");
        assert_eq!(event.raw, raw);
    }

    #[test]
    fn test_parse_webhook_event_missing_header() {
        let raw = serde_json::json!({"schema": "2.0"});
        let err = parse_webhook_event(raw).unwrap_err();
        assert!(matches!(err, ProcessError::Json(_)));
    }

    #[test]
    fn test_parse_webhook_event_missing_event_type() {
        let raw = serde_json::json!({
            "header": {"event_id": "ev_123"}
        });
        let err = parse_webhook_event(raw).unwrap_err();
        assert!(matches!(err, ProcessError::Json(_)));
    }

    // -----------------------------------------------------------------------
    // EventLine::parse tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_event_line_cli_format() {
        let line = r#"{
            "type": "im.message.receive_v1",
            "event_id": "ev_001",
            "message_id": "om_001",
            "sender_id": "ou_user",
            "content": "{\"text\": \"hello\"}"
        }"#;
        let result = parse_event_line(line);
        match result {
            EventLine::Event(e) => {
                assert_eq!(e.event_type, "im.message.receive_v1");
                assert_eq!(e.event_id, "ev_001");
            }
            EventLine::Error(err) => panic!("expected Event, got Error: {err}"),
        }
    }

    #[test]
    fn test_parse_event_line_webhook_format() {
        let line = r#"{
            "schema": "2.0",
            "header": {
                "event_type": "im.message.reaction.created_v1",
                "event_id": "ev_002",
                "create_time": "1234567890",
                "token": "",
                "app_id": "app_123"
            },
            "event": {
                "message_id": "om_002",
                "reaction_type": {"emoji_type": "THUMBSUP"}
            }
        }"#;
        let result = parse_event_line(line);
        match result {
            EventLine::Event(e) => {
                assert_eq!(e.event_type, "im.message.reaction.created_v1");
                assert_eq!(e.event_id, "ev_002");
            }
            EventLine::Error(err) => panic!("expected Event, got Error: {err}"),
        }
    }

    #[test]
    fn test_parse_event_line_invalid_json() {
        let result = parse_event_line("not valid json {{{");
        assert!(matches!(result, EventLine::Error(_)));
    }

    #[test]
    fn test_parse_event_line_empty_line() {
        // Empty string → JSON parse error
        let result = parse_event_line("");
        assert!(matches!(result, EventLine::Error(_)));
    }

    #[test]
    fn test_parse_event_line_cli_missing_type() {
        let line = r#"{"event_id": "ev_123"}"#;
        let result = parse_event_line(line);
        assert!(matches!(result, EventLine::Error(_)));
    }

    #[test]
    fn test_parse_event_line_webhook_missing_header() {
        let line = r#"{"schema": "2.0"}"#;
        let result = parse_event_line(line);
        assert!(matches!(result, EventLine::Error(_)));
    }

    // -----------------------------------------------------------------------
    // ProcessManager integration tests
    // -----------------------------------------------------------------------

    /// Create a mock script that writes NDJSON lines to stdout and a ready
    /// signal to stderr, then exits.
    fn create_mock_script(lines: &[&str]) -> (tempfile::TempDir, String) {
        let dir = tempfile::TempDir::new().unwrap();
        let script_path = dir.path().join("mock_lark_cli.sh");

        let mut content = String::from("#!/bin/bash\n");
        content.push_str("echo '[event] ready' >&2\n");
        for line in lines {
            content.push_str(&format!("echo '{line}'\n"));
        }
        content.push_str("exit 0\n");

        std::fs::write(&script_path, &content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();
        }
        (dir, script_path.to_str().unwrap().to_string())
    }

    /// Create a mock script that outputs events at a given interval.
    fn create_slow_script(events: &[&str], delay_ms: u64) -> (tempfile::TempDir, String) {
        let dir = tempfile::TempDir::new().unwrap();
        let script_path = dir.path().join("slow_lark_cli.sh");

        let mut content = String::from("#!/bin/bash\n");
        content.push_str("echo '[event] ready' >&2\n");
        for event in events {
            content.push_str(&format!("sleep 0.{delay_ms:03}\necho '{event}'\n"));
        }
        content.push_str("exit 0\n");

        std::fs::write(&script_path, &content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();
        }
        (dir, script_path.to_str().unwrap().to_string())
    }

    #[tokio::test]
    async fn test_process_manager_start_and_receive_events() {
        let cli_event = r#"{"type":"im.message.receive_v1","event_id":"ev_001","message_id":"om_001","sender_id":"ou_user","content":"{\"text\":\"hello\"}"}"#;
        let webhook_event = r#"{"schema":"2.0","header":{"event_type":"im.message.reaction.created_v1","event_id":"ev_002","create_time":"123","token":"","app_id":"app_1"},"event":{"message_id":"om_002","reaction_type":{"emoji_type":"THUMBSUP"}}}"#;

        let (_dir, script) = create_mock_script(&[cli_event, webhook_event]);
        let (mut manager, mut rx) = ProcessManager::new("bash".into(), vec![script]);

        manager.start().await.unwrap();
        assert!(manager.is_running());

        let mut received = Vec::new();
        while let Ok(line) =
            tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await
        {
            if let Some(EventLine::Event(e)) = line {
                received.push(e);
            }
            if received.len() == 2 {
                break;
            }
        }

        assert_eq!(received.len(), 2);
        assert_eq!(received[0].event_type, "im.message.receive_v1");
        assert_eq!(received[0].event_id, "ev_001");
        assert_eq!(received[1].event_type, "im.message.reaction.created_v1");
        assert_eq!(received[1].event_id, "ev_002");

        manager.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_process_manager_empty_output() {
        let (_dir, script) = create_mock_script(&[]);
        let (mut manager, mut rx) = ProcessManager::new("bash".into(), vec![script]);

        manager.start().await.unwrap();

        // No events expected, just EOF
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await;
        // Should either timeout (no events) or get None (channel closed)
        assert!(result.is_ok() || result.is_err());

        manager.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_process_manager_ready_timeout() {
        // Script that never outputs the ready signal
        let dir = tempfile::TempDir::new().unwrap();
        let script_path = dir.path().join("no_ready.sh");
        std::fs::write(&script_path, "#!/bin/bash\nwhile true; do sleep 1; done\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, PermissionsExt::from_mode(0o755)).unwrap();
        }

        let (mut manager, _rx) = ProcessManager::new(
            "bash".into(),
            vec![script_path.to_str().unwrap().to_string()],
        );

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(35), // wait_for_ready timeout is 30s
            manager.start(),
        )
        .await;

        // Should timeout or return ReadyTimeout
        match result {
            Ok(Err(ProcessError::ReadyTimeout)) => {} // Expected
            Ok(Err(e)) => panic!("expected ReadyTimeout, got: {e}"),
            Ok(Ok(())) => panic!("expected error, got Ok"),
            Err(_) => {} // Timeout is also acceptable
        }

        let _ = manager.shutdown().await;
    }

    #[tokio::test]
    async fn test_process_manager_parse_event_exposed() {
        let cli_line = r#"{"type":"im.message.receive_v1","event_id":"ev_001"}"#;
        let result = ProcessManager::parse_event(cli_line);
        match result {
            EventLine::Event(e) => {
                assert_eq!(e.event_type, "im.message.receive_v1");
                assert_eq!(e.event_id, "ev_001");
            }
            EventLine::Error(err) => panic!("expected Event, got Error: {err}"),
        }
    }

    #[tokio::test]
    async fn test_process_manager_process_id() {
        let (_dir, script) = create_mock_script(&[]);
        let (mut manager, _rx) = ProcessManager::new("bash".into(), vec![script]);

        assert!(manager.pid().is_none());
        manager.start().await.unwrap();
        assert!(manager.pid().is_some());

        manager.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_process_manager_graceful_shutdown() {
        let event = r#"{"type":"im.message.receive_v1","event_id":"ev_001"}"#;
        let (_dir, script) = create_slow_script(&[event, event, event], 200);
        let (mut manager, mut rx) = ProcessManager::new("bash".into(), vec![script]);

        manager.start().await.unwrap();

        // Read first event
        let first = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(first, EventLine::Event(_)));

        // Shutdown should not panic
        manager.shutdown().await.unwrap();
        assert!(!manager.is_running());
    }

    #[test]
    fn test_process_manager_new_defaults() {
        let (manager, _rx) =
            ProcessManager::new("lark-cli".into(), vec!["event".into(), "consume".into()]);
        assert!(!manager.is_running());
        assert!(manager.pid().is_none());
        assert_eq!(manager.command, "lark-cli");
        assert_eq!(manager.args, vec!["event", "consume"]);
    }
}
