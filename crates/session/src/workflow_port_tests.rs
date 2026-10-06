//! Tests for WorkflowPort state-erasure round-trip equivalence (Step 1.9).
//!
//! Pins the behavior contract of the workflow dependency inversion:
//!
//! - **Value round-trip equivalence**: every engine operation applied
//!   through the port (state crossing as `serde_json::Value`) yields a
//!   value identical to applying the same operation on the direct typed
//!   [`WorkflowRun`] — the erased representation is a faithful carrier.
//! - **Old checkpoint compatibility**: pre-erasure run JSON (bare-usize
//!   `pending_verify`, missing defaulted fields) decodes without field
//!   loss, drives engine ops across the port, and re-serializes to JSON
//!   the old typed reader still accepts.
//! - **Checkpoint-level round trip**: `SessionCheckpoint.workflow_run`
//!   carries the erased Value losslessly across serialize/parse cycles.

use std::collections::HashMap;

use closeclaw_workflow::definition::{JumpQuestion, Step, Transition, Workflow};
use closeclaw_workflow::engine::WorkflowEngine;
use closeclaw_workflow::run::{GoalHint, PendingVerify, Phase, StepHistoryStatus, WorkflowRun};

use crate::persistence::SessionCheckpoint;
use crate::workflow_port::real_engine_port::test_port;
use crate::workflow_port::{WorkflowGoalHint, WorkflowPhase};

// ── Fixtures ──────────────────────────────────────────────────────────

/// Two-step definition: step 0 has an enum jump question ("fast" → goto
/// step 1, default → complete) and allows blocking; step 1 verifies
/// through a default complete transition and forbids blocking.
fn make_definition() -> Workflow {
    Workflow {
        id: "wf-port-test".to_string(),
        name: "Port Test Workflow".to_string(),
        description: "Workflow for port round-trip tests".to_string(),
        version: Some("1.0".to_string()),
        allow_blocked: false,
        verify_retry_limit: 3,
        step_data_schema: serde_yaml::Value::Null,
        steps: vec![
            Step {
                id: 0,
                name: "Decide".to_string(),
                goal: "Choose an approach".to_string(),
                verify: vec!["Plan reviewed".to_string()],
                jump: vec![JumpQuestion {
                    id: "strategy".to_string(),
                    prompt: "Which strategy?".to_string(),
                    question_type: "enum".to_string(),
                    options: vec!["fast".to_string(), "slow".to_string()],
                    option_labels: vec![],
                }],
                transitions: vec![
                    Transition {
                        when: Some(serde_yaml::from_str("strategy: fast").unwrap()),
                        action: "goto".to_string(),
                        target_step: Some(1),
                    },
                    Transition {
                        when: None,
                        action: "complete".to_string(),
                        target_step: None,
                    },
                ],
                allow_blocked: Some(true),
            },
            Step {
                id: 1,
                name: "Implement".to_string(),
                goal: "Do the work".to_string(),
                verify: vec!["Work checked".to_string()],
                jump: vec![],
                transitions: vec![Transition {
                    when: None,
                    action: "complete".to_string(),
                    target_step: None,
                }],
                allow_blocked: None, // inherits workflow default (false)
            },
        ],
    }
}

fn make_run() -> WorkflowRun {
    WorkflowRun {
        workflow_id: "wf-port-test".to_string(),
        definition_name: "Port Test Workflow".to_string(),
        definition_version: "1.0".to_string(),
        current_step: 0,
        phase: Phase::Executing,
        current_step_entered_at: "2026-01-01T00:00:00+00:00".to_string(),
        step_history: vec![],
        step_data: serde_yaml::Value::Null,
        pending_goal_hint: GoalHint::Normal,
        pending_verify: PendingVerify::default(),
        paused_reason: String::new(),
    }
}

fn as_value<T: serde::Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap()
}

fn as_typed(value: &serde_json::Value) -> WorkflowRun {
    serde_json::from_value(value.clone()).unwrap()
}

