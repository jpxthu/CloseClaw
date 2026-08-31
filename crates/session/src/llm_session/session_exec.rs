//! Four-dimensional execution state methods for `ConversationSession`.
//!
//! Implements state transition and inspection for the LLM / tool / child
//! session dimensions defined in `closeclaw_common::session_state`. The
//! overall `exec_status()` combines the four dimensions according to the
//! state table in `docs/design/session/session-execution.md`.

use super::ConversationSession;
use crate::pending_operation_detail::PendingOperationDetail;
use closeclaw_common::{
    ChildSessionState, LlmState, SessionActivityDimensions, SessionExecStatus, ToolExecState,
};

#[allow(dead_code)] // Callers (gateway, tests) are integrated in later steps.
impl ConversationSession {
    // ── LLM state ─────────────────────────────────────────────────────────

    /// Sets the LLM interaction state.
    pub fn set_llm_state(&self, state: LlmState) {
        let mut guard = self.llm_state.write().expect("llm_state lock poisoned");
        *guard = state;
    }

    /// Returns the current LLM interaction state.
    pub fn llm_state(&self) -> LlmState {
        let guard = self.llm_state.read().expect("llm_state lock poisoned");
        *guard
    }

    // ── tool state ────────────────────────────────────────────────────────

    /// Registers a new tool call with detail information.
    ///
    /// Stores the `ToolExecState::Pending` alongside a
    /// [`PendingOperationDetail::ToolCall`] carrying `tool_name` and
    /// `args_summary` so that [`collect_pending_operations`](Self::collect_pending_operations)
    /// can include them in checkpoint data.
    ///
    /// Returns `true` if newly registered, `false` if a call with
    /// the same id already exists.
    pub(crate) fn register_tool_call(
        &self,
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        args_summary: impl Into<String>,
    ) -> bool {
        let id = call_id.into();
        let detail = PendingOperationDetail::ToolCall {
            tool_name: tool_name.into(),
            args_summary: args_summary.into(),
        };
        let mut states = self.tool_states.write().expect("tool_states lock poisoned");
        states
            .insert(id, (ToolExecState::Pending, Some(detail)))
            .is_none()
    }

