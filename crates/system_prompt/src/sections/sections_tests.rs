//! Tests for `sections` module.
//!
//! Extracted from the inline `#[cfg(test)] mod tests` in `sections.rs`
//! to keep that file focused on rendering logic.

use super::*;
use tempfile::tempdir;

#[test]
fn test_section_render_role() {
    let s = Section::RoleSection("You are a helpful assistant.".to_string());
    let rendered = s.render();
    assert!(rendered.contains("## Role"));
    assert!(rendered.contains("You are a helpful assistant"));
    assert!(s.is_cacheable());
}

#[test]
fn test_section_render_channel_context() {
    let s = Section::ChannelContext {
        chat_name: "test-chat".to_string(),
        sender_id: "ou_123".to_string(),
        timestamp: "2026-04-10T15:00:00+08:00".to_string(),
    };
    let rendered = s.render();
    assert!(rendered.contains("chat_name: test-chat"));
    assert!(rendered.contains("sender_id: ou_123"));
    assert!(!s.is_cacheable());
}

#[test]
fn test_section_render_session_state() {
    let s = Section::SessionState {
        pending_tasks: vec!["task1".to_string(), "task2".to_string()],
    };
    let rendered = s.render();
    assert!(rendered.contains("task1"));
    assert!(!s.is_cacheable());
}

#[test]
fn test_invalidate_section() {
    put_cached_section("test_section", "old content".to_string(), Some(100));
    assert_eq!(
        get_cached_section("test_section", Some(100)),
        Some("old content".to_string())
    );

    invalidate_section("test_section");
    assert_eq!(get_cached_section("test_section", Some(100)), None);
}

#[test]
fn test_cache_stale_on_mtime_change() {
    put_cached_section("file_section", "v1".to_string(), Some(100));
    // Same mtime → cache hit
    assert_eq!(
        get_cached_section("file_section", Some(100)),
        Some("v1".to_string())
    );
    // Different mtime → cache stale
    assert_eq!(get_cached_section("file_section", Some(200)), None);
}

#[test]
fn test_load_cached_file_section_fresh() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test.txt");
    std::fs::write(&file_path, "hello world").unwrap();

    // First load — cache miss, should read from file
    let result = load_cached_file_section("test", &file_path);
    assert_eq!(result, Some("hello world".to_string()));

    // Second load — cache hit, same content
    let result2 = load_cached_file_section("test", &file_path);
    assert_eq!(result2, Some("hello world".to_string()));

    // Modify file — cache should be stale
    // Sleep 1s to ensure mtime changes (filesystem mtime resolution is 1s)
    std::thread::sleep(std::time::Duration::from_secs(1));
    std::fs::write(&file_path, "updated content").unwrap();
    let result3 = load_cached_file_section("test", &file_path);
    assert_eq!(result3, Some("updated content".to_string()));
}

#[test]
fn test_skill_listing_section_render() {
    let s = Section::SkillListingSection(
        "- **foo**: desc — use when needed
- **bar**: desc"
            .to_string(),
    );
    let rendered = s.render();
    assert!(rendered.starts_with("## Available Skills\n\n"));
    assert!(rendered.contains("**foo**"));
    assert!(rendered.contains(" — use when needed"));
    // Empty renders empty
    let empty = Section::SkillListingSection(String::new());
    assert_eq!(empty.render(), "");
}

#[test]
fn test_git_status_render() {
    let s = Section::GitStatus("On branch master\n?? file.txt".to_string());
    let rendered = s.render();
    assert!(rendered.contains("## Git Status"));
    assert!(rendered.contains("On branch master"));
    assert!(!s.is_cacheable());
}

#[test]
fn test_working_directory_section() {
    let s = Section::WorkingDirectory("/home/user/.closeclaw/workspaces/agent1/user1/".to_string());
    assert!(!s.is_cacheable());
    assert_eq!(s.name(), "working_directory");
    let rendered = s.render();
    assert!(rendered.contains("## Working Directory"));
    assert!(rendered.contains("~/agent1/user1/"));
    assert!(!rendered.contains(".closeclaw"));
}

#[test]
fn test_sanitize_workdir_path() {
    assert_eq!(
        sanitize_workdir_path("/home/user/.closeclaw/workspaces/a/u/"),
        "~/a/u/"
    );
    assert_eq!(
        sanitize_workdir_path("/some/random/path"),
        "/some/random/path"
    );
    assert_eq!(sanitize_workdir_path(""), "");
}

