//! Unit tests for appends partition in `inject`.
//!
//! Extracted from `inject_tests.rs` (Step 1.7) to keep file under 900-line
//! hard limit. Covers Step 1.6 test dimensions:
//! - Dimension 2: Appends independent partition
//! - Dimension 3: Override path with appends
//! - SystemPromptDynamicBuilder direct tests

use super::inject::{build_full_system_prompt, SystemPromptDynamicBuilder};
use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::{
    DynamicPromptBuilder, DynamicPromptContext, PromptOverrides, RequestContext,
};
use std::path::Path;

use super::inject_tests::{make_meta, make_params};

// ── Dimension 2: Appends independent partition ──────────────────────────

/// Appends have their own ## Append heading after dynamic content.
#[test]
fn test_appends_have_own_heading_after_dynamic() {
    let meta = make_meta("alice", "feishu", 1700000000);
    let sections = super::inject::build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let appends = vec!["extra instruction".to_string()];
    let full = build_full_system_prompt(Some("static"), &sections, &appends, None);

    assert!(full.contains("## Append"));
    assert!(full.contains("extra instruction"));
    let append_pos = full.find("## Append").unwrap();
    let dynamic_pos = full.find("Channel Context").unwrap();
    assert!(
        append_pos > dynamic_pos,
        "Append section must come after dynamic content"
    );
}

// ── Dimension 3: Override path with appends ─────────────────────────────

/// When a priority prompt is active, appends are still appended at the
/// end as a separate section, while dynamic layers are not injected.
#[test]
fn test_override_with_appends_appended_separately() {
    let overrides = PromptOverrides {
        override_prompt: Some("override prompt".into()),
        agent_prompt: None,
        custom_prompt: None,
    };
    let meta = make_meta("u", "ch", 0);
    let sections = super::inject::build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let appends = vec!["override-append".to_string()];
    let full = build_full_system_prompt(Some("static"), &sections, &appends, Some(&overrides));

    assert!(full.contains("override prompt"));
    assert!(!full.contains("static"));
    assert!(!full.contains("Channel Context"));
    assert!(!full.contains("__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__"));
    assert!(full.contains("## Append"));
    assert!(full.contains("override-append"));
}

// ── SystemPromptDynamicBuilder direct tests ─────────────────────────────

/// SystemPromptDynamicBuilder: override path merges appends into dynamic
/// field (second element), third element is always None.
#[test]
fn test_dynamic_builder_override_merges_appends_into_dynamic() {
    let builder = SystemPromptDynamicBuilder;
    let ctx = RequestContext {
        sender_id: "u".into(),
        channel: "ch".into(),
        timestamp: 0,
        chat_name: String::new(),
    };
    let overrides = PromptOverrides {
        override_prompt: Some("override".into()),
        agent_prompt: None,
        custom_prompt: None,
    };
    let appends = vec!["append1".to_string(), "append2".to_string()];
    let dpctx = DynamicPromptContext {
        system_prompt: Some("static"),
        ctx: &ctx,
        workdir: Path::new("/tmp"),
        system_appends: &appends,
        session_created_at: 0,
        session_mode: SessionMode::Normal,
        overrides: Some(&overrides),
        is_compacted: false,
        is_sub_agent: false,
        is_git_status_enabled: false,
        mode_transition: None,
        plan_file_path: None,
    };

    let (s, d) = builder.build_prompt_parts(&dpctx);
    assert_eq!(s.as_deref(), Some("override"));
    // Appends merged into dynamic field (dynamic layer is empty on override path,
    // so dynamic = only append section)
    let dynamic_text = d.unwrap();
    assert!(
        dynamic_text.contains("append1"),
        "dynamic field should contain appends"
    );
    assert!(
        dynamic_text.contains("append2"),
        "dynamic field should contain appends"
    );
    assert!(
        dynamic_text.starts_with("## Append"),
        "dynamic field should start with ## Append heading when dynamic layer is empty"
    );
}

