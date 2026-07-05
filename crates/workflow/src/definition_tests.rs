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