#[test]
fn test_invalidate_skill_listing() {
    // Pre-populate the skill_listing cache with known content
    put_cached_section("skill_listing", "old skill content".to_string(), Some(999));
    // Verify it's cached
    assert_eq!(
        get_cached_section("skill_listing", Some(999)),
        Some("old skill content".to_string())
    );

    // Invalidate via the public API
    invalidate_skill_listing();

    // Cache should be cleared
    assert_eq!(get_cached_section("skill_listing", Some(999)), None);
}

// -----------------------------------------------------------------------
// Coverage for all remaining Section variants after WorkspaceSection removal
// -----------------------------------------------------------------------

#[test]
fn test_section_variants_name_and_cacheable() {
    // Declare sections as owned values first to avoid temporary drops.
    let sections: Vec<Section> = vec![
        Section::RoleSection("r".into()),
        Section::ToolsSection("t".into()),
        Section::MemorySection("m".into()),
        Section::HeartbeatSection("h".into()),
        Section::SkillListingSection("s".into()),
        Section::ChannelContext {
            chat_name: "c".into(),
            sender_id: "s".into(),
            timestamp: "t".into(),
        },
        Section::SessionState {
            pending_tasks: vec![],
        },
        Section::AppendSection("a".into()),
        Section::GitStatus("g".into()),
        Section::WorkingDirectory("/x/workspaces/a/b/".into()),
        Section::ModeInstruction {
            mode: SessionMode::Plan,
            plan_path: Some(PlanPath::Standard),
            sparse: false,
            sub_agent: false,
        },
        Section::ModeTransition {
            transition: ModeTransition::Reentry,
        },
    ];
    let expected: Vec<(&str, bool)> = vec![
        ("role", true),
        ("tools", false),
        ("memory", true),
        ("heartbeat", true),
        ("skill_listing", false),
        ("channel_context", false),
        ("session_state", false),
        ("append", false),
        ("git_status", false),
        ("working_directory", false),
        ("mode_instruction", false),
        ("mode_transition", false),
    ];
    for (sec, (expected_name, expected_cacheable)) in sections.iter().zip(expected.iter()) {
        assert_eq!(sec.name(), *expected_name);
        assert_eq!(sec.is_cacheable(), *expected_cacheable);
    }
}

// -----------------------------------------------------------------------
// ModeInstruction tests
// -----------------------------------------------------------------------

#[test]
fn test_mode_instruction_basics_and_auto() {
    let s = Section::ModeInstruction {
        mode: SessionMode::Normal,
        plan_path: None,
        sparse: false,
        sub_agent: false,
    };
    assert_eq!(s.name(), "mode_instruction");
    assert!(!s.is_cacheable());
    assert_eq!(s.render(), "");
    // Auto mode renders all 6 rules
    let rendered = render_mode_instruction(SessionMode::Auto, None);
    assert!(rendered.contains("Auto Mode Active"));
    assert!(rendered.contains("Execute immediately"));
    assert!(rendered.contains("Minimize interruptions"));
    assert!(rendered.contains("Prefer action over planning"));
    assert!(rendered.contains("Expect course corrections"));
    assert!(rendered.contains("Do not take overly destructive actions"));
    assert!(rendered.contains("Avoid data exfiltration"));
    // Default PlanPath is now Standard
    let s = Section::ModeInstruction {
        mode: SessionMode::Plan,
        plan_path: None,
        sparse: false,
        sub_agent: false,
    };
    assert!(s.render().contains("Standard Path"));
}

#[test]
fn test_mode_instruction_plan_standard_and_interview() {
    let std = Section::ModeInstruction {
        mode: SessionMode::Plan,
        plan_path: Some(PlanPath::Standard),
        sparse: false,
        sub_agent: false,
    };
    let r = std.render();
    assert!(r.contains("## Mode: Plan \u{2014} Standard Path"));
    assert!(r.contains("This supercedes any other instructions"));
    assert!(r.contains("Phase 4: Final Plan"));
    assert!(!r.contains("Phase 5"));
    assert!(!r.contains("Interview Path"));
    let intv = Section::ModeInstruction {
        mode: SessionMode::Plan,
        plan_path: Some(PlanPath::Interview),
        sparse: false,
        sub_agent: false,
    };
    let r = intv.render();
    assert!(r.contains("## Mode: Plan \u{2014} Interview Path"));
    assert!(r.contains("pair-planning"));
    assert!(r.contains("Don't explore exhaustively before engaging the user"));
    assert!(r.contains("Never ask what you could find out by reading the code"));
    assert!(r.contains("When to Converge"));
    assert!(r.contains("The Loop"));
    assert!(!r.contains("Standard Path"));
}

