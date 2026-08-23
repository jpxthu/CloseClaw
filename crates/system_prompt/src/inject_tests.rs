//! Unit tests for `inject::build_dynamic_sections`.
//!
//! Tests ordering guarantees and content correctness of the dynamic
//! section list.
//!
//! Covers Step 1.6 test dimensions:
//! - Boundary: Normal→Normal produces no mode instruction
//! - Ordering: ChannelContext → WorkingDirectory → ModeInstruction → GitStatus
//! - Mode transition: §6 transition prompts are injected when mode_transition is set

use super::inject::{build_dynamic_sections, DynamicSectionsParams};
use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::system_prompt::ModeTransition;
use closeclaw_execution::PlanPath;
use closeclaw_gateway::session_handler::MessageMetadata;
use std::collections::HashSet;

fn make_meta(sender: &str, channel: &str, ts: i64) -> MessageMetadata {
    MessageMetadata {
        sender_id: sender.to_string(),
        channel: channel.to_string(),
        timestamp: ts,
        chat_name: String::new(),
        trace_id: None,
        session_key: None,
    }
}

/// Helper: build a `DynamicSectionsParams` with defaults for optional fields.
fn make_params(meta: &MessageMetadata, session_mode: SessionMode) -> DynamicSectionsParams<'_> {
    DynamicSectionsParams {
        meta,
        workdir_path: None,
        session_mode,
        explicit_plan_path: None,
        user_input: None,
        is_compacted: false,
        is_sub_agent: false,
        is_git_status_enabled: false,
        mode_transition: None,
    }
}

// ── Boundary: Normal→Normal ─────────────────────────────────────────────────

/// Even Normal mode should not produce a ModeInstruction section.
#[test]
fn test_normal_mode_no_mode_instruction() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    assert!(
        !sections.iter().any(|s| s.name() == "mode_instruction"),
        "Normal mode should not inject ModeInstruction"
    );
}

// ── Ordering ────────────────────────────────────────────────────────────────

/// ChannelContext appears before ModeInstruction.
/// ChannelContext is always first, followed by ModeInstruction when active.
#[test]
fn test_ordering_without_transition_channel_then_mode_instruction() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&make_params(&meta, SessionMode::Auto));

    let channel_idx = sections.iter().position(|s| s.name() == "channel_context");
    let mode_idx = sections.iter().position(|s| s.name() == "mode_instruction");

    assert!(channel_idx.is_some());
    assert!(mode_idx.is_some());
    assert!(
        channel_idx.unwrap() < mode_idx.unwrap(),
        "ChannelContext should come before ModeInstruction"
    );
}

// ── ModeInstruction basic tests (migrated from inject.rs) ──────────────────

/// Plan mode injects a ModeInstruction section with "Plan" content.
#[test]
fn test_plan_mode_injects_instruction() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        explicit_plan_path: Some(PlanPath::Standard),
        ..make_params(&meta, SessionMode::Plan)
    });
    let mode_sec = sections.iter().find(|s| s.name() == "mode_instruction");
    assert!(
        mode_sec.is_some(),
        "Plan mode should inject ModeInstruction"
    );
    let rendered = mode_sec.unwrap().render();
    assert!(rendered.contains("Plan"));
}

/// Auto mode injects a ModeInstruction section with "Auto" content.
#[test]
fn test_auto_mode_injects_instruction() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&make_params(&meta, SessionMode::Auto));
    let mode_sec = sections.iter().find(|s| s.name() == "mode_instruction");
    assert!(
        mode_sec.is_some(),
        "Auto mode should inject ModeInstruction"
    );
    let rendered = mode_sec.unwrap().render();
    assert!(rendered.contains("Auto"));
}

/// Plan mode with explicit Standard path renders "Standard Path".
#[test]
fn test_plan_mode_explicit_standard_path() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        explicit_plan_path: Some(PlanPath::Standard),
        ..make_params(&meta, SessionMode::Plan)
    });
    let rendered = sections
        .iter()
        .find(|s| s.name() == "mode_instruction")
        .unwrap()
        .render();
    assert!(rendered.contains("Standard Path"));
    assert!(!rendered.contains("Interview Path"));
}