/// Normalize wall-clock timestamps (RFC3339 strings) to empty strings so
/// Value equality asserts only deterministic state.
///
/// Engine ops stamp capture time (`pending_verify.last_inject_time`,
/// `current_step_entered_at`, `step_history[*].completed_at`) at call
/// time; the direct-typed call and the port call necessarily observe
/// different instants. The helper blanks every RFC3339-shaped string in
/// both values — every other byte of state stays under assertion, and
/// the stamped fields are separately asserted non-empty where relevant.
fn normalize_timestamps(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            if chrono::DateTime::parse_from_rfc3339(s).is_ok() {
                *s = String::new();
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                normalize_timestamps(item);
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                normalize_timestamps(v);
            }
        }
        _ => {}
    }
}

/// Assert the port-applied op result equals the direct-typed op result
/// at value level (timestamps normalized; see `normalize_timestamps`).
fn assert_value_equivalent(mut direct: serde_json::Value, mut via_port: serde_json::Value) {
    normalize_timestamps(&mut direct);
    normalize_timestamps(&mut via_port);
    assert_eq!(
        direct, via_port,
        "port op must be value-equivalent to direct typed op"
    );
}

// ── ① Callback ops: port vs direct typed engine ──────────────────────

#[test]
fn test_engine_ops_value_equivalent_to_direct_typed_ops() {
    let port = test_port();
    let run = make_run();

    // on_goal_injected: hint reset (no timestamps stamped).
    let mut direct = run.clone();
    WorkflowEngine::on_goal_injected(&mut direct);
    let via_port = port.on_goal_injected(as_value(&run));
    assert_value_equivalent(as_value(&direct), via_port);

    // on_verify_injected within limit → Verifying.
    let mut direct = run.clone();
    WorkflowEngine::on_verify_injected(&mut direct, 3);
    let via_port = port.on_verify_injected(as_value(&run), 3);
    assert_eq!(
        port.run_phase(&via_port),
        Some(WorkflowPhase::Verifying),
        "verify injected within limit must reach Verifying through the port"
    );
    assert_value_equivalent(as_value(&direct), via_port.clone());
    let stamped = as_typed(&via_port);
    assert_eq!(stamped.pending_verify.count, 1);
    assert_eq!(stamped.pending_verify.max_retry_limit, 3);
    assert!(
        !stamped.pending_verify.last_inject_time.is_empty(),
        "port op must stamp the inject timestamp"
    );

    // on_verify_injected at the limit → Blocked with pause reason.
    let mut direct = run.clone();
    WorkflowEngine::on_verify_injected(&mut direct, 1);
    let via_port = port.on_verify_injected(as_value(&run), 1);
    assert_eq!(port.run_phase(&via_port), Some(WorkflowPhase::Blocked));
    assert_value_equivalent(as_value(&direct), via_port.clone());
    assert_eq!(as_typed(&via_port).paused_reason, "验收重试次数耗尽");

    // on_owner_resolve from Blocked → Verifying, reason cleared, count reset.
    let mut direct = run.clone();
    WorkflowEngine::on_verify_injected(&mut direct, 1);
    WorkflowEngine::on_owner_resolve(&mut direct);
    let blocked = port.on_verify_injected(as_value(&run), 1);
    let via_port = port.on_owner_resolve(blocked);
    assert_eq!(port.run_phase(&via_port), Some(WorkflowPhase::Verifying));
    assert_value_equivalent(as_value(&direct), via_port.clone());
    let resolved = as_typed(&via_port);
    assert_eq!(resolved.pending_verify.count, 0);
    assert_eq!(resolved.paused_reason, "");

    // on_owner_terminate → Complete.
    let mut direct = run.clone();
    WorkflowEngine::on_owner_terminate(&mut direct);
    let via_port = port.on_owner_terminate(as_value(&run));
    assert_eq!(port.run_phase(&via_port), Some(WorkflowPhase::Complete));
    assert_value_equivalent(as_value(&direct), via_port);

    // mark_blocked: port-only helper mirrors the manual field mutation
    // (recovery definition-change path).
    let mut direct = run.clone();
    direct.phase = Phase::Blocked;
    direct.paused_reason = "definition changed".to_string();
    let via_port = port.mark_blocked(as_value(&run), "definition changed");
    assert_eq!(port.run_phase(&via_port), Some(WorkflowPhase::Blocked));
    assert_value_equivalent(as_value(&direct), via_port);
}

