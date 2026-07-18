//! Unit tests for `mode_prompts` constants.
//!
//! Verifies that each prompt constant has correct newline structure,
//! content integrity, and that unchanged constants are unaffected.

use super::mode_prompts::*;

// ---------------------------------------------------------------------------
// Helper: verify a string is multi-line (not a single line of text)
// ---------------------------------------------------------------------------

fn assert_multiline(name: &str, text: &str) {
    let lines = text.lines().count();
    assert!(
        lines > 1,
        "{} should be multi-line but has only {} line(s):\n{:?}",
        name,
        lines,
        &text[..text.len().min(200)]
    );
}

fn assert_newline_after_title(name: &str, text: &str, title: &str) {
    let marker = format!("{}\n", title);
    assert!(
        text.contains(&marker),
        "{}: title '{}' must be followed by a newline, got:\n{:?}",
        name,
        title,
        &text[text.find(title).unwrap_or(0)..]
    );
}

// ===========================================================================
// 1. INTERVIEW_PATH_PROMPT — section 3
// ===========================================================================

#[test]
fn test_interview_path_multiline() {
    assert_multiline("INTERVIEW_PATH_PROMPT", INTERVIEW_PATH_PROMPT);
}

#[test]
fn test_interview_path_titles_have_newlines() {
    let titles = [
        "## Iterative Planning Workflow",
        "### The Loop",
        "### First Turn",
        "### Asking Good Questions",
        "### Plan File Structure",
        "### When to Converge",
        "### Ending Your Turn",
    ];
    for title in &titles {
        assert_newline_after_title("INTERVIEW_PATH_PROMPT", INTERVIEW_PATH_PROMPT, title);
    }
}

#[test]
fn test_interview_path_content_integrity() {
    // First turn body should not be glued to its title
    assert!(
        INTERVIEW_PATH_PROMPT.contains("### First Turn\n\nStart by"),
        "INTERVIEW_PATH_PROMPT: '### First Turn' should be followed by blank line then 'Start by'"
    );
    // "Ask the user" should not be glued to "Step 1."
    assert!(
        INTERVIEW_PATH_PROMPT.contains("step 1.\n"),
        "INTERVIEW_PATH_PROMPT: 'step 1.' should end with newline"
    );
    // Plan File Structure items
    assert!(
        INTERVIEW_PATH_PROMPT.contains("concise enough to scan quickly, but detailed\nenough to"),
        "INTERVIEW_PATH_PROMPT: multi-line item should have newline continuation"
    );
}

#[test]
fn test_interview_path_number_of_newlines() {
    // With proper formatting, this constant should have many \n characters
    // (each title, paragraph, and list item contributes at least one).
    let count = INTERVIEW_PATH_PROMPT.matches('\n').count();
    assert!(
        count > 30,
        "INTERVIEW_PATH_PROMPT should have many newlines, got {}",
        count
    );
}

// ===========================================================================
// 2. AUTO_MODE_PROMPT — section 4
// ===========================================================================

#[test]
fn test_auto_mode_multiline() {
    assert_multiline("AUTO_MODE_PROMPT", AUTO_MODE_PROMPT);
}

#[test]
fn test_auto_mode_title_newline() {
    assert_newline_after_title("AUTO_MODE_PROMPT", AUTO_MODE_PROMPT, "## Auto Mode Active");
}

#[test]
fn test_auto_mode_content_integrity() {
    // Each numbered rule should be on its own logical line
    let rules = [
        "1. Execute immediately",
        "2. Minimize interruptions",
        "3. Prefer action over planning",
        "4. Expect course corrections",
        "5. Do not take overly destructive actions",
        "6. Avoid data exfiltration",
    ];
    for rule in &rules {
        assert!(
            AUTO_MODE_PROMPT.contains(rule),
            "AUTO_MODE_PROMPT: missing rule '{}'",
            rule
        );
    }
    // Verify rules are separated by newlines, not glued together
    assert!(
        AUTO_MODE_PROMPT.contains("proceed on low-risk work.\n"),
        "AUTO_MODE_PROMPT: rule 1 body should end with newline"
    );
}

#[test]
fn test_auto_mode_number_of_newlines() {
    let count = AUTO_MODE_PROMPT.matches('\n').count();
    assert!(
        count > 15,
        "AUTO_MODE_PROMPT should have many newlines, got {}",
        count
    );
}

// ===========================================================================
// 3. AUTO_MODE_SPARSE — section 4 sparse
// ===========================================================================

#[test]
fn test_auto_mode_sparse_multiline() {
    assert_multiline("AUTO_MODE_SPARSE", AUTO_MODE_SPARSE);
}

