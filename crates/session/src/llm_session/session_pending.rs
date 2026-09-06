//! Pending operations collection for [`ConversationSession`].
//!
//! Extracts in-flight tool calls, child sessions and outbound messages
//! into [`PendingOperation`] records so they can be persisted in
//! checkpoints and recovered after a daemon restart.

use super::ConversationSession;
use crate::persistence::{
    PendingOperation, PendingOperationDetail as PoDetail, PendingOperationStatus,
    PendingOperationType,
};
use chrono::Utc;
use closeclaw_common::{ChildSessionState, ToolExecState};

impl ConversationSession {
    /// Collect pending operations from the current session state.
    pub fn collect_pending_operations(&self) -> Vec<PendingOperation> {
        let mut ops = Vec::new();
        let now = Utc::now();
        self.collect_pending_tool_calls(&mut ops, now);
        self.collect_pending_children(&mut ops, now);
        self.collect_pending_outbound(&mut ops, now);
        ops
    }

    fn collect_pending_tool_calls(
        &self,
        ops: &mut Vec<PendingOperation>,
        now: chrono::DateTime<Utc>,
    ) {
        let tool_states = self.tool_states.read().expect("tool_states lock poisoned");
        for (tool_id, (state, detail)) in tool_states.iter() {
            if matches!(
                state,
                ToolExecState::RunningForeground
                    | ToolExecState::RunningBackground
                    | ToolExecState::Pending
            ) {
                let op_detail = detail.clone().unwrap_or_else(|| PoDetail::ToolCall {
                    tool_name: tool_id.clone(),
                    args_summary: String::new(),
                });
                ops.push(PendingOperation {
                    op_id: tool_id.clone(),
                    op_type: PendingOperationType::ToolCall,
                    status: PendingOperationStatus::Running,
                    detail: op_detail,
                    created_at: now,
                });
            }
        }
    }

    fn collect_pending_children(
        &self,
        ops: &mut Vec<PendingOperation>,
        now: chrono::DateTime<Utc>,
    ) {
        let child_states = self
            .child_states
            .read()
            .expect("child_states lock poisoned");
        for (child_id, (state, detail)) in child_states.iter() {
            if matches!(state, ChildSessionState::Running) {
                let op_detail = detail.clone().unwrap_or_else(|| PoDetail::SubSessionSpawn {
                    child_session_id: child_id.clone(),
                    agent_id: String::new(),
                    task_summary: String::new(),
                });
                ops.push(PendingOperation {
                    op_id: child_id.clone(),
                    op_type: PendingOperationType::SubSessionSpawn,
                    status: PendingOperationStatus::Running,
                    detail: op_detail,
                    created_at: now,
                });
            }
        }
    }

    /// Outbound message tracking is handled at the checkpoint layer
    /// via `record_outbound_pending_op` / `clear_outbound_pending_op`.
    /// ConversationSession has no independent outbound queue — the
    /// unified queue holds inbound user messages, not outbound agent
    /// deliveries. This collector is intentionally a no-op to avoid
    /// emitting spurious OutboundMessage ops from unsent queue entries.
    fn collect_pending_outbound(
        &self,
        _ops: &mut Vec<PendingOperation>,
        _now: chrono::DateTime<Utc>,
    ) {
        // no-op: outbound tracking lives in checkpoint.pending_operations
    }
}