#[test]
fn test_tool_result_ops_value_equivalent_to_direct_typed_ops() {
    let port = test_port();
    let definition = make_definition();
    let definition_value = serde_json::to_value(&definition).unwrap();

    // handle_verify from Verifying on a step with jump questions → Jumping.
    let run = WorkflowRun {
        phase: Phase::Verifying,
        ..make_run()
    };
    let mut direct = run.clone();
    WorkflowEngine::handle_verify(&mut direct, &definition).unwrap();
    let (verifying_value, phase) = port
        .handle_verify(as_value(&run), &definition_value)
        .unwrap();
    assert_eq!(phase, WorkflowPhase::Jumping);
    assert_value_equivalent(as_value(&direct), verifying_value.clone());

    // handle_jump with a matching enum answer → goto step 1 (Executing).
    let mut direct = run.clone();
    WorkflowEngine::handle_verify(&mut direct, &definition).unwrap();
    let yaml_answers: HashMap<String, serde_yaml::Value> = HashMap::from([(
        "strategy".to_string(),
        serde_yaml::from_str("fast").unwrap(),
    )]);
    WorkflowEngine::handle_jump(&mut direct, &definition, &yaml_answers).unwrap();
    let mut json_answers = serde_json::Map::new();
    json_answers.insert("strategy".to_string(), serde_json::json!("fast"));
    let (jumped_value, phase) = port
        .handle_jump(verifying_value, &definition_value, &json_answers)
        .unwrap();
    assert_eq!(phase, WorkflowPhase::Executing);
    assert_value_equivalent(as_value(&direct), jumped_value.clone());
    let jumped = as_typed(&jumped_value);
    assert_eq!(jumped.current_step, 1, "goto must land on step 1");
    assert_eq!(jumped.step_history.len(), 1);
    assert_eq!(jumped.pending_verify.count, 0);

    // handle_verify on the final step (no jump, default complete) → Complete.
    let final_step = WorkflowRun {
        current_step: 1,
        phase: Phase::Verifying,
        ..make_run()
    };
    let mut direct = final_step.clone();
    WorkflowEngine::handle_verify(&mut direct, &definition).unwrap();
    let (via_port, phase) = port
        .handle_verify(as_value(&final_step), &definition_value)
        .unwrap();
    assert_eq!(phase, WorkflowPhase::Complete);
    assert_value_equivalent(as_value(&direct), via_port);

    // handle_blocked allowed (step 0 override) → Blocked with reason.
    let blocked_run = WorkflowRun {
        phase: Phase::Verifying,
        ..make_run()
    };
    let mut direct = blocked_run.clone();
    WorkflowEngine::handle_blocked(&mut direct, &definition, true, "need owner").unwrap();
    let via_port = port
        .handle_blocked(
            as_value(&blocked_run),
            &definition_value,
            true,
            "need owner",
        )
        .unwrap();
    assert_eq!(port.run_phase(&via_port), Some(WorkflowPhase::Blocked));
    assert_value_equivalent(as_value(&direct), via_port.clone());
    assert_eq!(as_typed(&via_port).paused_reason, "need owner");

    // handle_blocked not allowed (step 1 inherits workflow false) → both
    // sides surface the same engine error, state untouched.
    let final_verifying = WorkflowRun {
        current_step: 1,
        phase: Phase::Verifying,
        ..make_run()
    };
    let mut direct = final_verifying.clone();
    let direct_err =
        WorkflowEngine::handle_blocked(&mut direct, &definition, false, "blocked!").unwrap_err();
    let port_err = port
        .handle_blocked(
            as_value(&final_verifying),
            &definition_value,
            false,
            "blocked!",
        )
        .unwrap_err();
    assert_eq!(
        direct_err.to_string(),
        port_err,
        "port must surface the same engine error"
    );
    assert_eq!(as_value(&direct), as_value(&final_verifying));

    // handle_verify from the wrong phase → both sides error.
    let executing = make_run(); // phase = Executing, not Verifying
    let mut direct = executing.clone();
    assert!(WorkflowEngine::handle_verify(&mut direct, &definition).is_err());
    assert!(port
        .handle_verify(as_value(&executing), &definition_value)
        .is_err());
}