#[test]
fn test_mode_transition_renders_correct_text() {
    let reentry = Section::ModeTransition {
        transition: ModeTransition::Reentry,
    };
    assert_eq!(reentry.name(), "mode_transition");
    assert!(!reentry.is_cacheable());
    let r = reentry.render();
    assert!(r.contains("## Re-entering Plan Mode"));
    assert!(r.contains("returning to plan mode"));
    assert!(r.contains("Read the existing plan file"));

    let exit_plan = Section::ModeTransition {
        transition: ModeTransition::ExitPlan,
    };
    let r = exit_plan.render();
    assert!(r.contains("## Exited Plan Mode"));
    assert!(r.contains("can now make edits, run tools"));

    let exit_auto = Section::ModeTransition {
        transition: ModeTransition::ExitAuto,
    };
    let r = exit_auto.render();
    assert!(r.contains("## Exited Auto Mode"));
    assert!(r.contains("ask clarifying questions"));
}

// -----------------------------------------------------------------------
// Sparse / sub-agent variant tests
// -----------------------------------------------------------------------

#[test]
fn test_mode_instruction_sparse_and_sub_agent() {
    // Plan sparse → Standard Sparse text
    let rendered = render_mode_instruction_with_flags(
        SessionMode::Plan,
        Some(PlanPath::Standard),
        true,
        false,
    );
    assert!(rendered.contains("Plan mode still active"));
    assert!(rendered.contains("Read-only except plan file"));
    // Auto sparse → Auto Sparse text
    let rendered = render_mode_instruction_with_flags(SessionMode::Auto, None, true, false);
    assert!(rendered.contains("Auto mode still active"));
    assert!(rendered.contains("Execute autonomously"));
    // Sub-agent → Sub-agent Sparse text
    let rendered = render_mode_instruction_with_flags(
        SessionMode::Plan,
        Some(PlanPath::Standard),
        false,
        true,
    );
    assert!(rendered.contains("Plan mode is active"));
    assert!(rendered.contains("READ-ONLY actions"));
    // sub_agent takes precedence over sparse
    let rendered =
        render_mode_instruction_with_flags(SessionMode::Plan, Some(PlanPath::Standard), true, true);
    assert!(rendered.contains("READ-ONLY actions"));
    assert!(!rendered.contains("Plan mode still active"));
}

// -----------------------------------------------------------------------
// Bug fix verification: Section::render() ModeInstruction with flags
// -----------------------------------------------------------------------

#[test]
fn test_section_render_mode_instruction_uses_flags() {
    // When sparse=true, render() should output Standard Sparse text
    let s = Section::ModeInstruction {
        mode: SessionMode::Plan,
        plan_path: Some(PlanPath::Standard),
        sparse: true,
        sub_agent: false,
    };
    let r = s.render();
    assert!(
        r.contains("Plan mode still active"),
        "Expected Standard Sparse text from render(), got: {}",
        r
    );

    // When sub_agent=true, render() should output Sub-agent Sparse text
    let s = Section::ModeInstruction {
        mode: SessionMode::Plan,
        plan_path: Some(PlanPath::Standard),
        sparse: false,
        sub_agent: true,
    };
    let r = s.render();
    assert!(
        r.contains("Plan mode is active"),
        "Expected Sub-agent Sparse text from render(), got: {}",
        r
    );
}

// ── Gap 4: Interview Path global constraint ─────────────────────────────

/// Verify render_interview_path_instruction() includes PLAN_MODE_CONSTRAINT.
#[test]
fn test_interview_path_includes_plan_mode_constraint() {
    let output = render_interview_path_instruction();
    assert!(
        output.contains(PLAN_MODE_CONSTRAINT),
        "render_interview_path_instruction() must include PLAN_MODE_CONSTRAINT"
    );
}

/// Verify render_interview_path_instruction() output format matches
/// "## Mode: Plan — Interview Path\n\n{CONSTRAINT}\n\n{PROMPT}\n".
#[test]
fn test_interview_path_output_format() {
    let output = render_interview_path_instruction();
    let expected = format!(
        "## Mode: Plan \u{2014} Interview Path\n\n{}\n\n{}\n",
        PLAN_MODE_CONSTRAINT, INTERVIEW_PATH_PROMPT
    );
    assert_eq!(output, expected);
}

