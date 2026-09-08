//! Unit tests for workflow definition parsing.

use crate::definition::Workflow;

// ---------------------------------------------------------------------------
// Helper: build a minimal valid YAML for a single-step workflow
// ---------------------------------------------------------------------------

fn single_step_yaml() -> &'static str {
    r#"
id: test-workflow
name: Test Workflow
description: A test workflow
steps:
  - id: 0
    name: Step Zero
    goal: Do something
"#
}

fn multi_step_yaml() -> &'static str {
    r#"
id: multi-step
name: Multi Step
description: Multiple steps workflow
allow_blocked: true
verify_retry_limit: 5
step_data_schema:
  repo_url: string
  branch: string
steps:
  - id: 0
    name: Analyze
    goal: Analyze the issue
    verify:
      - Issue reproduced
      - Root cause identified
    jump:
      - id: needs_pr
        prompt: Does this need a PR?
        type: boolean
    transitions:
      - when:
          needs_pr: true
        action: goto
        target_step: 1
      - action: complete
  - id: 1
    name: Implement
    allow_blocked: true
    goal: Write the fix
    verify:
      - Fix implemented
      - Tests pass
    transitions:
      - action: complete
"#
}

fn frontmatter_wrapped_yaml() -> &'static str {
    r#"---
id: wrapped
name: Wrapped Workflow
description: Wrapped in frontmatter delimiters
steps:
  - id: 0
    name: Step One
    goal: Execute
---
"#
}

// ---------------------------------------------------------------------------
// parse_frontmatter: happy path
// ---------------------------------------------------------------------------

#[test]
fn test_parse_single_step_workflow() {
    let wf = Workflow::parse_frontmatter(single_step_yaml()).unwrap();
    assert_eq!(wf.id, "test-workflow");
    assert_eq!(wf.name, "Test Workflow");
    assert_eq!(wf.description, "A test workflow");
    assert_eq!(wf.steps.len(), 1);
    assert_eq!(wf.steps[0].id, 0);
    assert_eq!(wf.steps[0].name, "Step Zero");
    assert_eq!(wf.steps[0].goal, "Do something");
}

#[test]
fn test_parse_multi_step_workflow() {
    let wf = Workflow::parse_frontmatter(multi_step_yaml()).unwrap();
    assert_eq!(wf.id, "multi-step");
    assert!(wf.allow_blocked);
    assert_eq!(wf.verify_retry_limit, 5);
    assert_eq!(wf.steps.len(), 2);
    assert_eq!(wf.steps[0].id, 0);
    assert_eq!(wf.steps[1].id, 1);
    assert!(wf.steps[1].allow_blocked.unwrap());
}

#[test]
fn test_parse_with_frontmatter_delimiters() {
    let wf = Workflow::parse_frontmatter(frontmatter_wrapped_yaml()).unwrap();
    assert_eq!(wf.id, "wrapped");
    assert_eq!(wf.steps.len(), 1);
}

// ---------------------------------------------------------------------------
// parse_frontmatter: default values
// ---------------------------------------------------------------------------

#[test]
fn test_default_allow_blocked_is_false() {
    let wf = Workflow::parse_frontmatter(single_step_yaml()).unwrap();
    assert!(!wf.allow_blocked);
}

#[test]
fn test_default_verify_retry_limit() {
    let wf = Workflow::parse_frontmatter(single_step_yaml()).unwrap();
    assert_eq!(wf.verify_retry_limit, 3);
}

#[test]
fn test_default_step_data_schema() {
    let wf = Workflow::parse_frontmatter(single_step_yaml()).unwrap();
    assert!(wf.step_data_schema.is_null());
}

#[test]
fn test_default_step_verify_and_jumps() {
    let wf = Workflow::parse_frontmatter(single_step_yaml()).unwrap();
    assert!(wf.steps[0].verify.is_empty());
    assert!(wf.steps[0].jump.is_empty());
    assert!(wf.steps[0].transitions.is_empty());
    assert!(wf.steps[0].allow_blocked.is_none());
}

