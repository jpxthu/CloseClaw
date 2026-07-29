//! Unit tests for `inject::build_dynamic_sections`.
//!
//! Tests the one-shot injection of `ModeTransition` sections and ordering
//! guarantees in the dynamic section list.
//!
//! Covers Step 1.6 test dimensions:
//! - One-shot injection: transition appears exactly once per set+take cycle
//! - Repeated transitions: consecutive mode switches each produce one transition
//! - Boundary: Normal→Normal produces no transition
//! - Ordering: ModeInstruction → ModeTransition → ChannelContext

use super::inject::{build_dynamic_sections, DynamicSectionsParams};
use closeclaw_common::{ModeTransition, PlanPath, SessionMode};
use closeclaw_gateway::session_handler::MessageMetadata;

fn make_meta(sender: &str, channel: &str, ts: i64) -> MessageMetadata {
    MessageMetadata {
        sender_id: sender.to_string(),
        channel: channel.to_string(),
        timestamp: ts,
        chat_name: String::new(),
    }
}

/// Helper: build a `DynamicSectionsParams` with defaults for optional fields.
fn make_params(meta: &MessageMetadata, session_mode: SessionMode) -> DynamicSectionsParams<'_> {
    DynamicSectionsParams {
        meta,
        workdir_path: None,
        system_appends: &[],
        session_timestamp: None,
        session_mode,
        explicit_plan_path: None,
        user_input: None,
        pending_mode_transition: None,
        is_compacted: false,
        is_sub_agent: false,
        is_git_status_enabled: false,
    }
}

// ── One-shot injection ──────────────────────────────────────────────────────

/// After a ModeTransition is injected once, the next build with `None`
/// must NOT contain a transition section.
#[test]
fn test_one_shot_injection_transition_appears_exactly_once() {
    let meta = make_meta("u", "ch", 0);

    // First build — with ExitPlan transition
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        pending_mode_transition: Some(ModeTransition::ExitPlan),
        ..make_params(&meta, SessionMode::Auto)
    });
    let has_transition = sections.iter().any(|s| s.name() == "mode_transition");
    assert!(has_transition, "first build should include transition");

    // Second build — without transition (simulates take happened)
    let sections2 = build_dynamic_sections(&make_params(&meta, SessionMode::Auto));
    let has_transition2 = sections2.iter().any(|s| s.name() == "mode_transition");
    assert!(
        !has_transition2,
        "second build should NOT include transition (one-shot)"
    );
}

// ── Repeated transitions ────────────────────────────────────────────────────

/// Consecutive mode switches each produce exactly one transition section.
#[test]
fn test_repeated_transitions_each_produces_one() {
    let meta = make_meta("u", "ch", 0);

    // Switch 1: ExitPlan
    let s1 = build_dynamic_sections(&DynamicSectionsParams {
        pending_mode_transition: Some(ModeTransition::ExitPlan),
        ..make_params(&meta, SessionMode::Auto)
    });
    let t1 = s1.iter().find(|s| s.name() == "mode_transition").unwrap();
    assert!(t1.render().contains("Exited Plan Mode"));

    // Switch 2: Reentry
    let s2 = build_dynamic_sections(&DynamicSectionsParams {
        explicit_plan_path: Some(PlanPath::Standard),
        pending_mode_transition: Some(ModeTransition::Reentry),
        ..make_params(&meta, SessionMode::Plan)
    });
    let t2 = s2.iter().find(|s| s.name() == "mode_transition").unwrap();
    assert!(t2.render().contains("Re-entering Plan Mode"));

    // Switch 3: ExitAuto
    let s3 = build_dynamic_sections(&DynamicSectionsParams {
        pending_mode_transition: Some(ModeTransition::ExitAuto),
        ..make_params(&meta, SessionMode::Normal)
    });
    let t3 = s3.iter().find(|s| s.name() == "mode_transition").unwrap();
    assert!(t3.render().contains("Exited Auto Mode"));
}

// ── Boundary: Normal→Normal ─────────────────────────────────────────────────

/// Staying in Normal mode with no transition produces no ModeTransition section.
#[test]
fn test_normal_to_normal_no_transition() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let has_transition = sections.iter().any(|s| s.name() == "mode_transition");
    assert!(
        !has_transition,
        "Normal→Normal should not inject any transition"
    );
}

/// Even with an explicit ModeTransition::None-like scenario, Normal mode
/// should not produce a ModeInstruction section either.
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

