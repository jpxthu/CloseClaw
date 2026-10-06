//! Test-support thin WorkflowPort implementation wrapping the real
//! workflow engine, loader, and definition functions.
//!
//! Gateway tests construct `WorkflowHandler` values directly; the
//! handler requires a port, so tests supply this wrapper (the
//! production port implementation lives in the daemon composition
//! root).

#[cfg(test)]
pub(crate) mod test_port {
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

    /// Thin wrapper around the real workflow engine.
    pub(crate) struct RealEngineWorkflowPort;

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

    /// Build the test port as `Arc<dyn WorkflowPort>`.
    pub(crate) fn test_port() -> Arc<dyn WorkflowPort> {
        Arc::new(RealEngineWorkflowPort)
    }

    impl WorkflowPort for RealEngineWorkflowPort {
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
            let def = decode_definition(definition)
                .ok_or_else(|| "undecodable definition".to_string())?;
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
            let def = decode_definition(definition)
                .ok_or_else(|| "undecodable definition".to_string())?;
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
            let def = decode_definition(definition)
                .ok_or_else(|| "undecodable definition".to_string())?;
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
            decode_definition(definition)
                .and_then(|def| def.steps.get(step).map(build_jump_message))
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
            decode_definition(definition)
                .and_then(|def| def.steps.get(step).map(|s| s.name.clone()))
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
}
