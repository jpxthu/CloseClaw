//! Workflow tool result processing and engine state management.
//!
//! Intercepts workflow tool results (`workflow_start`, `workflow_verify`,
//! `workflow_jump`, `workflow_blocked`) from the LLM response, routes
//! them through the injected [`WorkflowPort`] to the workflow engine,
//! and manages blocked-state notifications for owner intervention.
//!
//! All engine state is held in its serialized (`serde_json::Value`)
//! form; typed engine access is delegated to the port.

use std::sync::Arc;

use closeclaw_common::ContentBlock;

use crate::workflow_port::{
    JumpQuestionSpec, WorkflowGoalHint, WorkflowPhase, WorkflowPort, WorkflowRunInfo,
};

/// Result of processing a verify/jump tool action, indicating the
/// resulting workflow phase transition.
pub enum JumpResult {
    /// Phase transitioned to Jumping; caller should inject jump message.
    Jumped,
    /// Phase transitioned to Complete; caller should trigger exit cleanup.
    Completed,
    /// Phase did not transition to jumping or complete (e.g., blocked,
    /// error, or no transitions).
    NotJumped,
}

/// Pending notification to send to the owner when the workflow is blocked.
#[derive(Debug, Clone)]
pub struct WorkflowNotification {
    /// Workflow definition name.
    pub workflow_name: String,
    /// Current step index (0-based).
    pub current_step: usize,
    /// Reason for blocking.
    pub reason: String,
    /// Notification message text.
    pub message: String,
}

/// Workflow tool result processor and engine state holder.
///
/// Manages the serialized workflow-run state and the active definition.
/// Tool results from the LLM are parsed and routed to the engine via
/// the injected [`WorkflowPort`]. Blocked-state notifications are
/// queued for the gateway to deliver to the owner.
#[derive(Clone)]
pub struct WorkflowHandler {
    /// The current workflow run state (serialized form).
    run: serde_json::Value,
    /// The workflow definition for the active run (serialized form).
    definition: serde_json::Value,
    /// Engine port for all workflow-crate operations.
    port: Arc<dyn WorkflowPort>,
    /// Pending notification to send to the owner (blocked state only).
    pending_notification: Option<WorkflowNotification>,
}

impl WorkflowHandler {
    /// Create a new handler from serialized run state, a serialized
    /// definition, and the engine port.
    pub fn new(
        run: serde_json::Value,
        definition: serde_json::Value,
        port: Arc<dyn WorkflowPort>,
    ) -> Self {
        Self {
            run,
            definition,
            port,
            pending_notification: None,
        }
    }

    /// Returns a reference to the serialized workflow run state.
    pub fn run_state(&self) -> &serde_json::Value {
        &self.run
    }

    /// Returns a reference to the serialized workflow definition.
    pub fn definition_state(&self) -> &serde_json::Value {
        &self.definition
    }

    /// Returns the engine port this handler decodes through.
    ///
    /// Lets session-side run-state queries reuse the port when only a
    /// handler (not the session-level port) has been installed.
    pub fn port(&self) -> &Arc<dyn WorkflowPort> {
        &self.port
    }

    /// Take the pending notification (if any), clearing it.
    pub fn take_notification(&mut self) -> Option<WorkflowNotification> {
        self.pending_notification.take()
    }
}

// ── State queries (delegated to the port) ───────────────────────
impl WorkflowHandler {
    /// Decoded view of the current run state, if decodable.
    pub fn run_info(&self) -> Option<WorkflowRunInfo> {
        self.port.run_info(&self.run)
    }

    /// Returns the current phase, if the run state is decodable.
    pub fn phase(&self) -> Option<WorkflowPhase> {
        self.port.run_phase(&self.run)
    }

    /// Returns `true` if the workflow is in a blocked state.
    pub fn is_blocked(&self) -> bool {
        self.phase() == Some(WorkflowPhase::Blocked)
    }

    /// Returns `true` if the workflow is complete.
    pub fn is_complete(&self) -> bool {
        self.phase() == Some(WorkflowPhase::Complete)
    }

    /// Returns `true` if the session idle condition should trigger
    /// a verify injection (phase == Executing).
    pub fn on_session_idle(&self) -> bool {
        self.port.on_session_idle(&self.run)
    }

