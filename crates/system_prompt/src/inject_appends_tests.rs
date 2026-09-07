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

/// SystemPromptDynamicBuilder: override path returns appends as
/// independent third partition, not mixed into dynamic.
#[test]
fn test_dynamic_builder_override_returns_appends_independent() {
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

    let (s, d, a) = builder.build_prompt_parts(&dpctx);
    assert_eq!(s.as_deref(), Some("override"));
    assert!(d.is_none(), "dynamic should be None on override path");
    assert!(a.is_some(), "appends should be present");
    let appends_text = a.unwrap();
    assert!(appends_text.contains("append1"));
    assert!(appends_text.contains("append2"));
    assert!(d.is_none() || !d.unwrap().contains("append1"));
}

/// SystemPromptDynamicBuilder: normal path returns appends as
/// independent third partition.
#[test]
fn test_dynamic_builder_normal_path_returns_appends_independent() {
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

    let (s, d, a) = builder.build_prompt_parts(&dpctx);
    assert_eq!(s.as_deref(), Some("static base"));
    assert!(d.is_some(), "dynamic should be present");
    assert!(d.as_deref().unwrap().contains("Channel Context"));
    assert!(a.is_some(), "appends should be present");
    let appends_text = a.unwrap();
    assert!(appends_text.contains("my-note"));
    assert!(
        !d.unwrap().contains("my-note"),
        "appends must NOT appear in dynamic partition"
    );
}

/// SystemPromptDynamicBuilder: empty appends returns None for third partition.
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

    let (_, _, a) = builder.build_prompt_parts(&dpctx);
    assert!(
        a.is_none(),
        "empty appends should return None for third partition"
    );
}
