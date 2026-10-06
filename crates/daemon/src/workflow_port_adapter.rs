//! Production [`WorkflowPort`] implementation for the daemon composition
//! root.
//!
//! [`EngineWorkflowPort`] is a stateless adapter over the
//! `closeclaw_workflow` engine, definition loader, and message builders:
//! all run state crosses the port as `serde_json::Value` (decoded to
//! `WorkflowRun` internally, executed, then re-encoded). It caches no
//! session-level state, so a single instance can be shared by the main
//! session creation path, child session creation, and recovery.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use closeclaw_session::workflow_port::{
    JumpQuestionSpec, WorkflowGoalHint, WorkflowPhase, WorkflowPort, WorkflowRunInfo,
};

use closeclaw_workflow::context_append::build_workflow_context_append;
use closeclaw_workflow::definition::{
    build_goal_message, build_jump_message, build_verify_message, Workflow,
};
use closeclaw_workflow::definition_loader::WorkflowDefinitionLoader;
use closeclaw_workflow::engine::WorkflowEngine;
use closeclaw_workflow::run::{GoalHint, Phase, WorkflowRun};

/// Stateless production adapter wrapping the real workflow engine.
pub struct EngineWorkflowPort;

/// Build the production workflow port as `Arc<dyn WorkflowPort>`.
pub fn engine_workflow_port() -> Arc<dyn WorkflowPort> {
    Arc::new(EngineWorkflowPort)
}

fn phase_to_mirror(phase: &Phase) -> WorkflowPhase {
    match phase {
        Phase::Executing => WorkflowPhase::Executing,
        Phase::Verifying => WorkflowPhase::Verifying,
        Phase::Jumping => WorkflowPhase::Jumping,
        Phase::Blocked => WorkflowPhase::Blocked,
        Phase::Complete => WorkflowPhase::Complete,
    }
}

fn hint_to_mirror(hint: &GoalHint) -> WorkflowGoalHint {
    match hint {
        GoalHint::Normal => WorkflowGoalHint::Normal,
        GoalHint::Reexecute => WorkflowGoalHint::Reexecute,
    }
}

fn hint_from_mirror(hint: WorkflowGoalHint) -> GoalHint {
    match hint {
        WorkflowGoalHint::Normal => GoalHint::Normal,
        WorkflowGoalHint::Reexecute => GoalHint::Reexecute,
    }
}

fn decode_run(run: &serde_json::Value) -> Option<WorkflowRun> {
    serde_json::from_value(run.clone()).ok()
}

fn decode_definition(definition: &serde_json::Value) -> Option<Workflow> {
    serde_json::from_value(definition.clone()).ok()
}

fn encode_run(run: &WorkflowRun) -> serde_json::Value {
    serde_json::to_value(run).unwrap_or(serde_json::Value::Null)
}

/// Convert a JSON answer map into the engine's YAML answer map,
/// mirroring the historical conversion (`to_string` → YAML parse).
fn json_answers_to_yaml(
    answers: &serde_json::Map<String, serde_json::Value>,
) -> HashMap<String, serde_yaml::Value> {
    answers
        .iter()
        .filter_map(|(k, v)| {
            let yaml_val: serde_yaml::Value = serde_yaml::from_str(&v.to_string()).ok()?;
            Some((k.clone(), yaml_val))
        })
        .collect()
}

impl WorkflowPort for EngineWorkflowPort {
    fn run_phase(&self, run: &serde_json::Value) -> Option<WorkflowPhase> {
        decode_run(run).map(|r| phase_to_mirror(&r.phase))
    }

    fn run_info(&self, run: &serde_json::Value) -> Option<WorkflowRunInfo> {
        let r = decode_run(run)?;
        Some(WorkflowRunInfo {
            workflow_id: r.workflow_id,
            definition_name: r.definition_name,
            definition_version: r.definition_version,
            current_step: r.current_step,
            phase: phase_to_mirror(&r.phase),
            pending_goal_hint: hint_to_mirror(&r.pending_goal_hint),
            paused_reason: r.paused_reason,
            last_history_step_name: r.step_history.last().map(|e| e.step_name.clone()),
        })
    }