/// Plan mode with explicit Interview path renders "Interview Path".
#[test]
fn test_plan_mode_explicit_interview_path() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        explicit_plan_path: Some(PlanPath::Interview),
        ..make_params(&meta, SessionMode::Plan)
    });
    let rendered = sections
        .iter()
        .find(|s| s.name() == "mode_instruction")
        .unwrap()
        .render();
    assert!(rendered.contains("Interview Path"));
    assert!(!rendered.contains("Standard Path"));
}

/// Plan mode auto-analysis with a clear bug-fix input selects Standard Path.
#[test]
fn test_plan_mode_auto_analysis_clear_input() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        user_input: Some(
            "Fix the bug in crates/system_prompt/src/sections.rs — should return None",
        ),
        ..make_params(&meta, SessionMode::Plan)
    });
    let rendered = sections
        .iter()
        .find(|s| s.name() == "mode_instruction")
        .unwrap()
        .render();
    assert!(rendered.contains("Standard Path"));
    assert!(!rendered.contains("Interview Path"));
}

/// Plan mode auto-analysis with an ambiguous input selects Interview Path.
#[test]
fn test_plan_mode_auto_analysis_ambiguous_input() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        user_input: Some("Make it better"),
        ..make_params(&meta, SessionMode::Plan)
    });
    let rendered = sections
        .iter()
        .find(|s| s.name() == "mode_instruction")
        .unwrap()
        .render();
    assert!(rendered.contains("Interview Path"));
    assert!(!rendered.contains("Standard Path"));
}

// ── ChannelContext chat_name tests ────────────────────────────────────────────

/// ChannelContext must render the actual chat_name from metadata,
/// not the channel type string.
#[test]
fn test_channel_context_renders_actual_chat_name() {
    let meta = MessageMetadata {
        sender_id: "ou_sender1".to_string(),
        channel: "feishu".to_string(),
        timestamp: 1700000000,
        chat_name: "Dev Team".to_string(),
        trace_id: None,
        session_key: None,
    };
    let sections = build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let channel_ctx = sections
        .iter()
        .find(|s| s.name() == "channel_context")
        .expect("ChannelContext section should be present");
    let rendered = channel_ctx.render();
    assert!(
        rendered.contains("chat_name: Dev Team"),
        "ChannelContext should render the actual chat_name, got: {}",
        rendered
    );
    assert!(
        !rendered.contains("chat_name: feishu"),
        "ChannelContext must NOT render the channel type as chat_name"
    );
}

/// When chat_name is empty, ChannelContext must render the empty string
/// gracefully (fallback path).
#[test]
fn test_channel_context_empty_chat_name_fallback() {
    let meta = MessageMetadata {
        sender_id: "ou_sender1".to_string(),
        channel: "feishu".to_string(),
        timestamp: 1700000000,
        chat_name: String::new(),
        trace_id: None,
        session_key: None,
    };
    let sections = build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let channel_ctx = sections
        .iter()
        .find(|s| s.name() == "channel_context")
        .expect("ChannelContext section should be present");
    let rendered = channel_ctx.render();
    assert!(
        rendered.contains("chat_name: "),
        "ChannelContext should render chat_name even when empty, got: {}",
        rendered
    );
    assert!(
        !rendered.contains("chat_name: feishu"),
        "Empty chat_name fallback must NOT fall back to channel type"
    );
}