/// Verify render_standard_path_instruction() and render_interview_path_instruction()
/// use the same PLAN_MODE_CONSTRAINT text.
#[test]
fn test_standard_and_interview_share_plan_mode_constraint() {
    let standard = render_standard_path_instruction();
    let interview = render_interview_path_instruction();
    // Both should contain the same constraint text
    assert!(standard.contains(PLAN_MODE_CONSTRAINT));
    assert!(interview.contains(PLAN_MODE_CONSTRAINT));
    // The constraint text should be identical in both outputs
    let standard_idx = standard.find(PLAN_MODE_CONSTRAINT).unwrap();
    let interview_idx = interview.find(PLAN_MODE_CONSTRAINT).unwrap();
    let standard_slice = &standard[standard_idx..standard_idx + PLAN_MODE_CONSTRAINT.len()];
    let interview_slice = &interview[interview_idx..interview_idx + PLAN_MODE_CONSTRAINT.len()];
    assert_eq!(standard_slice, interview_slice);
}

// ── Gap 1: Sparse injection tests ────────────────────────────────────────

/// Plan Mode + compacted → STANDARD_SPARSE text
#[test]
fn test_sparse_plan_mode_outputs_standard_sparse() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Plan,
        Some(PlanPath::Standard),
        true,  // is_compacted
        false, // is_sub_agent
    );
    assert!(
        output.contains("Plan mode still active"),
        "Plan Mode compacted should output STANDARD_SPARSE, got: {}",
        output
    );
    assert!(output.contains("Read-only except plan file"));
}

/// Auto Mode + compacted → AUTO_MODE_SPARSE text (different from Plan sparse)
#[test]
fn test_sparse_auto_mode_outputs_auto_sparse() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Auto,
        None,  // plan_path irrelevant for Auto
        true,  // is_compacted
        false, // is_sub_agent
    );
    assert!(
        output.contains("Auto mode still active"),
        "Auto Mode compacted should output AUTO_MODE_SPARSE, got: {}",
        output
    );
    assert!(output.contains("Execute autonomously"));
}

/// Plan sparse and Auto sparse produce different output
#[test]
fn test_sparse_plan_and_auto_produce_different_output() {
    let plan_sparse = render_mode_instruction_with_flags(
        SessionMode::Plan,
        Some(PlanPath::Standard),
        true,
        false,
    );
    let auto_sparse = render_mode_instruction_with_flags(SessionMode::Auto, None, true, false);
    assert_ne!(
        plan_sparse, auto_sparse,
        "Plan sparse and Auto sparse should produce different outputs"
    );
    assert!(plan_sparse.contains("Plan mode"));
    assert!(auto_sparse.contains("Auto mode"));
}

/// Not compacted → full prompt (no sparse text)
#[test]
fn test_not_compacted_outputs_full_prompt() {
    let plan_full = render_mode_instruction_with_flags(
        SessionMode::Plan,
        Some(PlanPath::Standard),
        false, // not compacted
        false,
    );
    // Full Plan prompt should contain standard path phases, not sparse
    assert!(plan_full.contains("Phase 1: Initial Understanding"));
    assert!(!plan_full.contains("Plan mode still active"));

    let auto_full = render_mode_instruction_with_flags(
        SessionMode::Auto,
        None,
        false, // not compacted
        false,
    );
    // Full Auto prompt should contain full auto instructions, not sparse
    assert!(auto_full.contains("Auto Mode Active"));
    assert!(!auto_full.contains("Auto mode still active"));
}

// ── Gap 2: Sub-agent injection tests ──────────────────────────────────────

/// is_sub_agent = true → SUBAGENT_SPARSE text
#[test]
fn test_sub_agent_true_outputs_subagent_sparse() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Plan,
        Some(PlanPath::Standard),
        false, // sparse irrelevant when sub_agent is true
        true,  // is_sub_agent
    );
    assert!(
        output.contains("Plan mode is active"),
        "Sub-agent should output SUBAGENT_SPARSE, got: {}",
        output
    );
    assert!(output.contains("READ-ONLY actions"));
    assert!(!output.contains("incremental edits"));
}

/// is_sub_agent = false → normal mode instruction (not sub-agent sparse)
#[test]
fn test_sub_agent_false_outputs_normal_instruction() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Plan,
        Some(PlanPath::Standard),
        false,
        false, // not sub-agent
    );
    assert!(output.contains("Phase 1: Initial Understanding"));
    assert!(!output.contains("incremental edits"));
}