// ---------------------------------------------------------------------------
// parse_frontmatter: step numbering starts at 0
// ---------------------------------------------------------------------------

#[test]
fn test_step_numbering_starts_at_zero() {
    let wf = Workflow::parse_frontmatter(multi_step_yaml()).unwrap();
    assert_eq!(wf.steps[0].id, 0);
    assert_eq!(wf.steps[1].id, 1);
}

// ---------------------------------------------------------------------------
// parse_frontmatter: transitions and jump questions
// ---------------------------------------------------------------------------

#[test]
fn test_parse_transitions_with_when() {
    let wf = Workflow::parse_frontmatter(multi_step_yaml()).unwrap();
    let step0 = &wf.steps[0];
    assert_eq!(step0.transitions.len(), 2);
    assert!(step0.transitions[0].when.is_some());
    assert_eq!(step0.transitions[0].action, "goto");
    assert_eq!(step0.transitions[0].target_step, Some(1));
    assert!(step0.transitions[1].when.is_none());
    assert_eq!(step0.transitions[1].action, "complete");
}

#[test]
fn test_parse_jump_questions() {
    let wf = Workflow::parse_frontmatter(multi_step_yaml()).unwrap();
    let jump = &wf.steps[0].jump;
    assert_eq!(jump.len(), 1);
    assert_eq!(jump[0].id, "needs_pr");
    assert_eq!(jump[0].question_type, "boolean");
    assert!(jump[0].options.is_empty());
}

#[test]
fn test_parse_enum_jump_question() {
    let yaml = r#"
id: enum-jump
name: Enum Jump
description: Has enum jump question
steps:
  - id: 0
    name: Decide
    goal: Choose a path
    jump:
      - id: path_choice
        prompt: Which path?
        type: enum
        options: [fast, slow, balanced]
        option_labels:
          - Fast path
          - Slow path
          - Balanced path
    transitions:
      - when:
          path_choice: fast
        action: goto
        target_step: 0
      - action: complete
"#;
    let wf = Workflow::parse_frontmatter(yaml).unwrap();
    let jump = &wf.steps[0].jump[0];
    assert_eq!(jump.options, vec!["fast", "slow", "balanced"]);
    assert_eq!(
        jump.option_labels,
        vec!["Fast path", "Slow path", "Balanced path"]
    );
}

// ---------------------------------------------------------------------------
// parse_frontmatter: step_data_schema
// ---------------------------------------------------------------------------

#[test]
fn test_parse_step_data_schema() {
    let wf = Workflow::parse_frontmatter(multi_step_yaml()).unwrap();
    let schema = wf.step_data_schema.as_mapping().unwrap();
    assert_eq!(schema.len(), 2);
}

// ---------------------------------------------------------------------------
// parse_frontmatter: errors
// ---------------------------------------------------------------------------

#[test]
fn test_parse_empty_yaml_returns_error() {
    let result = Workflow::parse_frontmatter("");
    assert!(result.is_err());
}