/// SystemPromptDynamicBuilder: normal path merges appends into dynamic
/// field, third element is always None.
#[test]
fn test_dynamic_builder_normal_path_merges_appends_into_dynamic() {
    let builder = SystemPromptDynamicBuilder;
    let ctx = RequestContext {
        sender_id: "u".into(),
        channel: "ch".into(),
        timestamp: 0,
        chat_name: "test".into(),
    };
    let appends = vec!["my-note".to_string()];
    let dpctx = DynamicPromptContext {
        system_prompt: Some("static base"),
        ctx: &ctx,
        workdir: Path::new("/tmp"),
        system_appends: &appends,
        session_created_at: 0,
        session_mode: SessionMode::Normal,
        overrides: None,
        is_compacted: false,
        is_sub_agent: false,
        is_git_status_enabled: false,
        mode_transition: None,
        plan_file_path: None,
    };

    let (s, d) = builder.build_prompt_parts(&dpctx);
    assert_eq!(s.as_deref(), Some("static base"));
    let dynamic_text = d.unwrap();
    assert!(
        dynamic_text.contains("Channel Context"),
        "dynamic field should contain ChannelContext"
    );
    // Appends merged into dynamic field: dynamic rendered + append section
    assert!(
        dynamic_text.contains("my-note"),
        "dynamic field should contain appends"
    );
    assert!(
        dynamic_text.contains("## Append"),
        "dynamic field should contain ## Append heading"
    );
    // Append section comes after ChannelContext
    let channel_pos = dynamic_text.find("Channel Context").unwrap();
    let append_pos = dynamic_text.find("## Append").unwrap();
    assert!(
        append_pos > channel_pos,
        "## Append must come after ChannelContext"
    );
}

/// SystemPromptDynamicBuilder: empty appends returns None for third
/// element and dynamic field contains only dynamic layer content.
#[test]
fn test_dynamic_builder_empty_appends_returns_none() {
    let builder = SystemPromptDynamicBuilder;
    let ctx = RequestContext {
        sender_id: "u".into(),
        channel: "ch".into(),
        timestamp: 0,
        chat_name: String::new(),
    };
    let dpctx = DynamicPromptContext {
        system_prompt: Some("static"),
        ctx: &ctx,
        workdir: Path::new("/tmp"),
        system_appends: &[],
        session_created_at: 0,
        session_mode: SessionMode::Normal,
        overrides: None,
        is_compacted: false,
        is_sub_agent: false,
        is_git_status_enabled: false,
        mode_transition: None,
        plan_file_path: None,
    };

    let (s, d) = builder.build_prompt_parts(&dpctx);
    assert_eq!(s.as_deref(), Some("static"));
    // Dynamic field should contain only ChannelContext (no ## Append)
    let dynamic_text = d.unwrap();
    assert!(dynamic_text.contains("Channel Context"));
    assert!(
        !dynamic_text.contains("## Append"),
        "empty appends should not produce ## Append heading"
    );
}

// ── Merge boundary: dynamic empty + appends non-empty ────────────────

/// When dynamic layer is empty (override path) and appends are non-empty,
/// the dynamic field contains only the append section (## Append heading
/// with numbered entries).
#[test]
fn test_dynamic_empty_appends_nonempty() {
    let builder = SystemPromptDynamicBuilder;
    let ctx = RequestContext {
        sender_id: "u".into(),
        channel: "ch".into(),
        timestamp: 0,
        chat_name: String::new(),
    };
    let overrides = PromptOverrides {
        override_prompt: Some("override".into()),
        agent_prompt: None,
        custom_prompt: None,
    };
    let appends = vec!["note A".to_string()];
    let dpctx = DynamicPromptContext {
        system_prompt: Some("static"),
        ctx: &ctx,
        workdir: Path::new("/tmp"),
        system_appends: &appends,
        session_created_at: 0,
        session_mode: SessionMode::Normal,
        overrides: Some(&overrides),
        is_compacted: false,
        is_sub_agent: false,
        is_git_status_enabled: false,
        mode_transition: None,
        plan_file_path: None,
    };

    let (s, d) = builder.build_prompt_parts(&dpctx);
    assert_eq!(s.as_deref(), Some("override"));
    let dynamic_text = d.unwrap();
    // Dynamic layer is empty on override path → dynamic field = only append section
    assert!(
        dynamic_text.starts_with("## Append"),
        "dynamic field should start with ## Append heading, got: {}",
        dynamic_text
    );
    assert!(dynamic_text.contains("note A"));
    assert!(!dynamic_text.contains("Channel Context"));
}

// ── Merge boundary: both empty ───────────────────────────────────────

/// When both dynamic layer and appends are empty, dynamic field is None.
#[test]
fn test_both_empty_dynamic_is_none() {
    let builder = SystemPromptDynamicBuilder;
    let ctx = RequestContext {
        sender_id: "u".into(),
        channel: "ch".into(),
        timestamp: 0,
        chat_name: String::new(),
    };
    let overrides = PromptOverrides {
        override_prompt: Some("override".into()),
        agent_prompt: None,
        custom_prompt: None,
    };
    let dpctx = DynamicPromptContext {
        system_prompt: Some("static"),
        ctx: &ctx,
        workdir: Path::new("/tmp"),
        system_appends: &[],
        session_created_at: 0,
        session_mode: SessionMode::Normal,
        overrides: Some(&overrides),
        is_compacted: false,
        is_sub_agent: false,
        is_git_status_enabled: false,
        mode_transition: None,
        plan_file_path: None,
    };

    let (s, d) = builder.build_prompt_parts(&dpctx);
    assert_eq!(s.as_deref(), Some("override"));
    assert!(d.is_none(), "both empty → dynamic field should be None");
}

