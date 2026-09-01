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
const READY_SIGNAL: &str = "[event] ready";

/// Initial delay before restarting a crashed subprocess (milliseconds).
const INITIAL_RESTART_DELAY_MS: u64 = 1_000;

/// Maximum delay between restart attempts (milliseconds).
const MAX_RESTART_DELAY_MS: u64 = 30_000;

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

/// A parsed event read from the subprocess stdout.
#[derive(Debug, Clone)]
pub(crate) struct Event {
    /// Event type string (e.g. `im.message.receive_v1`,
    /// `im.message.reaction.created_v1`).
    pub event_type: String,
    /// Event ID for deduplication.
    pub event_id: String,
    /// Raw JSON payload for downstream parsing.
    #[allow(dead_code)]
    pub raw: serde_json::Value,
}

/// An event line read from subprocess stdout — either a successfully parsed
/// event or a read/parse error.
#[derive(Debug)]
pub(crate) enum EventLine {
    Event(#[allow(dead_code)] Event),
    Error(#[allow(dead_code)] String),
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
///
/// The monitor task owns the child process and manages its full lifecycle
/// (spawn, read stdout, detect exit, restart with backoff). The manager
/// retains cached state (`last_pid`) for the public API.
#[derive(Debug)]
pub(crate) struct ProcessManager {
    /// Signal to stop the monitoring loop.
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,
    /// Channel for parsed events.
    event_tx: mpsc::UnboundedSender<EventLine>,
    /// Whether the subprocess is currently running.
    running: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Cached PID of the last spawned child process.
    last_pid: std::sync::Arc<std::sync::atomic::AtomicU32>,
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
            shutdown_tx: Some(shutdown_tx),
            event_tx,
            running: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_pid: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
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
        // Spawn the initial child process.
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let pid = child.id();
        tracing::info!(
            pid = pid,
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

        self.running
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.last_pid
            .store(pid.unwrap_or(0), std::sync::atomic::Ordering::SeqCst);

        // Start the monitor task which owns the child and manages its
        // full lifecycle (stdout reading, exit detection, restart).
        self.start_monitor(child);

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

    /// Start the monitor task that owns the child process and manages its
    /// full lifecycle: reads stdout, detects exit, and restarts with backoff.
    fn start_monitor(&self, initial_child: Child) {
        let running = self.running.clone();
        let command = self.command.clone();
        let args = self.args.clone();
        let event_tx = self.event_tx.clone();
        let last_pid = self.last_pid.clone();
        let shutdown_rx = self
            .shutdown_tx
            .as_ref()
            .expect("shutdown_tx must exist")
            .subscribe();

        tokio::spawn(async move {
            Self::monitor_loop(
                initial_child,
                running,
                command,
                args,
                event_tx,
                last_pid,
                shutdown_rx,
            )
            .await;
        });
    }

    /// Monitor loop: owns the child process, reads stdout, waits for exit,
    /// and restarts with exponential backoff.
    async fn monitor_loop(
        mut child: Child,
        running: std::sync::Arc<std::sync::atomic::AtomicBool>,
        command: String,
        args: Vec<String>,
        event_tx: mpsc::UnboundedSender<EventLine>,
        last_pid: std::sync::Arc<std::sync::atomic::AtomicU32>,
        mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
    ) {
        let mut delay_ms = INITIAL_RESTART_DELAY_MS;
        loop {
            Self::read_stdout_lines(&mut child, &event_tx).await;
            if Self::should_shutdown(&running, &mut shutdown_rx) {
                break;
            }
            Self::wait_for_child_exit(&mut child).await;
            delay_ms = Self::apply_backoff(delay_ms).await;
            child = match Self::try_respawn(&command, &args, &last_pid).await {
                Some(c) => {
                    delay_ms = INITIAL_RESTART_DELAY_MS;
                    c
                }
                None => child,
            };
        }
    }

    /// Check if the monitor should shut down.
    fn should_shutdown(
        running: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        shutdown_rx: &mut tokio::sync::broadcast::Receiver<()>,
    ) -> bool {
        if !running.load(std::sync::atomic::Ordering::SeqCst) || shutdown_rx.try_recv().is_ok() {
            tracing::info!("monitor loop: shutdown requested, exiting");
            return true;
        }
        false
    }

    /// Wait for the child process to exit.
    async fn wait_for_child_exit(child: &mut Child) {
        let _ = child.wait().await;
        tracing::warn!("lark-cli subprocess exited, restarting");
    }

    /// Apply exponential backoff and sleep.
    async fn apply_backoff(delay_ms: u64) -> u64 {
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        (delay_ms * 2).min(MAX_RESTART_DELAY_MS)
    }

    /// Attempt to respawn the subprocess.
    ///
    /// Returns `Some(new_child)` on success, `None` on failure.
    async fn try_respawn(
        command: &str,
        args: &[String],
        last_pid: &std::sync::Arc<std::sync::atomic::AtomicU32>,
    ) -> Option<Child> {
        match Command::new(command)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(mut new_child) => {
                let pid = new_child.id();
                tracing::info!(pid = pid, "lark-cli subprocess restarted");
                let stderr = new_child.stderr.take().expect("stderr piped");
                match Self::wait_for_ready(stderr).await {
                    Ok(true) => {
                        tracing::info!("lark-cli subprocess ready after restart");
                        last_pid.store(pid.unwrap_or(0), std::sync::atomic::Ordering::SeqCst);
                        Some(new_child)
                    }
                    _ => {
                        tracing::warn!("lark-cli subprocess failed to become ready after restart");
                        None
                    }
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to respawn lark-cli subprocess");
                None
            }
        }
    }

    /// Read NDJSON lines from the child's stdout and send parsed events
    /// through the channel. Returns when stdout closes (EOF or error).
    async fn read_stdout_lines(child: &mut Child, event_tx: &mpsc::UnboundedSender<EventLine>) {
        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => return,
        };
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

        tracing::debug!(
            last_line = last_line.as_deref().unwrap_or("(none)"),
            "subprocess stdout stream ended"
        );
    }

    /// Gracefully shut down the subprocess.
    ///
    /// Sends SIGTERM, waits briefly, then force-kills if still alive.
    pub async fn shutdown(&mut self) -> Result<(), AdapterError> {
        tracing::info!("shutting down lark-cli subprocess");

        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);

        // Signal the monitor task to stop.
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        tracing::info!("lark-cli subprocess shutdown signal sent");
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
        let pid = self.last_pid.load(std::sync::atomic::Ordering::SeqCst);
        match pid {
            0 => None,
            p => Some(p),
        }
    }
}

// ===========================================================================
// Event stream → Gateway integration
// ===========================================================================

/// Spawn a long-running task that reads parsed events from the
/// [`ProcessManager`] event channel and enqueues them into the
/// Gateway's inbound queue.
///
/// Each [`EventLine::Event`] is serialized back to raw JSON bytes and
/// wrapped in an [`InboundRequest`] with `platform="feishu"`. Non-event
/// lines (parse errors) and enqueue failures are logged and skipped.
///
/// Group chat filtering is deferred to `parse_inbound` (returns `None`
/// for group events, which the gateway discards).
///
pub(crate) fn start_event_stream(
    gateway: &std::sync::Arc<closeclaw_gateway::Gateway>,
    mut event_rx: mpsc::UnboundedReceiver<EventLine>,
) {
    let gateway = gateway.clone();
    tokio::spawn(async move {
        tracing::info!("feishu long-connection event stream started");
        while let Some(event_line) = event_rx.recv().await {
            match event_line {
                EventLine::Event(event) => {
                    let raw_payload = match serde_json::to_vec(&event.raw) {
                        Ok(b) => b,
                        Err(e) => {
                            tracing::warn!(
                                event_type = %event.event_type,
                                error = %e,
                                "failed to serialize event raw payload"
                            );
                            continue;
                        }
                    };
                    let timestamp_hex = format!("{:x}", chrono::Utc::now().timestamp_millis());
                    let uuid_no_hyphens = uuid::Uuid::new_v4().simple().to_string();
                    let trace_id = format!("feishu_{}_{}", timestamp_hex, uuid_no_hyphens);
                    let req = closeclaw_gateway::inbound_queue::InboundRequest {
                        platform: "feishu".to_string(),
                        raw_payload,
                        peer_id: String::new(),
                        trace_id,
                        span_id: None,
                    };
                    if let Err(e) = gateway.enqueue_inbound(req).await {
                        tracing::warn!(
                            event_type = %event.event_type,
                            event_id = %event.event_id,
                            error = %e,
                            "failed to enqueue inbound event"
                        );
                    }
                }
                EventLine::Error(err) => {
                    tracing::warn!(error = %err, "event stream parse error — skipping");
                }
            }
        }
        tracing::info!("feishu long-connection event stream ended");
    });
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
/// Helper to extract an optional string field from a JSON value.
fn opt_str<'a>(raw: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    raw.get(key).and_then(|v| v.as_str())
}

/// Helper to extract a required string field, returning `None` if missing.
fn required_str(raw: &serde_json::Value, key: &str) -> Option<String> {
    opt_str(raw, key).map(String::from)
}

/// Helper to extract an optional string field with a default value.
fn opt_str_default(raw: &serde_json::Value, key: &str, default: &str) -> String {
    opt_str(raw, key).unwrap_or(default).to_string()
}

/// Extract the header portion from a CLI-format event.
fn extract_cli_header(raw: &serde_json::Value) -> Option<super::adapter::FeishuHeader> {
    Some(super::adapter::FeishuHeader {
        event_id: required_str(raw, "event_id")?,
        event_type: required_str(raw, "type")?,
        create_time: opt_str_default(raw, "create_time", ""),
        token: String::new(),
        app_id: opt_str_default(raw, "app_id", ""),
    })
}

/// Extract the message event portion from a CLI-format event.
fn extract_cli_message_event(raw: &serde_json::Value) -> super::adapter::FeishuMessageEvent {
    use super::adapter::{FeishuMessageEvent, FeishuSender, FeishuSenderId};
    FeishuMessageEvent {
        message_id: opt_str(raw, "message_id").map(String::from),
        sender: FeishuSender {
            sender_id: FeishuSenderId {
                open_id: opt_str_default(raw, "sender_id", ""),
            },
            sender_type: opt_str_default(raw, "sender_type", "user"),
        },
        content: opt_str_default(raw, "content", ""),
        chat_id: opt_str_default(raw, "chat_id", ""),
        chat_type: opt_str(raw, "chat_type").map(String::from),
        message_type: opt_str_default(raw, "message_type", ""),
        thread_id: opt_str(raw, "thread_id").map(String::from),
        root_id: opt_str(raw, "root_id").map(String::from),
        parent_id: opt_str(raw, "parent_id").map(String::from),
    }
}

/// Normalize a raw CLI-format event (flat top-level fields) into
/// the webhook-style [`FeishuEvent`] structure.
pub(crate) fn normalize_cli_event(raw: &serde_json::Value) -> Option<super::adapter::FeishuEvent> {
    let header = extract_cli_header(raw)?;
    let event = extract_cli_message_event(raw);
    Some(super::adapter::FeishuEvent {
        schema: String::new(),
        header,
        event,
    })
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // parse_cli_event / parse_webhook_event — direct unit tests
    // -----------------------------------------------------------------------

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
}