    fn on_goal_injected(&self, run: serde_json::Value) -> serde_json::Value {
        match decode_run(&run) {
            Some(mut r) => {
                WorkflowEngine::on_goal_injected(&mut r);
                encode_run(&r)
            }
            None => run,
        }
    }

    fn on_verify_injected(
        &self,
        run: serde_json::Value,
        verify_retry_limit: usize,
    ) -> serde_json::Value {
        match decode_run(&run) {
            Some(mut r) => {
                WorkflowEngine::on_verify_injected(&mut r, verify_retry_limit);
                encode_run(&r)
            }
            None => run,
        }
    }

    fn on_owner_resolve(&self, run: serde_json::Value) -> serde_json::Value {
        match decode_run(&run) {
            Some(mut r) => {
                WorkflowEngine::on_owner_resolve(&mut r);
                encode_run(&r)
            }
            None => run,
        }
    }

    fn on_owner_terminate(&self, run: serde_json::Value) -> serde_json::Value {
        match decode_run(&run) {
            Some(mut r) => {
                WorkflowEngine::on_owner_terminate(&mut r);
                encode_run(&r)
            }
            None => run,
        }
    }

    fn mark_blocked(&self, run: serde_json::Value, reason: &str) -> serde_json::Value {
        match decode_run(&run) {
            Some(mut r) => {
                r.phase = Phase::Blocked;
                r.paused_reason = reason.to_string();
                encode_run(&r)
            }
            None => run,
        }
    }

    fn on_session_idle(&self, run: &serde_json::Value) -> bool {
        decode_run(run).is_some_and(|r| WorkflowEngine::on_session_idle(&r))
    }

    fn handle_verify(
        &self,
        run: serde_json::Value,
        definition: &serde_json::Value,
    ) -> Result<(serde_json::Value, WorkflowPhase), String> {
        let mut r = decode_run(&run).ok_or_else(|| "undecodable run state".to_string())?;
        let def =
            decode_definition(definition).ok_or_else(|| "undecodable definition".to_string())?;
        WorkflowEngine::handle_verify(&mut r, &def).map_err(|e| e.to_string())?;
        let phase = phase_to_mirror(&r.phase);
        Ok((encode_run(&r), phase))
    }

