// ═══════════════════════════════════════════════════════════════════════════
// Priority Prompt Override Tests (build_full_system_prompt)
// ═══════════════════════════════════════════════════════════════════════════

use super::session_handler::MessageMetadata;
use closeclaw_common::system_prompt::builder::PromptOverrides;
use closeclaw_common::system_prompt::inject::{
    build_dynamic_sections, build_full_system_prompt, DynamicSectionsParams,
};
use closeclaw_common::SessionMode;

fn make_meta(sender: &str, channel: &str, ts: i64) -> MessageMetadata {
    MessageMetadata {
        sender_id: sender.to_string(),
        channel: channel.to_string(),
        timestamp: ts,
        trace_id: None,
        session_key: None,
        span_id: None,
        chat_name: String::new(),
    }
}

fn make_params(meta: &MessageMetadata, session_mode: SessionMode) -> DynamicSectionsParams<'_> {
    DynamicSectionsParams {
        meta,
        workdir_path: None,
        session_timestamp: None,
        session_mode,
        explicit_plan_path: None,
        user_input: None,
        is_compacted: false,
        is_sub_agent: false,
        is_git_status_enabled: false,
        mode_transition: None,
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
    let dynamic = build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let full = build_full_system_prompt(Some("static"), &dynamic, &[], Some(&overrides));

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
    let dynamic = build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let full = build_full_system_prompt(Some("static"), &dynamic, &[], Some(&overrides));

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
    let dynamic = build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let full = build_full_system_prompt(Some("static"), &dynamic, &[], Some(&overrides));

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
    let dynamic = build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let full = build_full_system_prompt(Some("static"), &dynamic, &[], Some(&overrides));

    // Only override prompt appears
    assert!(full.contains("override wins"));
    assert!(!full.contains("agent ignored"));
    assert!(!full.contains("custom ignored"));
    // Static prompt is replaced
    assert!(!full.contains("static"));
}

/// Test (c): On priority hit, only appends are appended; no ChannelContext/GitStatus.
#[test]
fn test_priority_hit_only_appends() {
    let overrides = PromptOverrides {
        override_prompt: Some("override prompt".into()),
        agent_prompt: None,
        custom_prompt: None,
    };
    let meta = make_meta("alice", "telegram", 1700000000);
    let dynamic = build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let appends = vec!["extra instruction".to_string()];
    let full = build_full_system_prompt(Some("static"), &dynamic, &appends, Some(&overrides));

    // Override prompt is the base
    assert!(full.contains("override prompt"));
    // Appends are present
    assert!(full.contains("extra instruction"));
    assert!(full.contains("## Append"));
    // Dynamic layers are NOT injected
    assert!(
        !full.contains("sender_id: alice"),
        "ChannelContext should not appear on priority hit"
    );

    assert!(
        !full.contains("__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__"),
        "Boundary marker should not appear on priority hit",
    );
}

/// Test (c): Multiple appends on priority hit.
#[test]
fn test_priority_hit_multiple_appends() {
    let overrides = PromptOverrides {
        agent_prompt: Some("agent prompt".into()),
        ..Default::default()
    };
    let meta = make_meta("u", "ch", 0);
    let dynamic = build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let appends = vec!["first".to_string(), "second".to_string()];
    let full = build_full_system_prompt(Some("static"), &dynamic, &appends, Some(&overrides));

    assert!(full.contains("agent prompt"));
    assert!(full.contains("first"));
    assert!(full.contains("second"));
    assert!(!full.contains("sender_id"));
}

/// Test (d): When no override matches, normal behavior is preserved.
#[test]
fn test_priority_no_hit_normal_behavior() {
    let overrides = PromptOverrides::default(); // all None
    let meta = make_meta("bob", "feishu", 1700000000);
    let dynamic = build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let full = build_full_system_prompt(Some("static prompt"), &dynamic, &[], Some(&overrides));

    // Static prompt is preserved
    assert!(full.contains("static prompt"));
    // Boundary marker present
    assert!(full.contains("__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__"));
    // Dynamic layers injected
    assert!(full.contains("sender_id: bob"));
}

/// Test (e): None overrides behaves identically to normal path.
#[test]
fn test_priority_none_overrides_normal_behavior() {
    let meta = make_meta("carol", "ch", 1700000000);
    let dynamic = build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let appends = vec!["appendix".to_string()];
    let full_none = build_full_system_prompt(Some("static"), &dynamic, &appends, None);
    let full_default = build_full_system_prompt(
        Some("static"),
        &dynamic,
        &appends,
        Some(&PromptOverrides::default()),
    );

    // Both should produce the same output
    assert_eq!(full_none, full_default);
    // Both contain static + dynamic
    assert!(full_none.contains("static"));
    assert!(full_none.contains("sender_id: carol"));
    assert!(full_none.contains("appendix"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Dimension 6: Priority prompt + appends — appends still attached
// ═══════════════════════════════════════════════════════════════════════════

/// When a priority prompt is active, appends must still be appended
/// at the end of the output, after the priority prompt text.
#[test]
fn test_priority_prompt_appends_still_attached() {
    let overrides = PromptOverrides {
        override_prompt: Some("priority override".into()),
        ..Default::default()
    };
    let meta = make_meta("dave", "feishu", 0);
    let dynamic = build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let appends = vec!["appendix item".to_string()];
    let full = build_full_system_prompt(Some("static"), &dynamic, &appends, Some(&overrides));
    // Priority prompt is the base
    assert!(full.contains("priority override"));
    // Appends are present
    assert!(full.contains("## Append"));
    assert!(full.contains("appendix item"));
    // Dynamic layers are NOT injected (priority replaces them)
    assert!(!full.contains("sender_id: dave"));
    assert!(!full.contains("__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__"));
    // Append section comes after the priority prompt
    let priority_pos = full.find("priority override").unwrap();
    let append_pos = full.find("## Append").unwrap();
    assert!(append_pos > priority_pos);
}

/// Agent prompt priority + appends: appends attached after agent prompt.
#[test]
fn test_agent_prompt_priority_with_appends() {
    let overrides = PromptOverrides {
        agent_prompt: Some("agent-level prompt".into()),
        ..Default::default()
    };
    let meta = make_meta("u", "ch", 0);
    let dynamic = build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let appends = vec!["extra".to_string()];
    let full = build_full_system_prompt(Some("static"), &dynamic, &appends, Some(&overrides));
    assert!(full.contains("agent-level prompt"));
    assert!(full.contains("## Append"));
    assert!(full.contains("extra"));
    assert!(!full.contains("sender_id"));
}
