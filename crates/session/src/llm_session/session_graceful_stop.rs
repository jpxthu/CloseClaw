//! Graceful-stop / force-kill surface for `ConversationSession`.
//!
//! Extracted from `session_handles.rs` (issue #3164):
//! [`ConversationSession::graceful_stop`] with its wait-state
//! inspection helpers (`collect_waiting_items` /
//! `has_running_tools`), plus [`ConversationSession::force_kill`].

use std::time::Duration;

use tokio::sync::mpsc;

use closeclaw_common::{LlmState, ToolExecState};

use super::session_handles::{GracefulStopProgress, GracefulStopResult, STOP_KILL_TIMEOUT};
use super::ConversationSession;

#[allow(dead_code)]
impl ConversationSession {
    /// Wait for in-flight operations to complete, with a hard timeout.
    ///
    /// Polls every 100 ms for LLM streaming and tool execution state.
    /// Checks for forceful escalation via the session's shutdown handle.
    /// On timeout, sends a [`GracefulStopProgress`] report via
    /// `progress_tx` and returns [`GracefulStopResult::TimedOut`].
    ///
    /// This method does **not** clean up or persist state — that is
    /// the responsibility of `stop()` (cleanup) and the caller
    /// (`stop_single_session` in SessionManager, persistence).
    pub async fn graceful_stop(
        &self,
        timeout: Duration,
        progress_tx: Option<mpsc::Sender<GracefulStopProgress>>,
    ) -> GracefulStopResult {
        let start = tokio::time::Instant::now();
        let mut streaming_seen = false;

        loop {
            // ── forceful escalation check ────────────────────────────
            if let Some(sh) = self.get_shutdown_handle() {
                if sh.is_forceful() {
                    tracing::info!(
                        session_id = %self.session_id,
                        "graceful_stop: interrupted by forceful escalation"
                    );
                    return GracefulStopResult::Interrupted;
                }
            }

            // ── timeout check ────────────────────────────────────────
            if start.elapsed() >= timeout {
                let waiting = self.collect_waiting_items(start);
                let remaining = waiting.len();
                if let Some(tx) = progress_tx {
                    let _ = tx
                        .send(GracefulStopProgress {
                            waiting_items: waiting,
                            remaining,
                        })
                        .await;
                }
                return GracefulStopResult::TimedOut;
            }

            // ── state inspection ─────────────────────────────────────
            let state = self.llm_state();
            let is_streaming = matches!(state, LlmState::Receiving | LlmState::Requesting);
            let has_running_tools = self.has_running_tools();

            if is_streaming {
                streaming_seen = true;
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }

            if streaming_seen {
                if !has_running_tools {
                    // LLM stream ended and no tools are running.
                    // Pending tool calls (if any) will be extracted
                    // by the caller via extract_pending_tool_calls()
                    // and persisted to the checkpoint — matching the
                    // design doc requirement to record pending ops
                    // without executing tools.
                    return GracefulStopResult::Completed;
                }
                // Tools still running — wait for them.
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }

            // Not streaming — idle path.
            if !has_running_tools {
                return GracefulStopResult::Completed;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

#[allow(dead_code)]
impl ConversationSession {
    /// Forcefully kill tool processes and cancel in-flight LLM requests.
    ///
    /// Used by the forceful-shutdown path in
    /// `SessionManager::stop_single_session` to immediately terminate
    /// running tool processes and cancel ongoing LLM streams without
    /// clearing execution state. This allows
    /// [`collect_pending_operations`](Self::collect_pending_operations) to
    /// still observe tool_states / child_states for checkpoint recording.
    ///
    /// Operation order matches the design doc stop sequence: kill tool
    /// processes first, then cancel in-flight LLM requests. This ensures
    /// tool processes are terminated before the cancel token fires,
    /// preventing partial cleanup from racing with in-flight tools.
    ///
    /// Incomplete assistant message fragments are discarded: the cancel
    /// token causes in-flight streaming LLM calls to return
    /// `LLMError::Cancelled`, and the Gateway layer does not append
    /// partial assistant messages to the conversation history.
    ///
    /// This method is idempotent — cancelling an already-cancelled token
    /// and killing already-cleared handles are harmless no-ops.
    pub async fn force_kill(&self) {
        // Kill every registered tool process first, per design doc
        // stop sequence: cascade → kill tools → cancel LLM → cleanup.
        self.kill_tool_handles(STOP_KILL_TIMEOUT).await;

        // Then cancel in-flight LLM requests. Streaming calls observe
        // this via `cancel_token.cancelled()` in the Gateway's
        // `call_llm_streaming` select branch and return Cancelled.
        self.cancel_token.cancel();

        tracing::info!(
            session_id = %self.session_id,
            "force_kill: tool processes killed, LLM requests cancelled"
        );
    }

    /// Collect the names and elapsed times of items still in flight.
    fn collect_waiting_items(&self, start: tokio::time::Instant) -> Vec<(String, Duration)> {
        let elapsed = start.elapsed();
        let mut items = Vec::new();

        let state = self.llm_state();
        if matches!(state, LlmState::Receiving | LlmState::Requesting) {
            items.push(("llm_stream".to_string(), elapsed));
        }

        let tool_states = self.tool_states.read().expect("tool_states lock poisoned");
        for (id, (state, _)) in tool_states.iter() {
            if matches!(
                state,
                ToolExecState::RunningForeground | ToolExecState::RunningBackground
            ) {
                items.push((id.clone(), elapsed));
            }
        }
        items
    }

    /// Returns `true` if any tool is in a running state.
    fn has_running_tools(&self) -> bool {
        let tool_states = self.tool_states.read().expect("tool_states lock poisoned");
        tool_states.values().any(|(s, _)| {
            matches!(
                s,
                ToolExecState::RunningForeground | ToolExecState::RunningBackground
            )
        })
    }
}
