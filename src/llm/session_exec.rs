//! Three-dimensional execution state methods for `ConversationSession`.
//!
//! Implements state transition and inspection for the LLM / tool / child
//! session dimensions defined in `session_state.rs`. The overall
//! `exec_status()` combines the three dimensions according to the state
//! table in `docs/design/session/session-execution.md`.

use super::session::ConversationSession;
use super::session_state::{ChildSessionState, LlmState, SessionExecStatus, ToolExecState};

#[allow(dead_code)] // Callers (gateway, tests) are integrated in later steps.
impl ConversationSession {
    // ── LLM state ─────────────────────────────────────────────────────────

    /// Sets the LLM interaction state.
    pub(crate) fn set_llm_state(&self, state: LlmState) {
        let mut guard = self.llm_state.write().expect("llm_state lock poisoned");
        *guard = state;
    }

    /// Returns the current LLM interaction state.
    pub(crate) fn llm_state(&self) -> LlmState {
        let guard = self.llm_state.read().expect("llm_state lock poisoned");
        *guard
    }

    // ── tool state ────────────────────────────────────────────────────────

    /// Registers a new tool call. Returns `true` if newly registered,
    /// `false` if a call with the same id already exists.
    pub(crate) fn register_tool_call(&self, call_id: impl Into<String>) -> bool {
        let id = call_id.into();
        let mut states = self.tool_states.write().expect("tool_states lock poisoned");
        states.insert(id, ToolExecState::Pending).is_none()
    }

    /// Updates the state of a registered tool call. If the id is not
    /// registered, logs a warning and does nothing.
    pub(crate) fn update_tool_state(&self, call_id: &str, state: ToolExecState) {
        let mut states = self.tool_states.write().expect("tool_states lock poisoned");
        match states.get_mut(call_id) {
            Some(existing) => *existing = state,
            None => tracing::warn!(
                call_id = %call_id,
                "update_tool_state: call_id not registered"
            ),
        }
    }

    /// Deregisters a tool call. If the id is not registered, logs a
    /// warning and returns (no-op, no panic).
    pub(crate) fn deregister_tool_call(&self, call_id: &str) {
        let mut states = self.tool_states.write().expect("tool_states lock poisoned");
        if states.remove(call_id).is_none() {
            tracing::warn!(
                call_id = %call_id,
                "deregister_tool_call: call_id not registered"
            );
        }
    }

    /// Returns whether any tool call is currently running in the foreground.
    pub(crate) fn has_active_foreground_tool(&self) -> bool {
        let states = self.tool_states.read().expect("tool_states lock poisoned");
        states
            .values()
            .any(|s| matches!(s, ToolExecState::RunningForeground))
    }

    /// Returns whether any tool call is currently running in the background.
    pub(crate) fn has_active_background_tool(&self) -> bool {
        let states = self.tool_states.read().expect("tool_states lock poisoned");
        states
            .values()
            .any(|s| matches!(s, ToolExecState::RunningBackground))
    }

    // ── child session state ──────────────────────────────────────────────

    /// Registers a new child session in the `Running` state. Returns
    /// `true` if newly registered, `false` if a child with the same id
    /// already exists.
    pub(crate) fn register_child(&self, child_id: impl Into<String>) -> bool {
        let id = child_id.into();
        let mut states = self
            .child_states
            .write()
            .expect("child_states lock poisoned");
        states.insert(id, ChildSessionState::Running).is_none()
    }

    /// Updates the state of a registered child session. If the id is
    /// not registered, logs a warning and does nothing.
    pub(crate) fn update_child_state(&self, child_id: &str, state: ChildSessionState) {
        let mut states = self
            .child_states
            .write()
            .expect("child_states lock poisoned");
        match states.get_mut(child_id) {
            Some(existing) => *existing = state,
            None => tracing::warn!(
                child_id = %child_id,
                "update_child_state: child_id not registered"
            ),
        }
    }

    /// Deregisters a child session. If the id is not registered, logs a
    /// warning and returns (no-op, no panic).
    pub(crate) fn deregister_child(&self, child_id: &str) {
        let mut states = self
            .child_states
            .write()
            .expect("child_states lock poisoned");
        if states.remove(child_id).is_none() {
            tracing::warn!(
                child_id = %child_id,
                "deregister_child: child_id not registered"
            );
        }
    }

    /// Returns whether any child session is currently running.
    pub(crate) fn has_running_child(&self) -> bool {
        let states = self
            .child_states
            .read()
            .expect("child_states lock poisoned");
        states
            .values()
            .any(|s| matches!(s, ChildSessionState::Running))
    }

    // ── overall status ───────────────────────────────────────────────────

    /// Computes the overall session execution status by combining the
    /// three dimensions. Lock acquisition order is **always**
    /// LLM → Tool → Child to avoid potential deadlocks.
    pub(crate) fn exec_status(&self) -> SessionExecStatus {
        // 1. LLM dimension.
        let llm = self.llm_state.read().expect("llm_state lock poisoned");
        if matches!(*llm, LlmState::Requesting | LlmState::Receiving) {
            return SessionExecStatus::Busy;
        }
        drop(llm);

        // 2. Tool dimension.
        let tools = self.tool_states.read().expect("tool_states lock poisoned");
        if tools
            .values()
            .any(|s| matches!(s, ToolExecState::RunningForeground))
        {
            return SessionExecStatus::Busy;
        }
        let has_background_tool = tools
            .values()
            .any(|s| matches!(s, ToolExecState::RunningBackground));
        drop(tools);

        // 3. Child session dimension.
        let children = self
            .child_states
            .read()
            .expect("child_states lock poisoned");
        if children
            .values()
            .any(|s| matches!(s, ChildSessionState::Running))
        {
            return SessionExecStatus::Waiting;
        }

        if has_background_tool {
            SessionExecStatus::IdleWithBackgroundTasks
        } else {
            SessionExecStatus::Idle
        }
    }
}