/// ChannelContext with different channel types always uses chat_name,
/// regardless of which IM platform.
#[test]
fn test_channel_context_chat_name_independent_of_channel_type() {
    let channels = ["feishu", "telegram", "discord", "slack"];
    for ch in channels {
        let meta = MessageMetadata {
            sender_id: "u1".to_string(),
            channel: ch.to_string(),
            timestamp: 0,
            chat_name: "My Group".to_string(),
            trace_id: None,
            session_key: None,
        };
        let sections = build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
        let rendered = sections
            .iter()
            .find(|s| s.name() == "channel_context")
            .unwrap()
            .render();
        assert!(
            rendered.contains("chat_name: My Group"),
            "Channel '{}' should render chat_name, got: {}",
            ch,
            rendered
        );
        assert!(
            !rendered.contains(&format!("chat_name: {}", ch)),
            "Channel '{}' must NOT use channel type as chat_name",
            ch
        );
    }
}

// ── GitStatus config switch tests ──────────────────────────────────────────

/// When `git_status_enabled` is false (default), GitStatus section must
/// NOT be injected even when the workdir is a valid git repository.
#[test]
fn test_git_status_disabled_excludes_section() {
    let meta = make_meta("u", "ch", 0);
    // CARGO_MANIFEST_DIR is a git repo, but git_status is disabled
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        workdir_path: Some(env!("CARGO_MANIFEST_DIR")),
        is_git_status_enabled: false,
        ..make_params(&meta, SessionMode::Normal)
    });
    let has_git_status = sections.iter().any(|s| s.name() == "git_status");
    assert!(
        !has_git_status,
        "git_status_enabled=false must not inject GitStatus even for a git repo"
    );
    // WorkingDirectory should still be present
    let has_workdir = sections.iter().any(|s| s.name() == "working_directory");
    assert!(
        has_workdir,
        "WorkingDirectory should still be injected regardless of git_status"
    );
}

/// When `git_status_enabled` is true and the workdir is a git repo,
/// GitStatus section MUST be injected.
#[test]
fn test_git_status_enabled_includes_section_for_git_repo() {
    let meta = make_meta("u", "ch", 0);
    // CARGO_MANIFEST_DIR is a git repo
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        workdir_path: Some(env!("CARGO_MANIFEST_DIR")),
        is_git_status_enabled: true,
        ..make_params(&meta, SessionMode::Normal)
    });
    let has_git_status = sections.iter().any(|s| s.name() == "git_status");
    assert!(
        has_git_status,
        "git_status_enabled=true should inject GitStatus for a git repo"
    );
}

/// When `git_status_enabled` is true but the workdir is NOT a git repo,
/// GitStatus section must NOT be injected (build_git_status_for returns None).
#[test]
fn test_git_status_enabled_skips_section_for_non_git_repo() {
    let meta = make_meta("u", "ch", 0);
    // /tmp is typically not a git repo
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        workdir_path: Some("/tmp"),
        is_git_status_enabled: true,
        ..make_params(&meta, SessionMode::Normal)
    });
    let has_git_status = sections.iter().any(|s| s.name() == "git_status");
    assert!(
        !has_git_status,
        "git_status_enabled=true should not inject GitStatus for a non-git path"
    );
}

/// When workdir_path is None, GitStatus section must never appear
/// regardless of git_status_enabled setting.
#[test]
fn test_git_status_not_injected_without_workdir() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        workdir_path: None,
        is_git_status_enabled: true,
        ..make_params(&meta, SessionMode::Normal)
    });
    let has_git_status = sections.iter().any(|s| s.name() == "git_status");
    assert!(
        !has_git_status,
        "GitStatus must not appear when workdir_path is None"
    );
}

// ── Dimension 1: Full happy path ─────────────────────────────────────────

/// build_dynamic_sections produces exactly the four section types defined
/// by the design doc when all conditions are met: non-Normal mode (→
/// ModeInstruction), workdir present (→ WorkingDirectory), git status
/// enabled in a git repo (→ GitStatus), and always ChannelContext.
#[test]
fn test_happy_path_all_four_sections() {
    let meta = make_meta("user1", "feishu", 1700000000);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        workdir_path: Some(env!("CARGO_MANIFEST_DIR")),
        is_git_status_enabled: true,
        session_mode: SessionMode::Auto,
        ..make_params(&meta, SessionMode::Auto)
    });
    let names: HashSet<&str> = sections.iter().map(|s| s.name()).collect();
    assert!(names.contains("mode_instruction"));
    assert!(names.contains("channel_context"));
    assert!(names.contains("working_directory"));
    assert!(names.contains("git_status"));
    assert_eq!(names.len(), 4, "exactly four section types expected");
}