/// Sub-agent takes precedence over sparse
#[test]
fn test_sub_agent_precedence_over_sparse() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Plan,
        Some(PlanPath::Standard),
        true, // compacted
        true, // sub-agent
    );
    assert!(
        output.contains("READ-ONLY actions"),
        "Sub-agent should take precedence over sparse"
    );
    assert!(!output.contains("Plan mode still active"));
}

// ── Step 1.2: Auto Mode sub-agent tests ───────────────────────────────────

/// Auto Mode + sub_agent=true, sparse=false → full Auto Mode prompt
/// (sub_agent flag is ignored in Auto Mode, should not inject SUBAGENT_SPARSE)
#[test]
fn test_auto_mode_sub_agent_true_not_injecting_subagent_sparse() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Auto,
        None,  // plan_path irrelevant for Auto
        false, // not compacted
        true,  // sub-agent
    );
    assert!(
        output.contains("Auto Mode Active"),
        "Auto Mode sub_agent should render full Auto Mode prompt, got: {}",
        output
    );
    assert!(
        !output.contains("Plan mode is active"),
        "Auto Mode sub_agent must NOT contain SUBAGENT_SPARSE text"
    );
    assert!(
        !output.contains("READ-ONLY actions"),
        "Auto Mode sub_agent must NOT contain READ-ONLY constraint"
    );
}

/// Auto Mode + sub_agent=true, sparse=true → Auto Mode sparse text
/// (sub_agent flag is ignored in Auto Mode, should render AUTO_MODE_SPARSE)
#[test]
fn test_auto_mode_sub_agent_true_sparse_outputs_auto_sparse() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Auto,
        None, // plan_path irrelevant for Auto
        true, // compacted/sparse
        true, // sub-agent
    );
    assert!(
        output.contains("Auto mode still active"),
        "Auto Mode sub_agent+sparse should output AUTO_MODE_SPARSE, got: {}",
        output
    );
    assert!(
        !output.contains("Plan mode is active"),
        "Auto Mode sub_agent+sparse must NOT contain SUBAGENT_SPARSE text"
    );
    assert!(
        !output.contains("READ-ONLY actions"),
        "Auto Mode sub_agent+sparse must NOT contain READ-ONLY constraint"
    );
}

/// Normal Mode + sub_agent=true → empty string (Normal Mode has no mode instruction)
#[test]
fn test_normal_mode_sub_agent_true_returns_empty() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Normal,
        None,
        false,
        true, // sub-agent
    );
    assert_eq!(
        output, "",
        "Normal Mode sub_agent should return empty string, got: {}",
        output
    );
}

/// Auto Mode + sub_agent=false → behavior unchanged (regression check)
#[test]
fn test_auto_mode_sub_agent_false_regression() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Auto,
        None,
        false, // not compacted
        false, // not sub-agent
    );
    assert!(
        output.contains("Auto Mode Active"),
        "Auto Mode without sub_agent should render full Auto Mode prompt"
    );
    assert!(!output.contains("Plan mode is active"));
    assert!(!output.contains("READ-ONLY actions"));
}

/// Auto Mode + sub_agent=false, sparse=true → Auto Mode sparse (regression)
#[test]
fn test_auto_mode_sub_agent_false_sparse_regression() {
    let output = render_mode_instruction_with_flags(
        SessionMode::Auto,
        None,
        true,  // compacted/sparse
        false, // not sub-agent
    );
    assert!(
        output.contains("Auto mode still active"),
        "Auto Mode sparse without sub_agent should output AUTO_MODE_SPARSE"
    );
}

// ── ModeTransition section rendering ───────────────────────────────────────

#[test]
fn test_section_render_mode_transition_reentry() {
    let s = Section::ModeTransition {
        transition: ModeTransition::Reentry,
    };
    assert_eq!(s.name(), "mode_transition");
    assert!(!s.is_cacheable());
    let rendered = s.render();
    assert!(rendered.contains("Re-entering Plan Mode"));
    assert!(rendered.contains("Read the existing plan file"));
}

#[test]
fn test_section_render_mode_transition_exit_plan() {
    let s = Section::ModeTransition {
        transition: ModeTransition::ExitPlan,
    };
    let rendered = s.render();
    assert!(rendered.contains("Exited Plan Mode"));
    assert!(rendered.contains("can now make edits"));
}

#[test]
fn test_section_render_mode_transition_exit_auto() {
    let s = Section::ModeTransition {
        transition: ModeTransition::ExitAuto,
    };
    let rendered = s.render();
    assert!(rendered.contains("Exited Auto Mode"));
    assert!(rendered.contains("ask clarifying questions"));
}