    /// Returns the verify retry limit declared by the definition.
    pub fn verify_retry_limit(&self) -> Option<usize> {
        self.port.definition_verify_retry_limit(&self.definition)
    }

    /// Returns the effective `allow_blocked` for the current step.
    pub fn current_step_effective_allow_blocked(&self) -> Option<bool> {
        let step = self.run_info()?.current_step;
        self.port
            .step_effective_allow_blocked(&self.definition, step)
    }

    /// Render the verify checklist message for the given step.
    pub fn verify_message_for_step(&self, step: usize, allow_blocked: bool) -> Option<String> {
        self.port
            .verify_message(&self.definition, step, allow_blocked)
    }

    /// Render the jump question message for the given step.
    pub fn jump_message_for_step(&self, step: usize) -> Option<String> {
        self.port.jump_message(&self.definition, step)
    }

    /// Render the goal message for the given step.
    pub fn goal_message_for_step(&self, step: usize, hint: WorkflowGoalHint) -> Option<String> {
        self.port.goal_message(&self.definition, step, hint)
    }
}

// ── Tool result processing ──────────────────────────────────────
impl WorkflowHandler {
    /// Process a workflow tool result from the LLM response.
    ///
    /// Parses the `ContentBlock::ToolResult` content as JSON and routes
    /// the action to the appropriate engine method. Returns `true` if
    /// a workflow action was processed.
    pub fn process_tool_result(&mut self, content: &str) -> (bool, JumpResult) {
        let data: serde_json::Value = match serde_json::from_str(content) {
            Ok(v) => v,
            Err(_) => return (false, JumpResult::NotJumped),
        };

        let action = match data.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => return (false, JumpResult::NotJumped),
        };

        match action {
            "workflow_start" => (self.handle_start_result(&data), JumpResult::NotJumped),
            "workflow_verify" => self.handle_verify_result(),
            "workflow_jump" => self.handle_jump_result(&data),
            "workflow_blocked" => (self.handle_blocked_result(&data), JumpResult::NotJumped),
            _ => (false, JumpResult::NotJumped),
        }
    }

    /// Process all workflow tool results from LLM content blocks.
    ///
    /// Scans `ContentBlock::ToolResult` blocks for workflow actions and
    /// processes them. Returns `(processed, jump_result)` where
    /// `processed` is `true` if any workflow action was processed, and
    /// `jump_result` indicates whether the workflow entered jumping phase.
    pub fn process_content_blocks(&mut self, blocks: &[ContentBlock]) -> (bool, JumpResult) {
        let mut processed = false;
        let mut jump_result = JumpResult::NotJumped;
        for block in blocks {
            if let ContentBlock::ToolResult { content, .. } = block {
                let (action_processed, result) = self.process_tool_result(content);
                if action_processed {
                    processed = true;
                }
                if matches!(result, JumpResult::Jumped) {
                    jump_result = JumpResult::Jumped;
                }
                if matches!(result, JumpResult::Completed) {
                    jump_result = JumpResult::Completed;
                }
            }
        }
        (processed, jump_result)
    }

    /// Handle a `workflow_start` tool result.
    ///
    /// Records the goal injection timestamp.
    fn handle_start_result(&mut self, _data: &serde_json::Value) -> bool {
        self.run = self
            .port
            .on_goal_injected(std::mem::replace(&mut self.run, serde_json::Value::Null));
        let (step, name) = match self.run_info() {
            Some(info) => (info.current_step, info.definition_name),
            None => (0, String::new()),
        };
        tracing::debug!(step, workflow = %name, "workflow goal injected");
        true
    }

    /// Handle a `workflow_verify` tool result.
    ///
    /// Delegates to the port's `handle_verify` to evaluate transitions.
    /// Returns [`JumpResult::Jumped`] if the workflow entered jumping phase,
    /// [`JumpResult::NotJumped`] otherwise.
    fn handle_verify_result(&mut self) -> (bool, JumpResult) {
        // Fallible engine call: on Err the engine did not advance, so
        // `self.run` is left untouched (no transient Null swap needed).
        match self.port.handle_verify(self.run.clone(), &self.definition) {
            Ok((run, phase)) => {
                self.run = run;
                let jump_result = if phase == WorkflowPhase::Jumping {
                    JumpResult::Jumped
                } else {
                    JumpResult::NotJumped
                };
                let step = self.run_info().map(|i| i.current_step).unwrap_or(0);
                tracing::debug!(step, phase = ?phase, "verify processed");
                (true, jump_result)
            }
            Err(e) => {
                tracing::warn!(error = %e, "verify handling failed");
                (false, JumpResult::NotJumped)
            }
        }
    }
}