/// Section ordering: ChannelContext → WorkingDirectory →
/// ModeInstruction → GitStatus.
#[test]
fn test_happy_path_section_ordering() {
    let meta = make_meta("user1", "feishu", 1700000000);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        workdir_path: Some(env!("CARGO_MANIFEST_DIR")),
        is_git_status_enabled: true,
        session_mode: SessionMode::Plan,
        ..make_params(&meta, SessionMode::Plan)
    });
    let names: Vec<&str> = sections.iter().map(|s| s.name()).collect();
    assert_eq!(names[0], "channel_context");
    assert_eq!(names[1], "working_directory");
    assert_eq!(names[2], "mode_instruction");
    assert_eq!(names[3], "git_status");
}

// ── Dimension 3: No workdir → no WorkingDirectory, no GitStatus ─────────

/// When workdir_path is None, neither WorkingDirectory nor GitStatus
/// should appear, regardless of session mode or git status flag.
#[test]
fn test_no_workdir_excludes_working_directory_and_git_status() {
    let meta = make_meta("user1", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        workdir_path: None,
        is_git_status_enabled: true,
        session_mode: SessionMode::Auto,
        ..make_params(&meta, SessionMode::Auto)
    });
    let names: HashSet<&str> = sections.iter().map(|s| s.name()).collect();
    assert!(!names.contains("working_directory"));
    assert!(!names.contains("git_status"));
    // ModeInstruction and ChannelContext should still be present
    assert!(names.contains("mode_instruction"));
    assert!(names.contains("channel_context"));
}

// ── Dimension 4: GitStatus disabled → no GitStatus ──────────────────────

/// With workdir present but is_git_status_enabled=false, WorkingDirectory
/// is injected but GitStatus is not, even in a valid git repo.
#[test]
fn test_git_status_disabled_with_workdir_no_git_section() {
    let meta = make_meta("user1", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        workdir_path: Some(env!("CARGO_MANIFEST_DIR")),
        is_git_status_enabled: false,
        ..make_params(&meta, SessionMode::Normal)
    });
    let names: HashSet<&str> = sections.iter().map(|s| s.name()).collect();
    assert!(names.contains("working_directory"));
    assert!(!names.contains("git_status"));
}

// ── Negative: no removed Section types appear ────────────────────────────

/// Verify build_dynamic_sections never produces SessionState
/// or AppendSection (removed from dynamic layer).
/// ModeTransition IS now a valid section type (design doc §6).
#[test]
fn test_no_removed_section_types() {
    let meta = make_meta("user1", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        workdir_path: Some(env!("CARGO_MANIFEST_DIR")),
        is_git_status_enabled: true,
        session_mode: SessionMode::Plan,
        ..make_params(&meta, SessionMode::Plan)
    });
    let names: HashSet<&str> = sections.iter().map(|s| s.name()).collect();
    assert!(!names.contains("session_state"));
    assert!(!names.contains("append_section"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Mode Transition Tests — design doc §6
// ═══════════════════════════════════════════════════════════════════════════

/// Plan Mode re-entry injects a ModeTransition section with §6 re-entry content.
#[test]
fn test_plan_mode_reentry_injects_transition() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        session_mode: SessionMode::Plan,
        mode_transition: Some(ModeTransition::PlanModeReentry),
        ..make_params(&meta, SessionMode::Plan)
    });
    let transition = sections.iter().find(|s| s.name() == "mode_transition");
    assert!(
        transition.is_some(),
        "Plan Mode re-entry should inject ModeTransition section"
    );
    let rendered = transition.unwrap().render();
    assert!(
        rendered.contains("Re-entering Plan Mode"),
        "Should contain re-entry heading, got: {}",
        rendered
    );
    assert!(
        rendered.contains("Read the existing plan file"),
        "Should contain re-entry instructions"
    );
}