// ── Merge boundary: appends multi-entry numbering ────────────────────

/// Multiple appends render with correct 0-based numbering: [0], [1], [2].
#[test]
fn test_appends_multi_entry_numbering() {
    let overrides = PromptOverrides {
        override_prompt: Some("override".into()),
        agent_prompt: None,
        custom_prompt: None,
    };
    let meta = make_meta("u", "ch", 0);
    let sections = super::inject::build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let appends = vec![
        "first item".to_string(),
        "second item".to_string(),
        "third item".to_string(),
    ];
    let full = build_full_system_prompt(Some("static"), &sections, &appends, Some(&overrides));

    assert!(full.contains("[0] first item"));
    assert!(full.contains("[1] second item"));
    assert!(full.contains("[2] third item"));
}

/// Multi-entry appends via builder: numbering is preserved in dynamic field.
#[test]
fn test_dynamic_builder_multi_entry_appends_numbering() {
    let builder = SystemPromptDynamicBuilder;
    let ctx = RequestContext {
        sender_id: "u".into(),
        channel: "ch".into(),
        timestamp: 0,
        chat_name: String::new(),
    };
    let overrides = PromptOverrides {
        override_prompt: Some("override".into()),
        agent_prompt: None,
        custom_prompt: None,
    };
    let appends = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
    let dpctx = DynamicPromptContext {
        system_prompt: Some("static"),
        ctx: &ctx,
        workdir: Path::new("/tmp"),
        system_appends: &appends,
        session_created_at: 0,
        session_mode: SessionMode::Normal,
        overrides: Some(&overrides),
        is_compacted: false,
        is_sub_agent: false,
        is_git_status_enabled: false,
        mode_transition: None,
        plan_file_path: None,
    };

    let (_, d) = builder.build_prompt_parts(&dpctx);
    let dynamic_text = d.unwrap();
    assert!(dynamic_text.contains("[0] alpha"));
    assert!(dynamic_text.contains("[1] beta"));
    assert!(dynamic_text.contains("[2] gamma"));
}

// ── Override priority: agent_prompt path ──────────────────────────────

/// When only agent_prompt is set (no override), static is replaced by
/// agent_prompt and appends are merged into the dynamic field.
#[test]
fn test_agent_prompt_replaces_static_merges_appends() {
    let overrides = PromptOverrides {
        override_prompt: None,
        agent_prompt: Some("agent-level prompt".into()),
        custom_prompt: None,
    };
    let meta = make_meta("u", "ch", 0);
    let sections = super::inject::build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let appends = vec!["agent-append".to_string()];
    let full = build_full_system_prompt(Some("static"), &sections, &appends, Some(&overrides));

    assert!(full.contains("agent-level prompt"));
    assert!(!full.contains("static"));
    assert!(!full.contains("Channel Context"));
    assert!(full.contains("## Append"));
    assert!(full.contains("agent-append"));
}

/// agent_prompt via builder: static = agent_prompt, dynamic = only appends.
#[test]
fn test_dynamic_builder_agent_prompt_merges_appends() {
    let builder = SystemPromptDynamicBuilder;
    let ctx = RequestContext {
        sender_id: "u".into(),
        channel: "ch".into(),
        timestamp: 0,
        chat_name: String::new(),
    };
    let overrides = PromptOverrides {
        override_prompt: None,
        agent_prompt: Some("agent prompt".into()),
        custom_prompt: None,
    };
    let appends = vec!["note".to_string()];
    let dpctx = DynamicPromptContext {
        system_prompt: Some("static"),
        ctx: &ctx,
        workdir: Path::new("/tmp"),
        system_appends: &appends,
        session_created_at: 0,
        session_mode: SessionMode::Normal,
        overrides: Some(&overrides),
        is_compacted: false,
        is_sub_agent: false,
        is_git_status_enabled: false,
        mode_transition: None,
        plan_file_path: None,
    };

    let (s, d) = builder.build_prompt_parts(&dpctx);
    assert_eq!(s.as_deref(), Some("agent prompt"));
    let dynamic_text = d.unwrap();
    assert!(dynamic_text.starts_with("## Append"));
    assert!(dynamic_text.contains("note"));
}

// ── Override priority: custom_prompt path ─────────────────────────────