// ── Port mirror + undecodable-state boundaries ───────────────────────

#[test]
fn test_run_info_mirrors_typed_fields() {
    let port = test_port();
    let run = WorkflowRun {
        pending_goal_hint: GoalHint::Reexecute,
        paused_reason: "waiting".to_string(),
        step_history: vec![closeclaw_workflow::run::StepHistoryEntry {
            step_id: 0,
            step_name: "Decide".to_string(),
            entered_at: "2026-01-01T00:00:00+00:00".to_string(),
            completed_at: "2026-01-01T00:01:00+00:00".to_string(),
            status: StepHistoryStatus::Completed,
        }],
        ..make_run()
    };
    let info = port.run_info(&as_value(&run)).unwrap();
    assert_eq!(info.workflow_id, "wf-port-test");
    assert_eq!(info.definition_name, "Port Test Workflow");
    assert_eq!(info.definition_version, "1.0");
    assert_eq!(info.current_step, 0);
    assert_eq!(info.phase, WorkflowPhase::Executing);
    assert_eq!(info.pending_goal_hint, WorkflowGoalHint::Reexecute);
    assert_eq!(info.paused_reason, "waiting");
    assert_eq!(info.last_history_step_name.as_deref(), Some("Decide"));

    // on_session_idle mirrors the engine's phase gate.
    assert!(port.on_session_idle(&as_value(&run)));
    let complete = WorkflowRun {
        phase: Phase::Complete,
        ..run
    };
    assert!(!port.on_session_idle(&as_value(&complete)));
}

#[test]
fn test_undecodable_state_boundaries() {
    let port = test_port();
    let garbage = serde_json::json!({"not": "a workflow run"});

    assert_eq!(port.run_phase(&garbage), None);
    assert_eq!(port.run_info(&garbage), None);
    assert_eq!(port.run_phase(&serde_json::Value::Null), None);

    // Non-decoding state passes through unchanged (no panic, no loss).
    assert_eq!(port.on_goal_injected(garbage.clone()), garbage);
    assert_eq!(port.on_owner_resolve(garbage.clone()), garbage);

    // Engine-op methods report undecodable state as an error.
    let definition_value = as_value(&make_definition());
    assert!(port
        .handle_verify(garbage.clone(), &definition_value)
        .is_err());
    assert!(port
        .handle_jump(garbage.clone(), &definition_value, &serde_json::Map::new())
        .is_err());
    assert!(port
        .handle_blocked(garbage, &definition_value, true, "r")
        .is_err());
}

// ── Old-format run state: decode → operate → rewrite ─────────────────

/// The oldest checkpoint shape: `pending_verify` was a bare `usize` and
/// the later-added fields (`definition_name`, `current_step_entered_at`,
/// `step_data`, `pending_goal_hint`, `pending_verify` map keys,
/// `paused_reason`) were absent.
fn old_format_run_json() -> serde_json::Value {
    serde_json::from_str(
        r#"{
            "workflow_id": "wf-old",
            "definition_version": "0.9",
            "current_step": 0,
            "phase": "verifying",
            "step_history": [],
            "pending_verify": 2
        }"#,
    )
    .unwrap()
}

