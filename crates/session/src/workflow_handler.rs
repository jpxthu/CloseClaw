//! Workflow tool result processing and engine state management.
//!
//! Intercepts workflow tool results (`workflow_start`, `workflow_verify`,
//! `workflow_jump`, `workflow_blocked`) from the LLM response, routes
//! them to the appropriate [`WorkflowEngine`] method, and manages
//! blocked-state notifications for owner intervention.

use std::collections::HashMap;

use closeclaw_workflow::definition::Workflow;
use closeclaw_workflow::engine::WorkflowEngine;
use closeclaw_workflow::run::{Phase, WorkflowRun};

use closeclaw_common::ContentBlock;

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
/// Manages the [`WorkflowEngine`] and the current [`WorkflowRun`] state.
/// Tool results from the LLM are parsed and routed to the appropriate
/// engine method. Blocked-state notifications are queued for the
/// gateway to deliver to the owner.
#[derive(Clone)]
pub struct WorkflowHandler {
    /// The current workflow run state.
    run: WorkflowRun,
    /// The workflow definition for the active run.
    definition: Workflow,
    /// Pending notification to send to the owner (blocked state only).
    pending_notification: Option<WorkflowNotification>,
}

impl WorkflowHandler {
    /// Create a new handler from a started workflow run and definition.
    pub fn new(run: WorkflowRun, definition: Workflow) -> Self {
        Self {
            run,
            definition,
            pending_notification: None,
        }
    }

    /// Returns a reference to the current workflow run.
    pub fn run(&self) -> &WorkflowRun {
        &self.run
    }

    /// Returns a mutable reference to the current workflow run.
    pub fn run_mut(&mut self) -> &mut WorkflowRun {
        &mut self.run
    }

    /// Returns a reference to the workflow definition.
    pub fn definition(&self) -> &Workflow {
        &self.definition
    }

    /// Take the pending notification (if any), clearing it.
    pub fn take_notification(&mut self) -> Option<WorkflowNotification> {
        self.pending_notification.take()
    }

    /// Returns `true` if the workflow is in a blocked state.
    pub fn is_blocked(&self) -> bool {
        self.run.phase == Phase::Blocked
    }

    /// Returns `true` if the workflow is complete.
    pub fn is_complete(&self) -> bool {
        self.run.phase == Phase::Complete
    }

    /// Returns `true` if the session idle condition should trigger
    /// a verify injection (phase == Executing).
    pub fn on_session_idle(&self) -> bool {
        WorkflowEngine::on_session_idle(&self.run)
    }

    /// Process a workflow tool result from the LLM response.
    ///
    /// Parses the `ContentBlock::ToolResult` content as JSON and routes
    /// the action to the appropriate engine method. Returns `true` if
    /// a workflow action was processed.
    pub fn process_tool_result(&mut self, content: &str) -> bool {
        let data: serde_json::Value = match serde_json::from_str(content) {
            Ok(v) => v,
            Err(_) => return false,
        };

        let action = match data.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => return false,
        };