#[test]
fn test_auto_mode_sparse_content_integrity() {
    // Two sentences should be on separate lines
    assert!(
        AUTO_MODE_SPARSE.contains("Auto mode still active"),
        "AUTO_MODE_SPARSE: missing first sentence"
    );
    assert!(
        AUTO_MODE_SPARSE.contains("Execute autonomously"),
        "AUTO_MODE_SPARSE: missing second sentence"
    );
    // Verify they're on separate lines
    assert!(
        AUTO_MODE_SPARSE.contains("in conversation).\n"),
        "AUTO_MODE_SPARSE: first sentence should end with newline"
    );
}

// ===========================================================================
// 4. STANDARD_SPARSE — section 5
// ===========================================================================

#[test]
fn test_standard_sparse_multiline() {
    assert_multiline("STANDARD_SPARSE", STANDARD_SPARSE);
}

#[test]
fn test_standard_sparse_content_integrity() {
    assert!(
        STANDARD_SPARSE.contains("Plan mode still active"),
        "STANDARD_SPARSE: missing first sentence"
    );
    assert!(
        STANDARD_SPARSE.contains("AskUserQuestion"),
        "STANDARD_SPARSE: missing AskUserQuestion reference"
    );
    // Verify multi-line structure
    assert!(
        STANDARD_SPARSE.contains("in conversation).\n"),
        "STANDARD_SPARSE: first sentence should end with newline"
    );
    assert!(
        STANDARD_SPARSE
            .contains("approval. Never\nask about plan approval via text or AskUserQuestion."),
        "STANDARD_SPARSE: last part should have newline before 'ask about plan approval'"
    );
}

// ===========================================================================
// 5. SUBAGENT_SPARSE — section 5
// ===========================================================================

#[test]
fn test_subagent_sparse_multiline() {
    assert_multiline("SUBAGENT_SPARSE", SUBAGENT_SPARSE);
}

#[test]
fn test_subagent_sparse_content_integrity() {
    assert!(
        SUBAGENT_SPARSE.contains("Plan mode is active"),
        "SUBAGENT_SPARSE: missing opening"
    );
    assert!(
        SUBAGENT_SPARSE.contains("You can read the plan file"),
        "SUBAGENT_SPARSE: missing plan file reference"
    );
    // Verify the "to execute" fix — space between "to" and "execute"
    assert!(
        SUBAGENT_SPARSE.contains("to\nexecute"),
        "SUBAGENT_SPARSE: 'to execute' should be split across lines"
    );
    // Should NOT have the old glue
    assert!(
        !SUBAGENT_SPARSE.contains("to\\execute"),
        "SUBAGENT_SPARSE: should not have 'to\\execute' (no backslash without newline)"
    );
}

#[test]
fn test_subagent_sparse_paragraph_break() {
    // Two logical paragraphs separated by a blank line (\n\n)
    assert!(
        SUBAGENT_SPARSE.contains("the system. Instead, you should:\n\nYou can read"),
        "SUBAGENT_SPARSE: should have paragraph break between two sections"
    );
}

// ===========================================================================
// 6. MODE_REENTRY — section 6 Re-entry
// ===========================================================================

#[test]
fn test_mode_reentry_multiline() {
    assert_multiline("MODE_REENTRY", MODE_REENTRY);
}

#[test]
fn test_mode_reentry_title_newline() {
    assert_newline_after_title("MODE_REENTRY", MODE_REENTRY, "## Re-entering Plan Mode");
}

#[test]
fn test_mode_reentry_content_integrity() {
    // Title → blank line → intro
    assert!(
        MODE_REENTRY.contains("## Re-entering Plan Mode\n\nYou are"),
        "MODE_REENTRY: title should be followed by blank line then intro"
    );
    // "Before proceeding" should be on its own line
    assert!(
        MODE_REENTRY.contains("\nBefore proceeding"),
        "MODE_REENTRY: 'Before proceeding' should start on new line"
    );
    // Numbered steps
    assert!(
        MODE_REENTRY.contains("1. Read the existing"),
        "MODE_REENTRY: missing step 1"
    );
    assert!(
        MODE_REENTRY.contains("2. Evaluate"),
        "MODE_REENTRY: missing step 2"
    );
    assert!(
        MODE_REENTRY.contains("3. Decide"),
        "MODE_REENTRY: missing step 3"
    );
    assert!(
        MODE_REENTRY.contains("4. Always edit"),
        "MODE_REENTRY: missing step 4"
    );
    // Bullet sub-items
    assert!(
        MODE_REENTRY.contains("- Different task"),
        "MODE_REENTRY: missing 'Different task' bullet"
    );
    assert!(
        MODE_REENTRY.contains("- Same task"),
        "MODE_REENTRY: missing 'Same task' bullet"
    );
}

// ===========================================================================
// 7. MODE_EXIT_PLAN — section 6 Exit
// ===========================================================================

#[test]
fn test_mode_exit_plan_multiline() {
    assert_multiline("MODE_EXIT_PLAN", MODE_EXIT_PLAN);
}