#[test]
fn test_old_checkpoint_bare_usize_pending_verify_roundtrip() {
    let port = test_port();
    let old = old_format_run_json();

    // Read: count carried over, missing keys get documented defaults,
    // orchestration fields surface correctly through the mirror.
    let info = port.run_info(&old).expect("old-format run must decode");
    assert_eq!(info.workflow_id, "wf-old");
    assert_eq!(info.phase, WorkflowPhase::Verifying);
    let typed_old = as_typed(&old);
    assert_eq!(
        typed_old.pending_verify.count, 2,
        "bare usize must land in count"
    );
    assert_eq!(
        typed_old.pending_verify.max_retry_limit, 3,
        "missing max_retry_limit defaults to 3"
    );
    assert_eq!(typed_old.pending_verify.last_inject_time, "");
    assert_eq!(typed_old.definition_name, "");
    assert_eq!(typed_old.paused_reason, "");
    assert_eq!(typed_old.pending_goal_hint, GoalHint::Normal);

    // Operate: the engine advances old state across the port — count
    // 2 → 3 with a fresh limit of 5 stays under the limit (Verifying).
    let advanced = port.on_verify_injected(old, 5);
    assert_eq!(port.run_phase(&advanced), Some(WorkflowPhase::Verifying));
    let typed_advanced = as_typed(&advanced);
    assert_eq!(typed_advanced.pending_verify.count, 3);
    assert_eq!(typed_advanced.pending_verify.max_retry_limit, 5);
    assert!(!typed_advanced.pending_verify.last_inject_time.is_empty());

    // Rewrite: the new JSON is still readable by the old typed reader —
    // the map carries every key with the incremented count.
    let reparsed = as_typed(&advanced);
    assert_eq!(reparsed.pending_verify.count, 3);
    assert_eq!(reparsed.pending_verify.max_retry_limit, 5);
    let verify_json = advanced["pending_verify"].as_object().unwrap();
    assert_eq!(
        verify_json.len(),
        3,
        "rewritten pending_verify must be the full map"
    );
    assert!(verify_json.contains_key("count"));
    assert!(verify_json.contains_key("last_inject_time"));
    assert!(verify_json.contains_key("max_retry_limit"));
}

#[test]
fn test_old_checkpoint_partial_map_pending_verify_defaults() {
    let port = test_port();
    // Intermediate old format: map form but with only some keys.
    let old: serde_json::Value = serde_json::from_str(
        r#"{
            "workflow_id": "wf-old",
            "definition_version": "0.9",
            "current_step": 0,
            "phase": "blocked",
            "step_history": [],
            "pending_verify": {"count": 1},
            "paused_reason": "waiting for owner"
        }"#,
    )
    .unwrap();

    let typed_old = as_typed(&old);
    assert_eq!(typed_old.pending_verify.count, 1);
    assert_eq!(
        typed_old.pending_verify.max_retry_limit, 3,
        "map key absent → default 3"
    );
    assert_eq!(typed_old.pending_verify.last_inject_time, "");

    // Engine op across the port: owner resolve resets count, clears
    // reason, and the rewritten state keeps the full map shape.
    let resolved = port.on_owner_resolve(old);
    let typed_resolved = as_typed(&resolved);
    assert_eq!(typed_resolved.phase, Phase::Verifying);
    assert_eq!(typed_resolved.pending_verify.count, 0);
    assert_eq!(typed_resolved.paused_reason, "");
    assert_eq!(
        resolved["pending_verify"].as_object().unwrap().len(),
        3,
        "rewrite must not drop pending_verify keys"
    );
}

#[test]
fn test_old_checkpoint_missing_fields_preserved_on_rewrite() {
    let old = old_format_run_json();

    // Decode the old shape and write it back: every defaulted field is
    // materialized (no field loss), and the rewrite is a fixpoint — the
    // old reader round-trips it identically.
    let decoded = as_typed(&old);
    let rewritten = as_value(&decoded);
    for key in [
        "workflow_id",
        "definition_name",
        "definition_version",
        "current_step",
        "phase",
        "current_step_entered_at",
        "step_history",
        "step_data",
        "pending_goal_hint",
        "pending_verify",
        "paused_reason",
    ] {
        assert!(
            rewritten.get(key).is_some(),
            "rewritten run must contain `{key}` (no field loss on rewrite)"
        );
    }
    let again = as_typed(&rewritten);
    assert_eq!(as_value(&again), rewritten, "rewrite must be a fixpoint");
}