        match action {
            "workflow_start" => self.handle_start_result(&data),
            "workflow_verify" => self.handle_verify_result(),
            "workflow_jump" => self.handle_jump_result(&data),
            "workflow_blocked" => self.handle_blocked_result(&data),
            _ => false,
        }
    }

    /// Process all workflow tool results from LLM content blocks.
    ///
    /// Scans `ContentBlock::ToolResult` blocks for workflow actions and
    /// processes them. Returns `true` if any workflow action was processed.
    pub fn process_content_blocks(&mut self, blocks: &[ContentBlock]) -> bool {
        let mut processed = false;
        for block in blocks {
            if let ContentBlock::ToolResult { content, .. } = block {
                if self.process_tool_result(content) {
                    processed = true;
                }
            }
        }
        processed
    }

    /// Handle a `workflow_start` tool result.
    ///
    /// Records the goal injection timestamp.
    fn handle_start_result(&mut self, _data: &serde_json::Value) -> bool {
        WorkflowEngine::on_goal_injected(&mut self.run);
        tracing::debug!(
            step = self.run.current_step,
            workflow = %self.run.definition_name,
            "workflow goal injected"
        );
        true
    }

    /// Handle a `workflow_verify` tool result.
    ///
    /// Calls `WorkflowEngine::handle_verify` to evaluate transitions.
    /// If the engine returns a blocked state, queues a notification.
    fn handle_verify_result(&mut self) -> bool {
        match WorkflowEngine::handle_verify(&mut self.run, &self.definition) {
            Ok(_action) => {
                tracing::debug!(
                    step = self.run.current_step,
                    phase = ?self.run.phase,
                    "verify processed"
                );
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, "verify handling failed");
                false
            }
        }
    }

    /// Handle a `workflow_jump` tool result.
    ///
    /// Evaluates answers against transitions and executes the matched action.
    fn handle_jump_result(&mut self, data: &serde_json::Value) -> bool {
        let answers = match data.get("answers") {
            Some(a) => a.as_object().cloned().unwrap_or_default(),
            None => return false,
        };
        let yaml_answers: HashMap<String, serde_yaml::Value> = answers
            .into_iter()
            .filter_map(|(k, v)| {
                let yaml_val: serde_yaml::Value = serde_yaml::from_str(&v.to_string()).ok()?;
                Some((k, yaml_val))
            })
            .collect();

        match WorkflowEngine::handle_jump(&mut self.run, &self.definition, &yaml_answers) {
            Ok(action) => {
                tracing::debug!(
                    action = ?action,
                    step = self.run.current_step,
                    "jump processed"
                );
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, "jump handling failed");
                false
            }
        }
    }

    /// Handle a `workflow_blocked` tool result.
    ///
    /// Calls `WorkflowEngine::handle_blocked` and queues a notification
    /// for the owner if blocking is allowed.
    fn handle_blocked_result(&mut self, data: &serde_json::Value) -> bool {
        let reason = data
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        // Check if blocking is allowed for the current step.
        let step = match self.definition.steps.get(self.run.current_step) {
            Some(s) => s,
            None => return false,
        };
        let allow_blocked = step.allow_blocked.unwrap_or(false);

        if let Err(e) =
            WorkflowEngine::handle_blocked(&mut self.run, &self.definition, allow_blocked)
        {
            tracing::warn!(error = %e, "blocked handling failed");
            return false;
        }

        // Queue notification for the owner.
        let step_name = step.name.clone();
        self.pending_notification = Some(WorkflowNotification {
            workflow_name: self.run.definition_name.clone(),
            current_step: self.run.current_step,
            reason: reason.to_string(),
            message: format!(
                "⚠️ Workflow「{}」在 Step {} ({}) 被阻塞\n原因：{}\n\n请回复「恢复」继续执行，或「终止」结束工作流。",
                self.run.definition_name,
                self.run.current_step,
                step_name,
                reason,
            ),
        });

        tracing::info!(
            workflow = %self.run.definition_name,
            step = self.run.current_step,
            reason = %reason,
            "workflow blocked, owner notification queued"
        );
        true
    }

    /// Notify the engine that the verify limit has been exceeded.
    ///
    /// Called by the gateway when `on_verify_injected` transitions the
    /// run to blocked state. Queues a notification for the owner.
    pub fn on_verify_limit_exceeded(&mut self, verify_retry_limit: usize) {
        WorkflowEngine::on_verify_injected(&mut self.run, verify_retry_limit);
        if self.run.phase == Phase::Blocked {
            self.queue_verify_limit_notification(verify_retry_limit);
        }
    }

    /// Handle an owner resolve response.
    ///
    /// Transitions the workflow from blocked to verifying, clears
    /// pending_verify, and removes old goal message.
    pub fn on_owner_resolve(&mut self) {
        WorkflowEngine::on_owner_resolve(&mut self.run);
        self.pending_notification = None;
        tracing::info!(
            workflow = %self.run.definition_name,
            "owner resolved blocked workflow"
        );
    }

    /// Handle an owner terminate response.
    ///
    /// Transitions the workflow to complete phase.
    pub fn on_owner_terminate(&mut self) {
        WorkflowEngine::on_owner_terminate(&mut self.run);
        self.pending_notification = None;
        tracing::info!(
            workflow = %self.run.definition_name,
            "owner terminated workflow"
        );
    }

    /// Record that a verify message has been injected.
    ///
    /// Delegates to `WorkflowEngine::on_verify_injected` which
    /// increments `pending_verify` and may transition to blocked.
    pub fn on_verify_injected(&mut self, verify_retry_limit: usize) {
        WorkflowEngine::on_verify_injected(&mut self.run, verify_retry_limit);
        if self.run.phase == Phase::Blocked && self.pending_notification.is_none() {
            self.queue_verify_limit_notification(verify_retry_limit);
        }
    }

    /// Queue a notification for verify-limit-exceeded blocking.
    fn queue_verify_limit_notification(&mut self, verify_retry_limit: usize) {
        let step_name = self
            .definition
            .steps
            .get(self.run.current_step)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        self.pending_notification = Some(WorkflowNotification {
            workflow_name: self.run.definition_name.clone(),
            current_step: self.run.current_step,
            reason: format!("验证重试次数超过上限 ({})", verify_retry_limit),
            message: format!(
                "⚠️ Workflow「{}」在 Step {} ({}) 验证重试次数超过上限\n\n请回复「恢复」继续执行，或「终止」结束工作流。",
                self.run.definition_name, self.run.current_step, step_name,
            ),
        });
    }

    /// Record that a goal message has been injected.
    pub fn on_goal_injected(&mut self) {
        WorkflowEngine::on_goal_injected(&mut self.run);
    }
}
