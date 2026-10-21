//! Workflow-related methods for `ConversationSession`.

use closeclaw_common::processor::ContentBlock;

use std::sync::Arc;

use crate::workflow_handler::JumpResult;
use crate::workflow_port::{WorkflowPhase, WorkflowRunDecodeError};

use super::ConversationSession;

// ── Run / handler accessors ─────────────────────────────────────
impl ConversationSession {
    /// Returns the raw `Value`-erased workflow run state (checkpoint form).
    pub fn workflow_run_value(&self) -> Option<serde_json::Value> {
        self.workflow_run.clone()
    }

    /// Overwrites the raw `Value`-erased workflow run state (checkpoint form).
    pub fn set_workflow_run_value(&mut self, state: Option<serde_json::Value>) {
        self.workflow_run = state;
    }

    /// Returns the active workflow run, if any (test-only typed view).
    #[cfg(test)]
    pub fn workflow_run(&self) -> Option<closeclaw_workflow::run::WorkflowRun> {
        let state = self.workflow_run.as_ref()?;
        match serde_json::from_value(state.clone()) {
            Ok(run) => Some(run),
            Err(e) => {
                tracing::warn!(error = %e, "failed to decode workflow_run state");
                None
            }
        }
    }

    /// Sets the active workflow run state (test-only typed form).
    #[cfg(test)]
    pub fn set_workflow_run(&mut self, run: Option<closeclaw_workflow::run::WorkflowRun>) {
        self.workflow_run = run.and_then(|r| serde_json::to_value(r).ok());
    }

    /// Inject the [`WorkflowPort`](crate::workflow_port::WorkflowPort) for workflow
    /// engine operations.
    ///
    /// Called by the composition root (daemon) after session creation;
    /// child sessions inherit it via [`SpawnCreationContext`](crate::spawn::SpawnCreationContext).
    pub fn set_workflow_port(&mut self, port: Arc<dyn crate::workflow_port::WorkflowPort>) {
        self.workflow_port = Some(port);
    }

    /// Returns the injected [`WorkflowPort`](crate::workflow_port::WorkflowPort), if any.
    pub fn workflow_port(&self) -> Option<Arc<dyn crate::workflow_port::WorkflowPort>> {
        self.workflow_port.clone()
    }

    /// Returns the phase of the active workflow run as a string.
    ///
    /// The stored `Value` is decoded through the injected
    /// [`WorkflowPort`](crate::workflow_port::WorkflowPort),
    /// so no workflow type surfaces outside the session crate. Returns
    /// `Ok(None)` when no run is stored, the run has completed, or no port
    /// is injected, and `Err` when the stored value cannot be decoded —
    /// the caller owns the failure logging (target semantics stay with
    /// the caller's module). Callers use this to enforce the
    /// one-workflow-per-session constraint.
    pub fn active_workflow_run_phase(&self) -> Result<Option<String>, WorkflowRunDecodeError> {
        let Some(state) = self.workflow_run.as_ref() else {
            return Ok(None);
        };
        let Some(port) = self.workflow_port.as_ref() else {
            return Ok(None);
        };
        let Some(phase) = port.run_phase(state) else {
            return Err(WorkflowRunDecodeError);
        };
        if phase == WorkflowPhase::Complete {
            return Ok(None);
        }
        Ok(Some(format!("{:?}", phase)))
    }

    /// Returns `true` when a workflow context append is already present in
    /// the merged system append list.
    pub fn has_workflow_context(&self) -> bool {
        crate::workflow_recovery::has_workflow_context_marker(&self.system_appends())
    }

    pub fn workflow_handler(&self) -> Option<&crate::workflow_handler::WorkflowHandler> {
        self.workflow_handler.as_ref()
    }

    pub fn workflow_handler_mut(
        &mut self,
    ) -> Option<&mut crate::workflow_handler::WorkflowHandler> {
        self.workflow_handler.as_mut()
    }

    pub fn set_workflow_handler(
        &mut self,
        handler: Option<crate::workflow_handler::WorkflowHandler>,
    ) {
        self.workflow_handler = handler;
    }
}