// ── Checkpoint-level Value round trip ────────────────────────────────

#[test]
fn test_checkpoint_workflow_run_value_roundtrip() {
    let port = test_port();
    let run = make_run();

    // Write: erased Value inside the session checkpoint.
    let mut checkpoint = SessionCheckpoint::new("wf-checkpoint".into());
    checkpoint.workflow_run = Some(as_value(&run));
    let json = serde_json::to_string(&checkpoint).unwrap();

    // Parse back: the stored Value is identical to a fresh serialization
    // of the same run (lossless carry, deterministic for this state).
    let parsed: SessionCheckpoint = serde_json::from_str(&json).unwrap();
    let stored = parsed
        .workflow_run
        .clone()
        .expect("workflow_run survives round trip");
    assert_eq!(stored, as_value(&run));

    // Operate on the restored state across the port, write back, parse
    // again: state stays lossless through the full persistence cycle.
    let advanced = port.on_verify_injected(stored, 3);
    let mut checkpoint = parsed;
    checkpoint.workflow_run = Some(advanced);
    let json2 = serde_json::to_string(&checkpoint).unwrap();
    let reparsed: SessionCheckpoint = serde_json::from_str(&json2).unwrap();
    let restored = reparsed
        .workflow_run
        .expect("workflow_run survives second round trip");
    assert_eq!(port.run_phase(&restored), Some(WorkflowPhase::Verifying));
    assert_eq!(as_typed(&restored).pending_verify.count, 1);
}

#[test]
fn test_checkpoint_old_json_without_workflow_run_defaults_to_none() {
    // Old checkpoints predate the workflow_run key entirely → serde
    // default keeps it None (no workflow was active).
    let checkpoint = SessionCheckpoint::new("old-checkpoint".into());
    let mut value = serde_json::to_value(&checkpoint).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("workflow_run")
        .expect("fresh checkpoint serializes the workflow_run key");
    let parsed: SessionCheckpoint = serde_json::from_value(value).unwrap();
    assert!(
        parsed.workflow_run.is_none(),
        "old checkpoint without workflow_run key must default to None"
    );
}

// ── Definition metadata queries (message/definition layer parity) ────

#[test]
fn test_definition_metadata_queries_match_typed_definition() {
    let port = test_port();
    let definition = make_definition();
    let definition_value = serde_json::to_value(&definition).unwrap();

    assert_eq!(
        port.definition_verify_retry_limit(&definition_value),
        Some(3)
    );
    assert_eq!(
        port.definition_version(&definition_value),
        Some("1.0".to_string())
    );
    assert_eq!(port.definition_step_count(&definition_value), 2);
    // Step 0: explicit override true; step 1: inherits workflow default false.
    assert_eq!(
        port.step_effective_allow_blocked(&definition_value, 0),
        Some(true)
    );
    assert_eq!(
        port.step_effective_allow_blocked(&definition_value, 1),
        Some(false)
    );
    assert_eq!(
        port.step_effective_allow_blocked(&definition_value, 9),
        None
    );
    assert_eq!(
        port.step_name(&definition_value, 0),
        Some("Decide".to_string())
    );
    assert_eq!(port.step_name(&definition_value, 9), None);

    let questions = port.step_jump_questions(&definition_value, 0);
    assert_eq!(questions.len(), 1);
    assert_eq!(questions[0].id, "strategy");
    assert_eq!(questions[0].question_type, "enum");
    assert_eq!(
        questions[0].options,
        vec!["fast".to_string(), "slow".to_string()]
    );
    assert!(port.step_jump_questions(&definition_value, 1).is_empty());
    assert!(port.step_jump_questions(&definition_value, 9).is_empty());
}