    /// Updates the state of a registered tool call. If the id is not
    /// registered, logs a warning and does nothing.
    ///
    /// When the new state is terminal ([`ToolExecState::is_terminal`]),
    /// the entry is removed from the map immediately — terminal-state
    /// tools no longer participate in exec-status evaluation.
    pub(crate) fn update_tool_state(&self, call_id: &str, state: ToolExecState) {
        let mut states = self.tool_states.write().expect("tool_states lock poisoned");
        match states.get_mut(call_id) {
            Some((existing, _)) => {
                if state.is_terminal() {
                    states.remove(call_id);
                } else {
                    *existing = state;
                }
            }
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

    /// Returns whether any tool call is currently active in the foreground.
    ///
    /// A tool is considered foreground-active when it is in `Pending`
    /// (just registered, about to execute) or `RunningForeground` state.
    pub(crate) fn has_active_foreground_tool(&self) -> bool {
        let states = self.tool_states.read().expect("tool_states lock poisoned");
        states
            .values()
            .any(|(s, _)| matches!(s, ToolExecState::Pending | ToolExecState::RunningForeground))
    }

    /// Returns whether any tool call is currently running in the background.
    pub(crate) fn has_active_background_tool(&self) -> bool {
        let states = self.tool_states.read().expect("tool_states lock poisoned");
        states
            .values()
            .any(|(s, _)| matches!(s, ToolExecState::RunningBackground))
    }

    /// Returns the four-dimensional activity snapshot for this session.
    ///
    /// Each boolean maps to an independent activity dimension as defined
    /// in `docs/design/session/session-execution.md`:
    /// - `llm_active`: `LlmState ∈ {Requesting, Receiving}`
    /// - `foreground_tool_active`: any tool in `Pending | RunningForeground`
    /// - `background_tool_active`: any tool in `RunningBackground`
    /// - `child_active`: any child in `Running`
    pub fn activity_dimensions(&self) -> SessionActivityDimensions {
        let llm = self.llm_state.read().expect("llm_state lock poisoned");
        let llm_active = matches!(*llm, LlmState::Requesting | LlmState::Receiving);
        drop(llm);

        let tools = self.tool_states.read().expect("tool_states lock poisoned");
        let foreground_tool_active = tools
            .values()
            .any(|(s, _)| matches!(s, ToolExecState::Pending | ToolExecState::RunningForeground));
        let background_tool_active = tools
            .values()
            .any(|(s, _)| matches!(s, ToolExecState::RunningBackground));
        drop(tools);

        let child_active = self.has_running_child();

        SessionActivityDimensions {
            llm_active,
            foreground_tool_active,
            background_tool_active,
            child_active,
        }
    }
}

// ── child session state + overall status ─────────────────────────────

#[allow(dead_code)]
impl ConversationSession {
    /// Registers a new child session in the `Running` state with detail information.
    ///
    /// Stores the `ChildSessionState::Running` alongside a
    /// [`PendingOperationDetail::SubSessionSpawn`] carrying `agent_id` and
    /// `task_summary` so that [`collect_pending_operations`](Self::collect_pending_operations)
    /// can include them in checkpoint data.
    ///
    /// Returns `true` if newly registered, `false` if a child with the same id
    /// already exists.
    pub(crate) fn register_child(
        &self,
        child_id: impl Into<String>,
        agent_id: impl Into<String>,
        task_summary: impl Into<String>,
    ) -> bool {
        let id = child_id.into();
        let detail = PendingOperationDetail::SubSessionSpawn {
            child_session_id: id.clone(),
            agent_id: agent_id.into(),
            task_summary: task_summary.into(),
        };
        let mut states = self
            .child_states
            .write()
            .expect("child_states lock poisoned");
        states
            .insert(id, (ChildSessionState::Running, Some(detail)))
            .is_none()
    }

    /// Updates the state of a registered child session. If the id is
    /// not registered, logs a warning and does nothing.
    pub fn update_child_state(&self, child_id: &str, state: ChildSessionState) {
        let mut states = self
            .child_states
            .write()
            .expect("child_states lock poisoned");
        match states.get_mut(child_id) {
            Some((existing, _)) => *existing = state,
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
            .any(|(s, _)| matches!(s, ChildSessionState::Running))
    }

    /// Computes the overall session execution status by combining the
    /// four dimensions (LLM / foreground tool / background tool /
    /// child session) plus the yielding flag. Lock acquisition order
    /// is **always** LLM → Tool to avoid potential deadlocks.
    ///
    /// Per `docs/design/session/session-execution.md`:
    /// - `background_tool_active` and `child_active` do NOT affect
    ///   the idle/busy determination. Only `llm_active` and
    ///   `foreground_tool_active` drive Busy.
    /// - `Waiting` is only returned when the session is actively
    ///   yielding (`is_yielding=true`) with no LLM or foreground
    ///   tool activity.
    pub fn exec_status(&self) -> SessionExecStatus {
        // 1. LLM dimension.
        let llm = self.llm_state.read().expect("llm_state lock poisoned");
        if matches!(*llm, LlmState::Requesting | LlmState::Receiving) {
            return SessionExecStatus::Busy;
        }
        drop(llm);

        // 2. Foreground tool dimension.
        //    Pending and RunningForeground both count as foreground-active
        //    tools.  Pending is the transient state between register and
        //    the first update; it is treated as foreground because the tool
        //    is about to execute and should block the session.
        let tools = self.tool_states.read().expect("tool_states lock poisoned");
        if tools
            .values()
            .any(|(s, _)| matches!(s, ToolExecState::Pending | ToolExecState::RunningForeground))
        {
            return SessionExecStatus::Busy;
        }
        drop(tools);

        // 3. Yielding dimension.
        //    When the session is actively yielding (agent called
        //    sessions_yield), return Waiting — child_active does NOT
        //    cause Waiting.
        if self.is_waiting() {
            return SessionExecStatus::Waiting;
        }

        // 4. Background tool / child dimensions do NOT affect idle.
        //    Per design doc: background_tool_active and child_active
        //    are exposed via activity_dimensions() for consumers that
        //    need them, but exec_status() returns Idle when only these
        //    dimensions are active.
        SessionExecStatus::Idle
    }
}

// ── Spawn guard (first-layer defense) ─────────────────────────────

impl ConversationSession {
    /// Returns the number of child sessions currently in `Running` state.
    pub fn count_active_children(&self) -> usize {
        let states = self
            .child_states
            .read()
            .expect("child_states lock poisoned");
        states
            .values()
            .filter(|(s, _)| matches!(s, ChildSessionState::Running))
            .count()
    }

    /// Returns a summary of all currently active (Running) child sessions.
    ///
    /// Reads `child_states`, filters for `ChildSessionState::Running`,
    /// and extracts `agent_id` + `task_summary` from each
    /// [`PendingOperationDetail::SubSessionSpawn`]. Children whose detail
    /// is `None` (defensive) are silently skipped.
    ///
    /// Returns `None` when there are no running children.
    /// The returned text is meant for injection as a system message.
    pub fn active_children_summary(&self) -> Option<String> {
        let states = self
            .child_states
            .read()
            .expect("child_states lock poisoned");
        let summaries: Vec<String> = states
            .values()
            .filter(|(s, _)| matches!(s, ChildSessionState::Running))
            .filter_map(|(_, detail)| {
                if let Some(PendingOperationDetail::SubSessionSpawn {
                    agent_id,
                    task_summary,
                    ..
                }) = detail
                {
                    Some(format!("- {} : {}", agent_id, task_summary))
                } else {
                    None
                }
            })
            .collect();
        if summaries.is_empty() {
            return None;
        }
        Some(format!("当前活跃子 Session:\n{}", summaries.join("\n")))
    }

    /// Returns a spawn-guard reminder if the parent should yield.
    ///
    /// First-layer defense against silent spawn failures: when a parent
    /// agent has spawned child sessions but has not called
    /// `sessions_yield`, the system injects a reminder message so the
    /// LLM is prompted to yield and wait for results.
    ///
    /// Returns `Some(reminder)` when `has_active_children() && !is_waiting()`;
    /// `None` otherwise. The returned text is meant to be injected as a
    /// temporary system message (not persisted to checkpoint).
    pub fn spawn_guard_reminder(&self) -> Option<String> {
        if !self.has_active_children() || self.is_waiting() {
            return None;
        }
        let n = self.count_active_children();
        Some(format!(
            "你有 {} 个子 agent 仍在运行，建议 yield 等待结果",
            n
        ))
    }
}