#[test]
fn test_parse_missing_required_field_returns_error() {
    let yaml = r#"
name: No ID
description: Missing id field
steps:
  - id: 0
    name: Step
    goal: Goal
"#;
    let result = Workflow::parse_frontmatter(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err();
    // Should be a ParseError, not InvalidDefinition
    assert!(format!("{err}").contains("failed to parse"));
}

#[test]
fn test_parse_invalid_yaml_syntax_returns_error() {
    let yaml = r#"
id: bad
name: [invalid yaml
steps:
"#;
    let result = Workflow::parse_frontmatter(yaml);
    assert!(result.is_err());
    assert!(format!("{}", result.unwrap_err()).contains("failed to parse"));
}

#[test]
fn test_parse_steps_not_array_returns_error() {
    let yaml = r#"
id: bad-steps
name: Bad Steps
description: Steps is not an array
steps: "not an array"
"#;
    let result = Workflow::parse_frontmatter(yaml);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// parse_skill_md: happy path
// ---------------------------------------------------------------------------

#[test]
fn test_parse_skill_md_with_valid_frontmatter() {
    let md = "---\nid: skill-md\nname: Skill MD\ndescription: Parsed from SKILL.md\nsteps:\n  - id: 0\n    name: Step\n    goal: Goal\n---\n\nSome body content that should be ignored.\n";
    let wf = Workflow::parse_skill_md(md).unwrap();
    assert_eq!(wf.id, "skill-md");
}

// ---------------------------------------------------------------------------
// parse_skill_md: errors
// ---------------------------------------------------------------------------

#[test]
fn test_parse_skill_md_missing_opening_delimiter() {
    let result = Workflow::parse_skill_md("no frontmatter here");
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("missing opening"));
}

#[test]
fn test_parse_skill_md_missing_closing_delimiter() {
    let md = "---\nid: incomplete\nname: No End\nsteps: []";
    let result = Workflow::parse_skill_md(md);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("missing closing"));
}

#[test]
fn test_parse_skill_md_empty_file() {
    let result = Workflow::parse_skill_md("");
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// parse_frontmatter: custom retry limit
// ---------------------------------------------------------------------------

#[test]
fn test_custom_verify_retry_limit() {
    let yaml = r#"
id: retry-test
name: Retry Test
description: Custom retry limit
verify_retry_limit: 10
steps:
  - id: 0
    name: Step
    goal: Goal
"#;
    let wf = Workflow::parse_frontmatter(yaml).unwrap();
    assert_eq!(wf.verify_retry_limit, 10);
}

// ---------------------------------------------------------------------------
// parse_frontmatter: step with allow_blocked override
// ---------------------------------------------------------------------------

#[test]
fn test_step_allow_blocked_override() {
    let yaml = r#"
id: override-test
name: Override Test
description: Step level override
allow_blocked: false
steps:
  - id: 0
    name: Blocked Step
    allow_blocked: true
    goal: Can block
  - id: 1
    name: Non-blocked Step
    goal: Cannot block
"#;
    let wf = Workflow::parse_frontmatter(yaml).unwrap();
    assert!(!wf.allow_blocked);
    assert!(wf.steps[0].allow_blocked.unwrap());
    assert!(wf.steps[1].allow_blocked.is_none());
}

// ---------------------------------------------------------------------------
// round-trip: serialize then deserialize
// ---------------------------------------------------------------------------

#[test]
fn test_round_trip_serde() {
    let wf = Workflow::parse_frontmatter(multi_step_yaml()).unwrap();
    let serialized = serde_yaml::to_string(&wf).unwrap();
    let deserialized: Workflow = serde_yaml::from_str(&serialized).unwrap();
    assert_eq!(wf.id, deserialized.id);
    assert_eq!(wf.steps.len(), deserialized.steps.len());
    assert_eq!(wf.allow_blocked, deserialized.allow_blocked);
    assert_eq!(wf.verify_retry_limit, deserialized.verify_retry_limit);
}

// ---------------------------------------------------------------------------
// build_verify_message: basic rendering
// ---------------------------------------------------------------------------

use crate::definition::{build_goal_message, build_jump_message, build_verify_message, Step};
use crate::run::GoalHint;

fn make_step(id: usize, name: &str, verify: Vec<&str>) -> Step {
    Step {
        id,
        name: name.to_string(),
        allow_blocked: None,
        goal: "test goal".to_string(),
        verify: verify.into_iter().map(String::from).collect(),
        jump: vec![],
        transitions: vec![],
    }
}

#[test]
fn test_build_verify_message_with_checklist() {
    let step = make_step(2, "Deploy", vec!["Build succeeds", "Tests pass"]);
    let msg = build_verify_message(&step, false);
    assert!(msg.starts_with("Verify Step 2 (Deploy):"));
    assert!(msg.contains("Build succeeds"));
    assert!(msg.contains("Tests pass"));
    assert!(!msg.contains("workflow_blocked"));
}

#[test]
fn test_build_verify_message_without_checklist() {
    let step = make_step(0, "Analyze", vec![]);
    let msg = build_verify_message(&step, false);
    assert!(msg.contains("\u{2014} no explicit checklist."));
    assert!(!msg.contains("workflow_blocked"));
}

#[test]
fn test_build_verify_message_allow_blocked_true() {
    let step = make_step(1, "Fix", vec!["Issue resolved"]);
    let msg = build_verify_message(&step, true);
    assert!(msg.contains("workflow_blocked"));
    assert!(msg.contains("如果确认任务无法继续"));
}

#[test]
fn test_build_verify_message_allow_blocked_false() {
    let step = make_step(1, "Fix", vec!["Issue resolved"]);
    let msg = build_verify_message(&step, false);
    assert!(!msg.contains("workflow_blocked"));
}

#[test]
fn test_build_verify_message_empty_name() {
    let step = make_step(0, "", vec!["Check"]);
    let msg = build_verify_message(&step, false);
    assert!(msg.starts_with("Verify Step 0 ():"));
    assert!(msg.contains("Check"));
}

#[test]
fn test_build_verify_message_empty_name_with_allow_blocked() {
    let step = make_step(0, "", vec![]);
    let msg = build_verify_message(&step, true);
    assert!(msg.contains("\u{2014} no explicit checklist."));
    assert!(msg.contains("workflow_blocked"));
}

// ---------------------------------------------------------------------------
// build_goal_message: Normal mode
// ---------------------------------------------------------------------------

#[test]
fn test_build_goal_message_normal_format() {
    let mut step = make_step(1, "Analyze", vec![]);
    step.goal = "Find the root cause".to_string();
    let msg = build_goal_message(&step, GoalHint::Normal);
    assert!(msg.starts_with("[workflow goal] Step 1: Analyze"));
    assert!(msg.contains("Find the root cause"));
    assert!(!msg.contains("重新执行"));
}

#[test]
fn test_build_goal_message_reexecute_appends_hint() {
    let mut step = make_step(2, "Deploy", vec![]);
    step.goal = "Deploy to staging".to_string();
    let msg = build_goal_message(&step, GoalHint::Reexecute);
    assert!(msg.starts_with("[workflow goal] Step 2: Deploy"));
    assert!(msg.contains("Deploy to staging"));
    assert!(msg.contains("重新执行"));
    assert!(msg.contains("step_data"));
    assert!(msg.contains("验收清单"));
}

// ---------------------------------------------------------------------------
// build_jump_message: enum with option_labels renders ABCD
// ---------------------------------------------------------------------------

fn make_jump_step_with_enum_labels() -> Step {
    Step {
        id: 0,
        name: "Decide".to_string(),
        allow_blocked: None,
        goal: "Choose path".to_string(),
        verify: vec![],
        jump: vec![crate::definition::JumpQuestion {
            id: "path".to_string(),
            prompt: "Which path?".to_string(),
            question_type: "enum".to_string(),
            options: vec!["fast".into(), "slow".into(), "balanced".into()],
            option_labels: vec![
                "Fast path".into(),
                "Slow path".into(),
                "Balanced path".into(),
            ],
        }],
        transitions: vec![],
    }
}

#[test]
fn test_build_jump_message_enum_with_labels_abcd() {
    let step = make_jump_step_with_enum_labels();
    let msg = build_jump_message(&step);
    assert!(msg.contains("A \u{2014} Fast path"));
    assert!(msg.contains("B \u{2014} Slow path"));
    assert!(msg.contains("C \u{2014} Balanced path"));
    assert!(msg.contains("path = <选项字母>"));
    assert!(msg.contains("workflow_jump({answers: {<id>: <值>, ...}})"));
    assert!(msg.contains("enum 类型的答案传选项字母"));
}

// ---------------------------------------------------------------------------
// build_jump_message: enum without labels falls back to options
// ---------------------------------------------------------------------------

fn make_jump_step_enum_no_labels() -> Step {
    Step {
        id: 1,
        name: "Choose".to_string(),
        allow_blocked: None,
        goal: "Pick one".to_string(),
        verify: vec![],
        jump: vec![crate::definition::JumpQuestion {
            id: "choice".to_string(),
            prompt: "Pick an option".to_string(),
            question_type: "enum".to_string(),
            options: vec!["opt_a".into(), "opt_b".into()],
            option_labels: vec![],
        }],
        transitions: vec![],
    }
}

#[test]
fn test_build_jump_message_enum_no_labels_fallback_options() {
    let step = make_jump_step_enum_no_labels();
    let msg = build_jump_message(&step);
    assert!(msg.contains("A \u{2014} opt_a"));
    assert!(msg.contains("B \u{2014} opt_b"));
    assert!(msg.contains("choice = <选项字母>"));
}

// ---------------------------------------------------------------------------
// build_jump_message: boolean renders true/false
// ---------------------------------------------------------------------------

fn make_jump_step_boolean() -> Step {
    Step {
        id: 0,
        name: "Ready".to_string(),
        allow_blocked: None,
        goal: "Check readiness".to_string(),
        verify: vec![],
        jump: vec![crate::definition::JumpQuestion {
            id: "go_next".to_string(),
            prompt: "Ready to proceed?".to_string(),
            question_type: "boolean".to_string(),
            options: vec![],
            option_labels: vec![],
        }],
        transitions: vec![],
    }
}

#[test]
fn test_build_jump_message_boolean_renders_true_false() {
    let step = make_jump_step_boolean();
    let msg = build_jump_message(&step);
    assert!(msg.contains("true / false"));
    assert!(msg.contains("go_next = true 或 false"));
    assert!(msg.contains("workflow_jump({answers: {<id>: <值>, ...}})"));
}

// ---------------------------------------------------------------------------
// build_jump_message: multiple questions rendered in ABCD order
// ---------------------------------------------------------------------------

fn make_jump_step_multi() -> Step {
    Step {
        id: 2,
        name: "Multi".to_string(),
        allow_blocked: None,
        goal: "Multiple questions".to_string(),
        verify: vec![],
        jump: vec![
            crate::definition::JumpQuestion {
                id: "q1".to_string(),
                prompt: "First question".to_string(),
                question_type: "boolean".to_string(),
                options: vec![],
                option_labels: vec![],
            },
            crate::definition::JumpQuestion {
                id: "q2".to_string(),
                prompt: "Second question".to_string(),
                question_type: "enum".to_string(),
                options: vec!["x".into(), "y".into()],
                option_labels: vec!["X label".into(), "Y label".into()],
            },
        ],
        transitions: vec![],
    }
}

#[test]
fn test_build_jump_message_multiple_questions_abcd_order() {
    let step = make_jump_step_multi();
    let msg = build_jump_message(&step);
    assert!(msg.contains("A. First question"));
    assert!(msg.contains("B. Second question"));
    assert!(msg.contains("A \u{2014} X label"));
    assert!(msg.contains("B \u{2014} Y label"));
    assert!(msg.contains("true / false"));
    assert!(msg.contains("q2 = <选项字母>"));
}

// ---------------------------------------------------------------------------
// build_jump_message: header includes step id and name
// ---------------------------------------------------------------------------

#[test]
fn test_build_jump_message_header() {
    let step = make_jump_step_boolean();
    let msg = build_jump_message(&step);
    assert!(msg.starts_with("Jump Step 0 (Ready) \u{2014} 请回答以下跳转问题:"));
}