/// Plan Mode exit injects a ModeTransition section with §6 exit content.
#[test]
fn test_plan_mode_exit_injects_transition() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        session_mode: SessionMode::Normal,
        mode_transition: Some(ModeTransition::PlanModeExit),
        ..make_params(&meta, SessionMode::Normal)
    });
    let transition = sections.iter().find(|s| s.name() == "mode_transition");
    assert!(
        transition.is_some(),
        "Plan Mode exit should inject ModeTransition section"
    );
    let rendered = transition.unwrap().render();
    assert!(
        rendered.contains("Exited Plan Mode"),
        "Should contain exit heading, got: {}",
        rendered
    );
}

/// Auto Mode exit injects a ModeTransition section with §6 auto exit content.
#[test]
fn test_auto_mode_exit_injects_transition() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        session_mode: SessionMode::Normal,
        mode_transition: Some(ModeTransition::AutoModeExit),
        ..make_params(&meta, SessionMode::Normal)
    });
    let transition = sections.iter().find(|s| s.name() == "mode_transition");
    assert!(
        transition.is_some(),
        "Auto Mode exit should inject ModeTransition section"
    );
    let rendered = transition.unwrap().render();
    assert!(
        rendered.contains("Exited Auto Mode"),
        "Should contain auto exit heading, got: {}",
        rendered
    );
}

/// No mode transition → no ModeTransition section.
#[test]
fn test_no_mode_transition_no_section() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&make_params(&meta, SessionMode::Plan));
    assert!(
        !sections.iter().any(|s| s.name() == "mode_transition"),
        "Without mode_transition, no ModeTransition section should appear"
    );
}

/// Mode transition content matches design doc §6 verbatim.
#[test]
fn test_mode_transition_content_matches_design_doc() {
    let meta = make_meta("u", "ch", 0);

    // Plan re-entry
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        session_mode: SessionMode::Plan,
        mode_transition: Some(ModeTransition::PlanModeReentry),
        ..make_params(&meta, SessionMode::Plan)
    });
    let rendered = sections
        .iter()
        .find(|s| s.name() == "mode_transition")
        .unwrap()
        .render();
    assert!(rendered.contains("Treat this as a fresh planning session."));
    assert!(rendered.contains("Do not assume the existing"));

    // Plan exit
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        session_mode: SessionMode::Normal,
        mode_transition: Some(ModeTransition::PlanModeExit),
        ..make_params(&meta, SessionMode::Normal)
    });
    let rendered = sections
        .iter()
        .find(|s| s.name() == "mode_transition")
        .unwrap()
        .render();
    assert!(rendered.contains("You can now make edits, run tools, and take"));
    assert!(rendered.contains("Reference the plan file if needed."));

    // Auto exit
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        session_mode: SessionMode::Normal,
        mode_transition: Some(ModeTransition::AutoModeExit),
        ..make_params(&meta, SessionMode::Normal)
    });
    let rendered = sections
        .iter()
        .find(|s| s.name() == "mode_transition")
        .unwrap()
        .render();
    assert!(rendered.contains("The user may now want to interact more"));
    assert!(rendered.contains("ask clarifying questions when the approach is"));
}

/// Mode transition appears after ModeInstruction in section ordering.
#[test]
fn test_mode_transition_ordering_after_mode_instruction() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        session_mode: SessionMode::Plan,
        mode_transition: Some(ModeTransition::PlanModeReentry),
        ..make_params(&meta, SessionMode::Plan)
    });
    let mode_idx = sections.iter().position(|s| s.name() == "mode_instruction");
    let transition_idx = sections.iter().position(|s| s.name() == "mode_transition");
    assert!(mode_idx.is_some(), "ModeInstruction should be present");
    assert!(transition_idx.is_some(), "ModeTransition should be present");
    assert!(
        mode_idx.unwrap() < transition_idx.unwrap(),
        "ModeInstruction should come before ModeTransition"
    );
}