// ── Jump handling ───────────────────────────────────────────────
impl WorkflowHandler {
    /// Handle a `workflow_jump` tool result.
    ///
    /// Evaluates answers against transitions and executes the matched action.
    /// For enum questions, maps single-letter answers (A, B, C, …) to the
    /// corresponding internal option value before passing to the engine.
    ///
    /// Returns [`JumpResult::Completed`] when the workflow transitions
    /// to the `Complete` phase.
    fn handle_jump_result(&mut self, data: &serde_json::Value) -> (bool, JumpResult) {
        let mut answers = match data.get("answers") {
            Some(a) => a.as_object().cloned().unwrap_or_default(),
            None => return (false, JumpResult::NotJumped),
        };

        // Map enum letter answers to internal option values.
        self.map_enum_letter_answers(&mut answers);

        // Fallible engine call: on Err the engine did not advance, so
        // `self.run` is left untouched (no transient Null swap needed).
        match self
            .port
            .handle_jump(self.run.clone(), &self.definition, &answers)
        {
            Ok((run, phase)) => {
                self.run = run;
                let jump_result = if phase == WorkflowPhase::Complete {
                    JumpResult::Completed
                } else {
                    JumpResult::NotJumped
                };
                let step = self.run_info().map(|i| i.current_step).unwrap_or(0);
                tracing::debug!(step, phase = ?phase, "jump processed");
                (true, jump_result)
            }
            Err(e) => {
                tracing::warn!(error = %e, "jump handling failed");
                (false, JumpResult::NotJumped)
            }
        }
    }

    /// Map single-letter enum answers to their internal option values.
    ///
    /// When an agent answers an enum question with a letter like "A",
    /// this maps it to the corresponding `options[index]` value so that
    /// transition evaluation can match against `expected_value`.
    pub(crate) fn map_enum_letter_answers(
        &self,
        answers: &mut serde_json::Map<String, serde_json::Value>,
    ) {
        let step = match self.run_info() {
            Some(info) => info.current_step,
            None => return,
        };
        for q in self.port.step_jump_questions(&self.definition, step) {
            let JumpQuestionSpec {
                id,
                question_type,
                options,
            } = q;
            if question_type != "enum" || options.is_empty() {
                continue;
            }
            let letter = match answers.get(&id).and_then(|v| v.as_str()) {
                Some(s) => s,
                None => continue,
            };
            let idx = match Self::try_map_enum_answer(letter) {
                Some(i) => i,
                None => continue,
            };
            if idx < options.len() {
                answers.insert(id, serde_json::Value::String(options[idx].clone()));
            }
        }
    }

    /// Try to parse a single-letter enum answer into an option index.
    ///
    /// Returns `Some(index)` for a single uppercase ASCII letter (A → 0,
    /// B → 1, …), or `None` if the letter is lowercase, multi-char,
    /// or non-ASCII.
    fn try_map_enum_answer(letter: &str) -> Option<usize> {
        if letter.len() != 1 {
            return None;
        }
        let b = letter.as_bytes()[0];
        if !b.is_ascii_uppercase() {
            return None;
        }
        Some((b - b'A') as usize)
    }
}