// ── Tool result processing ──────────────────────────────────────
impl ConversationSession {
    /// Process workflow tool results from LLM content blocks.
    ///
    /// Returns `true` if any action was processed.
    /// If the workflow transitions to jumping phase, removes completed
    /// verify messages and injects a jump message into the transcript.
    pub fn process_workflow_tool_results(&mut self, blocks: &[ContentBlock]) -> bool {
        self.ensure_workflow_handler();

        // ── Step 1.4: Reject workflow_start when an active workflow exists ──
        if self.has_active_workflow() {
            let has_start = blocks.iter().any(Self::is_workflow_start_block);
            if has_start {
                let phase = self
                    .workflow_run_state_phase()
                    .map(|p| format!("{p:?}"))
                    .unwrap_or_default();
                self.inject_workflow_message(&format!(
                    "⚠️ 无法启动新工作流：当前 Session 已有活跃的工作流运行（阶段：{phase}）。\n请等待当前工作流完成后再启动新工作流。"
                ));
                tracing::warn!(
                    phase = %phase,
                    "rejected workflow_start: active workflow exists"
                );
                return false;
            }
        }

        if let Some(ref mut handler) = self.workflow_handler {
            let was_jumping = handler.phase() == Some(WorkflowPhase::Jumping);
            let was_blocked_before = handler.phase() == Some(WorkflowPhase::Blocked);
            let (processed, jump_result) = handler.process_content_blocks(blocks);
            if processed {
                self.workflow_run = Some(handler.run_state().clone());
            }
            let jump_msg = if matches!(jump_result, JumpResult::Jumped) {
                let current_step = handler.run_info().map(|i| i.current_step).unwrap_or(0);
                handler.jump_message_for_step(current_step)
            } else {
                None
            };
            // handler borrow ends here; safe to call self methods
            if let Some(msg) = jump_msg {
                // Verify completed → remove verify messages + tool exchange.
                self.remove_workflow_verify_messages();
                self.remove_workflow_tool_exchange(&["workflow_verify", "workflow_blocked"]);
                self.inject_workflow_message(&msg);
                tracing::debug!("jump message injected into transcript");
            } else if was_jumping {
                // Jump completed → remove jump messages + tool exchange.
                self.remove_workflow_jump_messages();
                self.remove_workflow_tool_exchange(&["workflow_jump"]);
                // Post-jump phase dispatch: inject goal or trigger cleanup.
                let info = self.workflow_handler.as_ref().and_then(|h| h.run_info());
                let (current_phase, current_step, hint) = match info {
                    Some(i) => (Some(i.phase), i.current_step, i.pending_goal_hint),
                    None => (None, 0, Default::default()),
                };
                self.dispatch_post_jump_phase(current_phase, current_step, hint);
                tracing::debug!("jump messages cleaned up after phase transition");
            } else if processed && !was_blocked_before {
                self.erase_blocked_phase_messages();
            }
            processed
        } else {
            false
        }
    }

    pub fn take_workflow_notification(
        &mut self,
    ) -> Option<crate::workflow_handler::WorkflowNotification> {
        self.workflow_handler
            .as_mut()
            .and_then(|h| h.take_notification())
    }
}

// ── Workflow state queries ──────────────────────────────────────
impl ConversationSession {
    /// Erase verify injection + tool_call + tool_result when blocked phase
    /// is entered. Mirrors the erasure done on verify→jump transitions.
    fn erase_blocked_phase_messages(&mut self) {
        let is_blocked = self
            .workflow_handler
            .as_ref()
            .is_some_and(|h| h.is_blocked());
        if is_blocked {
            self.remove_workflow_verify_messages();
            self.remove_workflow_tool_exchange(&["workflow_verify", "workflow_blocked"]);
            tracing::debug!("blocked phase: verify messages erased");
        }
    }

    pub fn is_workflow_blocked(&self) -> bool {
        self.workflow_handler
            .as_ref()
            .is_some_and(|h| h.is_blocked())
    }

    /// Returns the phase of the stored run state, if any and decodable.
    fn workflow_run_state_phase(&self) -> Option<WorkflowPhase> {
        let state = self.workflow_run.as_ref()?;
        let port = self.workflow_port.as_ref()?;
        port.run_phase(state)
    }

    /// Returns `true` if there is an active (non-Complete) workflow run.
    fn has_active_workflow(&self) -> bool {
        matches!(self.workflow_run_state_phase(), Some(p) if p != WorkflowPhase::Complete)
    }

    /// Returns `true` if a content block is a `workflow_start` tool result.
    fn is_workflow_start_block(block: &ContentBlock) -> bool {
        match block {
            ContentBlock::ToolResult { content, .. } => {
                serde_json::from_str::<serde_json::Value>(content)
                    .ok()
                    .and_then(|v| {
                        v.get("action")
                            .and_then(|a| a.as_str())
                            .map(|a| a == "workflow_start")
                    })
                    .unwrap_or(false)
            }
            _ => false,
        }
    }
}
