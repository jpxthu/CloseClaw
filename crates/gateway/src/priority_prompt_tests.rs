// ═══════════════════════════════════════════════════════════════════════════
// Priority Prompt Override Tests (build_full_system_prompt)
// ═══════════════════════════════════════════════════════════════════════════

use super::session_handler::MessageMetadata;
use closeclaw_common::system_prompt::builder::PromptOverrides;
use closeclaw_common::system_prompt::inject::{build_dynamic_sections, build_full_system_prompt};
use closeclaw_common::SessionMode;

fn make_meta(sender: &str, channel: &str, ts: i64) -> MessageMetadata {
    MessageMetadata {
        sender_id: sender.to_string(),
        channel: channel.to_string(),
        timestamp: ts,
    }
}

/// Test (a): Three-tier priority: override > agent > custom.
/// When override_prompt is set, it wins over agent_prompt and custom_prompt.
#[test]
fn test_priority_override_wins_over_agent_and_custom() {
    let overrides = PromptOverrides {
        override_prompt: Some("override prompt".into()),
        agent_prompt: Some("agent prompt".into()),
        custom_prompt: Some("custom prompt".into()),
    };
    let meta = make_meta("u", "ch", 0);
    let dynamic = build_dynamic_sections(&meta, None, &[], None, SessionMode::Normal);
    let full = build_full_system_prompt(Some("static"), &dynamic, Some(&overrides));

    assert!(full.contains("override prompt"));
    assert!(!full.contains("agent prompt"));
    assert!(!full.contains("custom prompt"));
}

/// Test (a): When override is None, agent_prompt wins over custom_prompt.
#[test]
fn test_priority_agent_wins_over_custom() {
    let overrides = PromptOverrides {
        override_prompt: None,
        agent_prompt: Some("agent prompt".into()),
        custom_prompt: Some("custom prompt".into()),
    };
    let meta = make_meta("u", "ch", 0);
    let dynamic = build_dynamic_sections(&meta, None, &[], None, SessionMode::Normal);
    let full = build_full_system_prompt(Some("static"), &dynamic, Some(&overrides));

    assert!(full.contains("agent prompt"));
    assert!(!full.contains("custom prompt"));
}

/// Test (a): When override and agent are None, custom_prompt is used.
#[test]
fn test_priority_custom_fallback() {
    let overrides = PromptOverrides {
        override_prompt: None,
        agent_prompt: None,
        custom_prompt: Some("custom prompt".into()),
    };
    let meta = make_meta("u", "ch", 0);
    let dynamic = build_dynamic_sections(&meta, None, &[], None, SessionMode::Normal);
    let full = build_full_system_prompt(Some("static"), &dynamic, Some(&overrides));

    assert!(full.contains("custom prompt"));
}

/// Test (b): Mutual exclusivity — override takes precedence, agent/custom ignored.
#[test]
fn test_priority_override_mutual_exclusivity() {
    let overrides = PromptOverrides {
        override_prompt: Some("override wins".into()),
        agent_prompt: Some("agent ignored".into()),
        custom_prompt: Some("custom ignored".into()),
    };
    let meta = make_meta("u", "ch", 0);
    let dynamic = build_dynamic_sections(&meta, None, &[], None, SessionMode::Normal);
    let full = build_full_system_prompt(Some("static"), &dynamic, Some(&overrides));

    // Only override prompt appears
    assert!(full.contains("override wins"));
    assert!(!full.contains("agent ignored"));
    assert!(!full.contains("custom ignored"));
    // Static prompt is replaced
    assert!(!full.contains("static"));
}

/// Test (c): On priority hit, only AppendSection is appended;
/// no ChannelContext/SessionState/GitStatus.
#[test]
fn test_priority_hit_only_appends_append_section() {
    let overrides = PromptOverrides {
        override_prompt: Some("override prompt".into()),
        agent_prompt: None,
        custom_prompt: None,
    };
    let meta = make_meta("alice", "telegram", 1700000000);
    let dynamic = build_dynamic_sections(
        &meta,
        None,
        &["extra instruction".into()],
        None,
        SessionMode::Normal,
    );
    let full = build_full_system_prompt(Some("static"), &dynamic, Some(&overrides));

    // Override prompt is the base
    assert!(full.contains("override prompt"));
    // AppendSection is present
    assert!(full.contains("extra instruction"));
    assert!(full.contains("## Append"));
    // Dynamic layers are NOT injected
    assert!(
        !full.contains("sender_id: alice"),
        "ChannelContext should not appear on priority hit"
    );
    assert!(
        !full.contains("pending_tasks:"),
        "SessionState should not appear on priority hit"
    );
    assert!(
        !full.contains("__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__"),
        "Boundary marker should not appear on priority hit",
    );
}

/// Test (c): AppendSection with multiple entries on priority hit.
#[test]
fn test_priority_hit_multiple_appends() {
    let overrides = PromptOverrides {
        agent_prompt: Some("agent prompt".into()),
        ..Default::default()
    };
    let meta = make_meta("u", "ch", 0);
    let dynamic = build_dynamic_sections(
        &meta,
        None,
        &["first".into(), "second".into()],
        None,
        SessionMode::Normal,
    );
    let full = build_full_system_prompt(Some("static"), &dynamic, Some(&overrides));

    assert!(full.contains("agent prompt"));
    assert!(full.contains("first"));
    assert!(full.contains("second"));
    assert!(!full.contains("sender_id"));
    assert!(!full.contains("pending_tasks:"));
}

/// Test (d): When no override matches, normal behavior is preserved.
#[test]
fn test_priority_no_hit_normal_behavior() {
    let overrides = PromptOverrides::default(); // all None
    let meta = make_meta("bob", "feishu", 1700000000);
    let dynamic = build_dynamic_sections(&meta, None, &[], None, SessionMode::Normal);
    let full = build_full_system_prompt(Some("static prompt"), &dynamic, Some(&overrides));

    // Static prompt is preserved
    assert!(full.contains("static prompt"));
    // Boundary marker present
    assert!(full.contains("__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__"));
    // Dynamic layers injected
    assert!(full.contains("sender_id: bob"));
    assert!(full.contains("pending_tasks:"));
}

/// Test (e): None overrides behaves identically to normal path.
#[test]
fn test_priority_none_overrides_normal_behavior() {
    let meta = make_meta("carol", "ch", 1700000000);
    let dynamic =
        build_dynamic_sections(&meta, None, &["appendix".into()], None, SessionMode::Normal);
    let full_none = build_full_system_prompt(Some("static"), &dynamic, None);
    let full_default =
        build_full_system_prompt(Some("static"), &dynamic, Some(&PromptOverrides::default()));

    // Both should produce the same output
    assert_eq!(full_none, full_default);
    // Both contain static + dynamic
    assert!(full_none.contains("static"));
    assert!(full_none.contains("sender_id: carol"));
    assert!(full_none.contains("pending_tasks:"));
    assert!(full_none.contains("appendix"));
}