/// ModeTransition appears after ModeInstruction and before ChannelContext.
#[test]
fn test_ordering_mode_instruction_then_transition_then_channel() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        explicit_plan_path: Some(PlanPath::Standard),
        pending_mode_transition: Some(ModeTransition::Reentry),
        ..make_params(&meta, SessionMode::Plan)
    });

    let mode_idx = sections.iter().position(|s| s.name() == "mode_instruction");
    let transition_idx = sections.iter().position(|s| s.name() == "mode_transition");
    let channel_idx = sections.iter().position(|s| s.name() == "channel_context");

    assert!(mode_idx.is_some(), "ModeInstruction should be present");
    assert!(transition_idx.is_some(), "ModeTransition should be present");
    assert!(channel_idx.is_some(), "ChannelContext should be present");

    let m = mode_idx.unwrap();
    let t = transition_idx.unwrap();
    let c = channel_idx.unwrap();
    assert!(
        m < t,
        "ModeInstruction ({m}) should come before ModeTransition ({t})"
    );
    assert!(
        t < c,
        "ModeTransition ({t}) should come before ChannelContext ({c})"
    );
}

/// Without a pending transition, ModeInstruction is still followed by ChannelContext.
#[test]
fn test_ordering_without_transition_mode_instruction_then_channel() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&make_params(&meta, SessionMode::Auto));

    let mode_idx = sections.iter().position(|s| s.name() == "mode_instruction");
    let channel_idx = sections.iter().position(|s| s.name() == "channel_context");

    assert!(mode_idx.is_some());
    assert!(channel_idx.is_some());
    assert!(
        mode_idx.unwrap() < channel_idx.unwrap(),
        "ModeInstruction should come before ChannelContext even without transition"
    );
}

// ── ExitPlan in Plan→Auto scenario ──────────────────────────────────────────

/// Simulates Plan→Auto transition: ModeInstruction is for Auto mode,
/// and ExitPlan transition is injected.
#[test]
fn test_plan_to_auto_transition_exit_plan() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        pending_mode_transition: Some(ModeTransition::ExitPlan),
        ..make_params(&meta, SessionMode::Auto)
    });

    // Auto mode instruction should be present
    let mode_sec = sections.iter().find(|s| s.name() == "mode_instruction");
    assert!(
        mode_sec.is_some(),
        "Auto mode should inject ModeInstruction"
    );
    let rendered = mode_sec.unwrap().render();
    assert!(
        rendered.contains("Auto"),
        "ModeInstruction should be for Auto mode"
    );

    // ExitPlan transition should be present
    let transition = sections.iter().find(|s| s.name() == "mode_transition");
    assert!(
        transition.is_some(),
        "ExitPlan transition should be injected"
    );
    let rendered = transition.unwrap().render();
    assert!(
        rendered.contains("Exited Plan Mode"),
        "Transition should render ExitPlan content"
    );
}

// ── Reentry in Normal/Plan scenario ─────────────────────────────────────────

/// Reentry is injected when re-entering Plan Mode from Normal mode
/// with an existing plan.
#[test]
fn test_reentry_plan_mode_with_existing_plan() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        explicit_plan_path: Some(PlanPath::Standard),
        pending_mode_transition: Some(ModeTransition::Reentry),
        ..make_params(&meta, SessionMode::Plan)
    });

    let transition = sections.iter().find(|s| s.name() == "mode_transition");
    assert!(
        transition.is_some(),
        "Reentry transition should be injected"
    );
    let rendered = transition.unwrap().render();
    assert!(
        rendered.contains("Re-entering Plan Mode"),
        "Reentry transition should render correct content"
    );
}

// ── ExitAuto in Auto→Normal scenario ────────────────────────────────────────

/// ExitAuto is injected when leaving Auto Mode for Normal.
#[test]
fn test_exit_auto_from_auto_to_normal() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(&DynamicSectionsParams {
        pending_mode_transition: Some(ModeTransition::ExitAuto),
        ..make_params(&meta, SessionMode::Normal)
    });

    // Normal mode should NOT have ModeInstruction
    assert!(
        !sections.iter().any(|s| s.name() == "mode_instruction"),
        "Normal mode should not have ModeInstruction"
    );

    // But ExitAuto transition should be present
    let transition = sections.iter().find(|s| s.name() == "mode_transition");
    assert!(
        transition.is_some(),
        "ExitAuto transition should be injected"
    );
    let rendered = transition.unwrap().render();
    assert!(
        rendered.contains("Exited Auto Mode"),
        "ExitAuto should render correct content"
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
