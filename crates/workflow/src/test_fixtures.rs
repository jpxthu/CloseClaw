//! Shared test fixtures for workflow engine tests.

use crate::definition::Workflow;

pub fn simple_workflow() -> Workflow {
    let yaml = r#"
id: simple
name: Simple
description: Single step workflow
steps:
  - id: 0
    name: Only Step
    goal: Do the thing
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
    transitions:
      - action: goto
        target_step: 1
  - id: 1
    name: Second
    goal: Step two
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
    transitions:
      - action: goto
        target_step: 1
  - id: 1
    name: Cannot Block
    goal: Must not block
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
    transitions:
      - action: complete
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}

pub fn conditional_only_workflow() -> Workflow {
    let yaml = r#"
id: conditional-only
name: Conditional Only
description: No default transitions
steps:
  - id: 0
    name: Decide
    goal: Choose
    jump:
      - id: go_next
        prompt: Go?
        type: boolean
    transitions:
      - when:
          go_next: true
        action: goto
        target_step: 1
  - id: 1
    name: End
    goal: Done
    transitions:
      - action: complete
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}

pub fn goto_to_blockable_workflow() -> Workflow {
    let yaml = r#"
id: goto-block
name: Goto Block
description: Goto then block
steps:
  - id: 0
    name: First
    goal: Go to step 1
    transitions:
      - action: goto
        target_step: 1
  - id: 1
    name: Second
    goal: Will block
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
    transitions:
      - action: complete
  - id: 2
    name: Slow Path
    goal: Do it slow
    transitions:
      - action: complete
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}
