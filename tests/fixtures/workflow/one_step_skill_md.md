---
id: test-wf
name: Test WF
description: A test workflow
steps:
  - id: 0
    name: Step 0
    goal: Do first thing
    verify:
      - Check output
    transitions:
      - action: complete
---

Body.
