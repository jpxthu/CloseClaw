//! Shared test fixtures for workflow engine tests.

use crate::definition::{Step, Transition, Workflow};

pub fn simple_workflow() -> Workflow {
    let yaml = r#"
id: simple
name: Simple
description: Single step workflow
steps:
  - id: 0
    name: Only Step
    goal: Do the thing
    verify:
      - Task completed
    transitions:
      - action: complete
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}

pub fn two_step_goto_workflow() -> Workflow {
    let yaml = r#"
id: two-step
name: Two Step
description: Two step workflow
steps:
  - id: 0
    name: First
    goal: Step one
    verify:
      - Step one done
    jump:
      - id: go_next
        prompt: Go to next?
        type: boolean
    transitions:
      - when:
          go_next: true
        action: goto
        target_step: 1
      - action: complete
  - id: 1
    name: Second
    goal: Step two
    verify:
      - Step two done
    transitions:
      - action: complete
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}

pub fn two_step_default_goto_workflow() -> Workflow {
    let yaml = r#"
id: default-goto
name: Default Goto
description: Default goto on first step
steps:
  - id: 0
    name: First
    goal: Step one
    verify:
      - Step one done
    transitions:
      - action: goto
        target_step: 1
  - id: 1
    name: Second
    goal: Step two
    verify:
      - Step two done
    transitions:
      - action: complete
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}

pub fn reexecute_workflow() -> Workflow {
    let yaml = r#"
id: reexec
name: Reexec
description: Reexecute workflow
steps:
  - id: 0
    name: Loop
    goal: Loop until done
    verify:
      - Loop iteration done
    jump:
      - id: retry
        prompt: Retry?
        type: boolean
    transitions:
      - when:
          retry: true
        action: reexecute
        target_step: 0
      - action: complete
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}

pub fn blocked_workflow() -> Workflow {
    let yaml = r#"
id: blocked
name: Blocked
description: Blockable workflow
allow_blocked: false
steps:
  - id: 0
    name: Can Block
    allow_blocked: true
    goal: Might block
    verify:
      - Check done
    transitions:
      - action: goto
        target_step: 1
  - id: 1
    name: Cannot Block
    goal: Must not block
    verify:
      - Final check
    transitions:
      - action: complete
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}

pub fn three_step_lifecycle_workflow() -> Workflow {
    let yaml = r#"
id: lifecycle
name: Lifecycle
description: Full lifecycle
steps:
  - id: 0
    name: Setup
    goal: Set up
    verify:
      - Setup done
    jump:
      - id: ready
        prompt: Ready?
        type: boolean
    transitions:
      - when:
          ready: true
        action: goto
        target_step: 1
      - action: complete
  - id: 1
    name: Execute
    goal: Do work
    verify:
      - Work done
    jump:
      - id: done
        prompt: Done?
        type: boolean
    transitions:
      - when:
          done: true
        action: goto
        target_step: 2
      - action: complete
  - id: 2
    name: Cleanup
    goal: Clean up
    verify:
      - Cleanup done
    transitions:
      - action: complete
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}
pub fn enum_jump_workflow() -> Workflow {
    let yaml = r#"
id: enum-jump
name: Enum Jump
description: Workflow with enum jump questions
steps:
  - id: 0
    name: Decide
    goal: Choose a strategy
    verify:
      - Strategy chosen
    jump:
      - id: strategy
        prompt: Which strategy?
        type: enum
        options:
          - fast
          - slow
    transitions:
      - when:
          strategy: fast
        action: goto
        target_step: 1
      - when:
          strategy: slow
        action: goto
        target_step: 2
      - action: complete
  - id: 1
    name: Fast Path
    goal: Do it fast
    verify:
      - Fast done
    transitions:
      - action: complete
  - id: 2
    name: Slow Path
    goal: Do it slow
    verify:
      - Slow done
    transitions:
      - action: complete
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}

// -----------------------------------------------------------------------
// Struct-constructed fixtures: intentionally invalid workflows for testing
// engine runtime defense (bypass YAML validation).
// -----------------------------------------------------------------------

/// A single-step workflow with no transitions and empty verify.
/// Tests the verify-exhaust → blocked → resolve → re-verify path.
pub fn no_transitions_workflow() -> Workflow {
    Workflow {
        id: "no-trans".into(),
        name: "No Trans".into(),
        description: "".into(),
        version: None,
        allow_blocked: false,
        verify_retry_limit: 3,
        step_data_schema: serde_yaml::Value::Null,
        steps: vec![Step {
            id: 0,
            name: "Only".into(),
            allow_blocked: None,
            goal: "No transitions".into(),
            verify: vec![],
            jump: vec![],
            transitions: vec![],
        }],
    }
}

/// Two-step workflow: step 0 has a default goto to step 1;
/// step 1 has no transitions and empty verify.
pub fn goto_then_no_transitions_workflow() -> Workflow {
    Workflow {
        id: "goto-block".into(),
        name: "Goto Block".into(),
        description: "".into(),
        version: None,
        allow_blocked: false,
        verify_retry_limit: 3,
        step_data_schema: serde_yaml::Value::Null,
        steps: vec![
            Step {
                id: 0,
                name: "First".into(),
                allow_blocked: None,
                goal: "Go to step 1".into(),
                verify: vec![],
                jump: vec![],
                transitions: vec![Transition {
                    when: None,
                    action: "goto".into(),
                    target_step: Some(1),
                }],
            },
            Step {
                id: 1,
                name: "Second".into(),
                allow_blocked: None,
                goal: "No transitions".into(),
                verify: vec![],
                jump: vec![],
                transitions: vec![],
            },
        ],
    }
}
