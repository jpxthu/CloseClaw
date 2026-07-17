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

use super::inject::build_dynamic_sections;
use closeclaw_common::{ModeTransition, PlanPath, SessionMode};
use closeclaw_gateway::session_handler::MessageMetadata;

fn make_meta(sender: &str, channel: &str, ts: i64) -> MessageMetadata {
    MessageMetadata {
        sender_id: sender.to_string(),
        channel: channel.to_string(),
        timestamp: ts,
    }
}

// ── One-shot injection ──────────────────────────────────────────────────────

/// After a ModeTransition is injected once, the next build with `None`
/// must NOT contain a transition section.
#[test]
fn test_one_shot_injection_transition_appears_exactly_once() {
    let meta = make_meta("u", "ch", 0);

    // First build — with ExitPlan transition
    let sections = build_dynamic_sections(
        &meta,
        None,
        &[],
        None,
        SessionMode::Auto,
        None,
        None,
        Some(ModeTransition::ExitPlan),
    );
    let has_transition = sections.iter().any(|s| s.name() == "mode_transition");
    assert!(has_transition, "first build should include transition");

    // Second build — without transition (simulates take happened)
    let sections2 =
        build_dynamic_sections(&meta, None, &[], None, SessionMode::Auto, None, None, None);
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
    let s1 = build_dynamic_sections(
        &meta,
        None,
        &[],
        None,
        SessionMode::Auto,
        None,
        None,
        Some(ModeTransition::ExitPlan),
    );
    let t1 = s1.iter().find(|s| s.name() == "mode_transition").unwrap();
    assert!(t1.render().contains("Exited Plan Mode"));

    // Switch 2: Reentry
    let s2 = build_dynamic_sections(
        &meta,
        None,
        &[],
        None,
        SessionMode::Plan,
        Some(PlanPath::Standard),
        None,
        Some(ModeTransition::Reentry),
    );
    let t2 = s2.iter().find(|s| s.name() == "mode_transition").unwrap();
    assert!(t2.render().contains("Re-entering Plan Mode"));

    // Switch 3: ExitAuto
    let s3 = build_dynamic_sections(
        &meta,
        None,
        &[],
        None,
        SessionMode::Normal,
        None,
        None,
        Some(ModeTransition::ExitAuto),
    );
    let t3 = s3.iter().find(|s| s.name() == "mode_transition").unwrap();
    assert!(t3.render().contains("Exited Auto Mode"));
}

// ── Boundary: Normal→Normal ─────────────────────────────────────────────────

/// Staying in Normal mode with no transition produces no ModeTransition section.
#[test]
fn test_normal_to_normal_no_transition() {
    let meta = make_meta("u", "ch", 0);
    let sections = build_dynamic_sections(
        &meta,
        None,
        &[],
        None,
        SessionMode::Normal,
        None,
        None,
        None,
    );
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
    let sections = build_dynamic_sections(
        &meta,
        None,
        &[],
        None,
        SessionMode::Normal,
        None,
        None,
        None,
    );
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
    let sections = build_dynamic_sections(
        &meta,
        None,
        &[],
        None,
        SessionMode::Plan,
        Some(PlanPath::Standard),
        None,
        Some(ModeTransition::Reentry),
    );

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
    let sections =
        build_dynamic_sections(&meta, None, &[], None, SessionMode::Auto, None, None, None);

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
    let sections = build_dynamic_sections(
        &meta,
        None,
        &[],
        None,
        SessionMode::Auto,
        None,
        None,
        Some(ModeTransition::ExitPlan),
    );

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
    let sections = build_dynamic_sections(
        &meta,
        None,
        &[],
        None,
        SessionMode::Plan,
        Some(PlanPath::Standard),
        None,
        Some(ModeTransition::Reentry),
    );

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
    let sections = build_dynamic_sections(
        &meta,
        None,
        &[],
        None,
        SessionMode::Normal,
        None,
        None,
        Some(ModeTransition::ExitAuto),
    );

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