#[test]
fn test_mode_exit_plan_title_newline() {
    assert_newline_after_title("MODE_EXIT_PLAN", MODE_EXIT_PLAN, "## Exited Plan Mode");
}

#[test]
fn test_mode_exit_plan_content_integrity() {
    assert!(
        MODE_EXIT_PLAN.contains("## Exited Plan Mode\n\nYou have exited"),
        "MODE_EXIT_PLAN: title should be followed by blank line then body"
    );
    assert!(
        MODE_EXIT_PLAN.contains("can now make edits"),
        "MODE_EXIT_PLAN: missing expected text"
    );
}

// ===========================================================================
// 8. MODE_EXIT_AUTO — section 6 Auto Mode Exit
// ===========================================================================

#[test]
fn test_mode_exit_auto_multiline() {
    assert_multiline("MODE_EXIT_AUTO", MODE_EXIT_AUTO);
}

#[test]
fn test_mode_exit_auto_title_newline() {
    assert_newline_after_title("MODE_EXIT_AUTO", MODE_EXIT_AUTO, "## Exited Auto Mode");
}

#[test]
fn test_mode_exit_auto_content_integrity() {
    assert!(
        MODE_EXIT_AUTO.contains("## Exited Auto Mode\n\nYou have exited"),
        "MODE_EXIT_AUTO: title should be followed by blank line then body"
    );
    assert!(
        MODE_EXIT_AUTO.contains("ask clarifying questions"),
        "MODE_EXIT_AUTO: missing expected text"
    );
}

// ===========================================================================
// Regression: PLAN_MODE_CONSTRAINT and STANDARD_PATH_PHASES unchanged
// ===========================================================================

/// Known-good snapshot of PLAN_MODE_CONSTRAINT (should never change).
const PLAN_MODE_CONSTRAINT_SNAPSHOT: &str = "\
Plan mode is active. The user indicated that \
they do not want you to execute yet — \
you MUST NOT make any edits (with the \
exception of the plan file mentioned below), \
run any non-readonly tools (including changing \
configs or making commits), or otherwise make \
any changes to the system. This supercedes \
any other instructions you have received.";

/// Known-good snapshot of STANDARD_PATH_PHASES first & last lines.
const STANDARD_PATH_PHASES_START: &str = "\
### Phase 1: Initial Understanding";
const STANDARD_PATH_PHASES_END: &str = "test the changes end-to-end.\n";

#[test]
fn test_plan_mode_constraint_regression() {
    assert!(
        PLAN_MODE_CONSTRAINT == PLAN_MODE_CONSTRAINT_SNAPSHOT,
        "PLAN_MODE_CONSTRAINT must not change — regression detected"
    );
}

#[test]
fn test_standard_path_phases_regression() {
    assert!(
        STANDARD_PATH_PHASES.starts_with(STANDARD_PATH_PHASES_START),
        "STANDARD_PATH_PHASES: start changed — regression detected"
    );
    assert!(
        STANDARD_PATH_PHASES.ends_with(STANDARD_PATH_PHASES_END),
        "STANDARD_PATH_PHASES: end changed — regression detected"
    );
    // Should contain all 4 phases
    for phase in &[
        "Phase 1: Initial Understanding",
        "Phase 2: Design",
        "Phase 3: Review",
        "Phase 4: Final Plan",
    ] {
        assert!(
            STANDARD_PATH_PHASES.contains(phase),
            "STANDARD_PATH_PHASES: missing '{}'",
            phase
        );
    }
}

// ===========================================================================
// Title format: every ## / ### header must be followed by \n
// ===========================================================================

/// Collect all markdown headers from a prompt constant and verify they
/// are each followed by a newline (not glued to the next word).
fn assert_all_titles_have_newlines(name: &str, text: &str) {
    for line in text.lines() {
        if line.starts_with('#') {
            assert!(
                text.contains(&format!("{}\n", line)),
                "{}: header '{}' must be followed by newline",
                name,
                line
            );
        }
    }
}

#[test]
fn test_all_interview_titles_format() {
    assert_all_titles_have_newlines("INTERVIEW_PATH_PROMPT", INTERVIEW_PATH_PROMPT);
}

#[test]
fn test_all_auto_mode_titles_format() {
    assert_all_titles_have_newlines("AUTO_MODE_PROMPT", AUTO_MODE_PROMPT);
}

#[test]
fn test_all_mode_reentry_titles_format() {
    assert_all_titles_have_newlines("MODE_REENTRY", MODE_REENTRY);
}

#[test]
fn test_all_mode_exit_plan_titles_format() {
    assert_all_titles_have_newlines("MODE_EXIT_PLAN", MODE_EXIT_PLAN);
}

#[test]
fn test_all_mode_exit_auto_titles_format() {
    assert_all_titles_have_newlines("MODE_EXIT_AUTO", MODE_EXIT_AUTO);
}