// ── Engine callbacks and notifications ──────────────────────────
impl WorkflowHandler {
    /// Handle a `workflow_blocked` tool result.
    ///
    /// Routes through the port's `handle_blocked` and queues a
    /// notification for the owner if blocking is allowed.
    fn handle_blocked_result(&mut self, data: &serde_json::Value) -> bool {
        let reason = data
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        // Check if blocking is allowed for the current step.
        let current_step = match self.run_info() {
            Some(info) => info.current_step,
            None => return false,
        };
        let allow_blocked = match self
            .port
            .step_effective_allow_blocked(&self.definition, current_step)
        {
            Some(a) => a,
            None => return false,
        };

        // Fallible engine call: on Err the engine did not advance, so
        // `self.run` is left untouched (no transient Null swap needed).
        match self
            .port
            .handle_blocked(self.run.clone(), &self.definition, allow_blocked, reason)
        {
            Ok(run) => {
                self.run = run;
                let workflow_name = self
                    .run_info()
                    .map(|i| i.definition_name)
                    .unwrap_or_default();
                let step_name = self
                    .port
                    .step_name(&self.definition, current_step)
                    .unwrap_or_default();
                self.pending_notification = Some(WorkflowNotification {
                    workflow_name: workflow_name.clone(),
                    current_step,
                    reason: reason.to_string(),
                    message: format!(
                        "⚠️ Workflow「{}」在 Step {} ({}) 被阻塞\n原因：{}\n\n请回复「恢复」继续执行，或「终止」结束工作流。",
                        workflow_name, current_step, step_name, reason,
                    ),
                });
                tracing::info!(
                    workflow = %workflow_name,
                    step = current_step,
                    reason = %reason,
                    "workflow blocked, owner notification queued"
                );
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, "blocked handling failed");
                false
            }
        }
    }

    /// Notify the engine that the verify limit has been exceeded.
    ///
    /// Called by the gateway when `on_verify_injected` transitions the
    /// run to blocked state. Queues a notification for the owner.
    pub fn on_verify_limit_exceeded(&mut self, verify_retry_limit: usize) {
        self.run = self.port.on_verify_injected(
            std::mem::replace(&mut self.run, serde_json::Value::Null),
            verify_retry_limit,
        );
        if self.is_blocked() {
            self.queue_verify_limit_notification(verify_retry_limit);
        }
    }

    /// Handle an owner resolve response.
    ///
    /// Transitions the workflow from blocked to verifying, clears
    /// pending_verify, and removes old goal message.
    pub fn on_owner_resolve(&mut self) {
        self.run = self
            .port
            .on_owner_resolve(std::mem::replace(&mut self.run, serde_json::Value::Null));
        self.pending_notification = None;
        let workflow = self
            .run_info()
            .map(|i| i.definition_name)
            .unwrap_or_default();
        tracing::info!(workflow = %workflow, "owner resolved blocked workflow");
    }

    /// Handle an owner terminate response.
    ///
    /// Transitions the workflow to complete phase.
    pub fn on_owner_terminate(&mut self) {
        self.run = self
            .port
            .on_owner_terminate(std::mem::replace(&mut self.run, serde_json::Value::Null));
        self.pending_notification = None;
        let workflow = self
            .run_info()
            .map(|i| i.definition_name)
            .unwrap_or_default();
        tracing::info!(workflow = %workflow, "owner terminated workflow");
    }

    /// Record that a verify message has been injected.
    ///
    /// Delegates to the port's `on_verify_injected` which
    /// increments `pending_verify` and may transition to blocked.
    pub fn on_verify_injected(&mut self, verify_retry_limit: usize) {
        self.run = self.port.on_verify_injected(
            std::mem::replace(&mut self.run, serde_json::Value::Null),
            verify_retry_limit,
        );
        if self.is_blocked() && self.pending_notification.is_none() {
            self.queue_verify_limit_notification(verify_retry_limit);
        }
    }

    /// Queue a notification for verify-limit-exceeded blocking.
    fn queue_verify_limit_notification(&mut self, verify_retry_limit: usize) {
        let (workflow_name, current_step) = match self.run_info() {
            Some(info) => (info.definition_name, info.current_step),
            None => (String::new(), 0),
        };
        let step_name = self
            .port
            .step_name(&self.definition, current_step)
            .unwrap_or_default();
        self.pending_notification = Some(WorkflowNotification {
            workflow_name: workflow_name.clone(),
            current_step,
            reason: format!("验证重试次数超过上限 ({})", verify_retry_limit),
            message: format!(
                "⚠️ Workflow「{}」在 Step {} ({}) 验证重试次数超过上限\n\n请回复「恢复」继续执行，或「终止」结束工作流。",
                workflow_name, current_step, step_name,
            ),
        });
    }

    /// Record that a goal message has been injected.
    pub fn on_goal_injected(&mut self) {
        self.run = self
            .port
            .on_goal_injected(std::mem::replace(&mut self.run, serde_json::Value::Null));
    }
}