/// When only custom_prompt is set (no override/agent), static is replaced
/// by custom_prompt and appends are merged into the dynamic field.
#[test]
fn test_custom_prompt_replaces_static_merges_appends() {
    let overrides = PromptOverrides {
        override_prompt: None,
        agent_prompt: None,
        custom_prompt: Some("user custom prompt".into()),
    };
    let meta = make_meta("u", "ch", 0);
    let sections = super::inject::build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let appends = vec!["custom-append".to_string()];
    let full = build_full_system_prompt(Some("static"), &sections, &appends, Some(&overrides));

    assert!(full.contains("user custom prompt"));
    assert!(!full.contains("static"));
    assert!(!full.contains("Channel Context"));
    assert!(full.contains("## Append"));
    assert!(full.contains("custom-append"));
}

/// custom_prompt via builder: static = custom_prompt, dynamic = only appends.
#[test]
fn test_dynamic_builder_custom_prompt_merges_appends() {
    let builder = SystemPromptDynamicBuilder;
    let ctx = RequestContext {
        sender_id: "u".into(),
        channel: "ch".into(),
        timestamp: 0,
        chat_name: String::new(),
    };
    let overrides = PromptOverrides {
        override_prompt: None,
        agent_prompt: None,
        custom_prompt: Some("custom prompt".into()),
    };
    let appends = vec!["item".to_string()];
    let dpctx = DynamicPromptContext {
        system_prompt: Some("static"),
        ctx: &ctx,
        workdir: Path::new("/tmp"),
        system_appends: &appends,
        session_created_at: 0,
        session_mode: SessionMode::Normal,
        overrides: Some(&overrides),
        is_compacted: false,
        is_sub_agent: false,
        is_git_status_enabled: false,
        mode_transition: None,
        plan_file_path: None,
    };

    let (s, d) = builder.build_prompt_parts(&dpctx);
    assert_eq!(s.as_deref(), Some("custom prompt"));
    let dynamic_text = d.unwrap();
    assert!(dynamic_text.starts_with("## Append"));
    assert!(dynamic_text.contains("item"));
}

// ── Override priority: override > agent > custom ──────────────────────

/// When all three overrides are set, override_prompt wins.
#[test]
fn test_override_wins_over_agent_and_custom() {
    let overrides = PromptOverrides {
        override_prompt: Some("highest".into()),
        agent_prompt: Some("agent".into()),
        custom_prompt: Some("custom".into()),
    };
    let meta = make_meta("u", "ch", 0);
    let sections = super::inject::build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let appends = vec!["append".to_string()];
    let full = build_full_system_prompt(Some("static"), &sections, &appends, Some(&overrides));

    assert!(full.contains("highest"));
    assert!(!full.contains("agent"));
    assert!(!full.contains("custom"));
    assert!(full.contains("## Append"));
}

/// When override is None but agent is set, agent wins over custom.
#[test]
fn test_agent_wins_over_custom() {
    let overrides = PromptOverrides {
        override_prompt: None,
        agent_prompt: Some("agent wins".into()),
        custom_prompt: Some("custom loses".into()),
    };
    let meta = make_meta("u", "ch", 0);
    let sections = super::inject::build_dynamic_sections(&make_params(&meta, SessionMode::Normal));
    let full = build_full_system_prompt(Some("static"), &sections, &[], Some(&overrides));

    assert!(full.contains("agent wins"));
    assert!(!full.contains("custom loses"));
}

/// Priority path with empty appends: result is just the priority prompt.
#[test]
fn test_priority_path_empty_appends() {
    let overrides = PromptOverrides {
        override_prompt: Some("override".into()),
        agent_prompt: None,
        custom_prompt: None,
    };
    let full = build_full_system_prompt(Some("static"), &[], &[], Some(&overrides));
    assert_eq!(full, "override");
}

/// Priority path with appends: format is `priority + "\n\n## Append\n" + numbered`.
#[test]
fn test_priority_path_with_appends_format() {
    let overrides = PromptOverrides {
        override_prompt: Some("priority".into()),
        agent_prompt: None,
        custom_prompt: None,
    };
    let appends = vec!["x".to_string(), "y".to_string()];
    let full = build_full_system_prompt(Some("static"), &[], &appends, Some(&overrides));

    // Must start with the priority prompt
    assert!(full.starts_with("priority"));
    // Must contain the append heading and entries
    assert!(full.contains("## Append"));
    assert!(full.contains("[0] x"));
    assert!(full.contains("[1] y"));
    // Must not contain the old static prompt
    assert!(!full.contains("static"));
    // Must not contain boundary marker
    assert!(!full.contains("__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__"));
}