    fn handle_jump(
        &self,
        run: serde_json::Value,
        definition: &serde_json::Value,
        answers: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(serde_json::Value, WorkflowPhase), String> {
        let mut r = decode_run(&run).ok_or_else(|| "undecodable run state".to_string())?;
        let def =
            decode_definition(definition).ok_or_else(|| "undecodable definition".to_string())?;
        let yaml_answers = json_answers_to_yaml(answers);
        WorkflowEngine::handle_jump(&mut r, &def, &yaml_answers).map_err(|e| e.to_string())?;
        let phase = phase_to_mirror(&r.phase);
        Ok((encode_run(&r), phase))
    }

    fn handle_blocked(
        &self,
        run: serde_json::Value,
        definition: &serde_json::Value,
        allow_blocked: bool,
        reason: &str,
    ) -> Result<serde_json::Value, String> {
        let mut r = decode_run(&run).ok_or_else(|| "undecodable run state".to_string())?;
        let def =
            decode_definition(definition).ok_or_else(|| "undecodable definition".to_string())?;
        WorkflowEngine::handle_blocked(&mut r, &def, allow_blocked, reason)
            .map_err(|e| e.to_string())?;
        Ok(encode_run(&r))
    }

    fn load_definition(
        &self,
        name: &str,
        agent_workspace: Option<&Path>,
        global_workflows: Option<&Path>,
    ) -> Result<serde_json::Value, String> {
        WorkflowDefinitionLoader::load(name, agent_workspace, global_workflows)
            .map(|def| serde_json::to_value(&def).unwrap_or(serde_json::Value::Null))
            .map_err(|e| e.to_string())
    }

    fn goal_message(
        &self,
        definition: &serde_json::Value,
        step: usize,
        hint: WorkflowGoalHint,
    ) -> Option<String> {
        decode_definition(definition).and_then(|def| {
            def.steps
                .get(step)
                .map(|s| build_goal_message(s, hint_from_mirror(hint)))
        })
    }

    fn jump_message(&self, definition: &serde_json::Value, step: usize) -> Option<String> {
        decode_definition(definition).and_then(|def| def.steps.get(step).map(build_jump_message))
    }

    fn verify_message(
        &self,
        definition: &serde_json::Value,
        step: usize,
        allow_blocked: bool,
    ) -> Option<String> {
        decode_definition(definition).and_then(|def| {
            def.steps
                .get(step)
                .map(|s| build_verify_message(s, allow_blocked))
        })
    }

    fn build_context_append(&self, definition: &serde_json::Value) -> String {
        match decode_definition(definition) {
            Some(def) => build_workflow_context_append(&def),
            None => String::new(),
        }
    }

    fn recovery_jump_message(
        &self,
        run: &serde_json::Value,
        definition: &serde_json::Value,
    ) -> Option<String> {
        let r = decode_run(run)?;
        let def = decode_definition(definition)?;
        WorkflowEngine::build_recovery_jump_message(&r, &def)
    }

    fn definition_verify_retry_limit(&self, definition: &serde_json::Value) -> Option<usize> {
        decode_definition(definition).map(|def| def.verify_retry_limit)
    }

    fn definition_version(&self, definition: &serde_json::Value) -> Option<String> {
        decode_definition(definition).and_then(|def| def.version)
    }

    fn definition_step_count(&self, definition: &serde_json::Value) -> usize {
        decode_definition(definition).map_or(0, |def| def.steps.len())
    }

    fn step_effective_allow_blocked(
        &self,
        definition: &serde_json::Value,
        step: usize,
    ) -> Option<bool> {
        decode_definition(definition).and_then(|def| {
            def.steps
                .get(step)
                .map(|s| s.allow_blocked.unwrap_or(def.allow_blocked))
        })
    }

    fn step_name(&self, definition: &serde_json::Value, step: usize) -> Option<String> {
        decode_definition(definition).and_then(|def| def.steps.get(step).map(|s| s.name.clone()))
    }

    fn step_jump_questions(
        &self,
        definition: &serde_json::Value,
        step: usize,
    ) -> Vec<JumpQuestionSpec> {
        decode_definition(definition)
            .and_then(|def| def.steps.get(step).cloned())
            .map(|s| {
                s.jump
                    .iter()
                    .map(|q| JumpQuestionSpec {
                        id: q.id.clone(),
                        question_type: q.question_type.clone(),
                        options: q.options.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_workflow::definition::{JumpQuestion, Step, Transition};
    use closeclaw_workflow::run::PendingVerify;

    use serde_json::json;

    /// Two-step definition: default `goto` transition advances from
    /// step 0 to step 1, whose default `complete` transition ends the
    /// workflow.
    fn definition_value() -> serde_json::Value {
        workflow_value(vec![
            Step {
                id: 0,
                name: "step-one".into(),
                allow_blocked: None,
                goal: "do the thing".into(),
                verify: vec!["check A".into()],
                jump: vec![],
                transitions: vec![Transition {
                    when: None,
                    action: "goto".into(),
                    target_step: Some(1),
                }],
            },
            Step {
                id: 1,
                name: "step-two".into(),
                allow_blocked: None,
                goal: "finish up".into(),
                verify: vec!["final check".into()],
                jump: vec![],
                transitions: vec![Transition {
                    when: None,
                    action: "complete".into(),
                    target_step: None,
                }],
            },
        ])
    }

    /// Single-step definition with a boolean jump question gated on a
    /// `when` transition.
    fn jump_definition_value() -> serde_json::Value {
        workflow_value(vec![Step {
            id: 0,
            name: "ask-first".into(),
            allow_blocked: None,
            goal: "ask then act".into(),
            verify: vec![],
            jump: vec![JumpQuestion {
                id: "q1".into(),
                prompt: "continue?".into(),
                question_type: "boolean".into(),
                options: vec![],
                option_labels: vec![],
            }],
            transitions: vec![Transition {
                when: Some(serde_yaml::from_str("q1: true").unwrap()),
                action: "complete".into(),
                target_step: None,
            }],
        }])
    }

    fn workflow_value(steps: Vec<Step>) -> serde_json::Value {
        let def = Workflow {
            id: "wf-test".into(),
            name: "test-workflow".into(),
            description: "test".into(),
            version: Some("1.0".into()),
            allow_blocked: false,
            verify_retry_limit: 3,
            step_data_schema: serde_yaml::Value::Null,
            steps,
        };
        serde_json::to_value(&def).unwrap()
    }

    /// Fresh run for step 0 in the given phase.
    fn run_value_in_phase(phase: Phase) -> serde_json::Value {
        let run = WorkflowRun {
            workflow_id: "run-1".into(),
            definition_name: "test-workflow".into(),
            definition_version: "1.0".into(),
            current_step: 0,
            phase,
            current_step_entered_at: String::new(),
            step_history: Vec::new(),
            step_data: serde_yaml::Value::Null,
            pending_goal_hint: GoalHint::Normal,
            pending_verify: PendingVerify::default(),
            paused_reason: String::new(),
        };
        serde_json::to_value(&run).unwrap()
    }

    /// Fresh run in the `Executing` phase for step 0.
    fn run_value() -> serde_json::Value {
        run_value_in_phase(Phase::Executing)
    }

    #[test]
    fn run_phase_and_run_info_decode_serialized_state() {
        let port = EngineWorkflowPort;
        let run = run_value();
        assert_eq!(port.run_phase(&run), Some(WorkflowPhase::Executing));
        let info = port.run_info(&run).expect("run info decodable");
        assert_eq!(info.workflow_id, "run-1");
        assert_eq!(info.definition_name, "test-workflow");
        assert_eq!(info.definition_version, "1.0");
        assert_eq!(info.current_step, 0);
        assert_eq!(info.phase, WorkflowPhase::Executing);
        assert_eq!(info.pending_goal_hint, WorkflowGoalHint::Normal);
        assert_eq!(info.paused_reason, "");
        assert_eq!(info.last_history_step_name, None);
    }

    #[test]
    fn undecodable_state_is_reported_or_passed_through() {
        let port = EngineWorkflowPort;
        let garbage = json!({ "not": "a workflow run" });
        assert_eq!(port.run_phase(&garbage), None);
        assert_eq!(port.run_info(&garbage), None);
        // State-changing ops pass undecodable input through unchanged.
        assert_eq!(port.on_goal_injected(garbage.clone()), garbage);
        assert_eq!(port.mark_blocked(garbage.clone(), "r"), garbage);
        assert!(!port.on_session_idle(&garbage));
        // Engine ops surface undecodable inputs as errors.
        assert!(port
            .handle_verify(garbage.clone(), &definition_value())
            .is_err());
        assert!(port.handle_verify(run_value(), &garbage).is_err());
    }

    #[test]
    fn verify_flow_advances_state_machine_through_value_roundtrip() {
        let port = EngineWorkflowPort;
        let def = definition_value();
        // First verify injection: checklist goes out, phase → Verifying.
        let run = port.on_verify_injected(run_value(), 3);
        assert_eq!(port.run_phase(&run), Some(WorkflowPhase::Verifying));
        assert!(port.on_session_idle(&run));
        // Agent responds to the verify → goto transition advances to
        // step 1, appending step-one to the history.
        let (run, phase) = port.handle_verify(run, &def).expect("verify accepted");
        assert_eq!(phase, WorkflowPhase::Executing);
        let info = port.run_info(&run).unwrap();
        assert_eq!(info.current_step, 1);
        assert_eq!(info.last_history_step_name, Some("step-one".into()));
        // Second verify cycle on the final step → workflow completes.
        let run = port.on_verify_injected(run, 3);
        assert_eq!(port.run_phase(&run), Some(WorkflowPhase::Verifying));
        let (run, phase) = port
            .handle_verify(run, &def)
            .expect("final verify accepted");
        assert_eq!(phase, WorkflowPhase::Complete);
        assert_eq!(port.run_phase(&run), Some(WorkflowPhase::Complete));
        let info = port.run_info(&run).unwrap();
        // The engine's `complete` transition ends the workflow without
        // appending history (only goto does), so the last entry is
        // still step-one.
        assert_eq!(info.last_history_step_name, Some("step-one".into()));
        // Retry limit reached at injection time → Blocked.
        let exhausted = port.on_verify_injected(run_value(), 0);
        assert_eq!(port.run_phase(&exhausted), Some(WorkflowPhase::Blocked));
    }

    #[test]
    fn jump_flow_maps_json_answers_and_executes_transition() {
        let port = EngineWorkflowPort;
        let def = jump_definition_value();
        let questions = port.step_jump_questions(&def, 0);
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].id, "q1");
        assert_eq!(questions[0].question_type, "boolean");
        // Run sits in Jumping (questions injected); matching answers
        // fire the `when`-gated transition.
        let run = run_value_in_phase(Phase::Jumping);
        let mut answers = serde_json::Map::new();
        answers.insert("q1".to_string(), json!(true));
        let (run, phase) = port
            .handle_jump(run, &def, &answers)
            .expect("jump accepted");
        assert_eq!(phase, WorkflowPhase::Complete);
        assert_eq!(port.run_phase(&run), Some(WorkflowPhase::Complete));
        // Non-matching answers leave no matched transition → error.
        let mut no_match = serde_json::Map::new();
        no_match.insert("q1".to_string(), json!(false));
        let run = run_value_in_phase(Phase::Jumping);
        assert!(port.handle_jump(run, &def, &no_match).is_err());
    }

    #[test]
    fn owner_and_blocked_transitions_roundtrip_through_value() {
        let port = EngineWorkflowPort;
        // Blocked transition carries the pause reason across the port.
        let blocked = port.mark_blocked(run_value(), "waiting for owner");
        assert_eq!(port.run_phase(&blocked), Some(WorkflowPhase::Blocked));
        assert_eq!(
            port.run_info(&blocked).unwrap().paused_reason,
            "waiting for owner"
        );
        // Owner resolve clears the block and returns to Verifying.
        let resolved = port.on_owner_resolve(blocked);
        assert_eq!(port.run_phase(&resolved), Some(WorkflowPhase::Verifying));
        assert_eq!(port.run_info(&resolved).unwrap().paused_reason, "");
        // Owner terminate completes the workflow.
        let terminated = port.on_owner_terminate(run_value());
        assert_eq!(port.run_phase(&terminated), Some(WorkflowPhase::Complete));
    }

    #[test]
    fn definition_metadata_and_messages_are_rendered_from_value() {
        let port = EngineWorkflowPort;
        let def = definition_value();
        assert_eq!(port.definition_verify_retry_limit(&def), Some(3));
        assert_eq!(port.definition_version(&def), Some("1.0".into()));
        assert_eq!(port.definition_step_count(&def), 2);
        assert_eq!(port.step_effective_allow_blocked(&def, 0), Some(false));
        assert_eq!(port.step_effective_allow_blocked(&def, 9), None);
        assert_eq!(port.step_name(&def, 0), Some("step-one".into()));
        assert_eq!(port.step_name(&def, 1), Some("step-two".into()));
        assert!(port.step_jump_questions(&def, 0).is_empty());
        // Message rendering.
        let goal = port.goal_message(&def, 0, WorkflowGoalHint::Normal);
        assert!(goal.unwrap().contains("do the thing"));
        let verify = port.verify_message(&def, 1, false);
        assert!(verify.unwrap().contains("final check"));
        // Context append is built from the definition.
        assert!(!port.build_context_append(&def).is_empty());
        // Out-of-range step renders nothing.
        assert!(port
            .goal_message(&def, 9, WorkflowGoalHint::Normal)
            .is_none());
        assert_eq!(port.step_name(&def, 9), None);
        // Undecodable definition degrades gracefully.
        assert_eq!(port.definition_step_count(&json!(null)), 0);
        assert_eq!(port.build_context_append(&json!(null)), "");
        // Jump message renders from the jump-carrying definition.
        let jump_def = jump_definition_value();
        assert!(port
            .jump_message(&jump_def, 0)
            .unwrap()
            .contains("continue?"));
    }
}
