//! Unit tests for `split_static_dynamic` and `DynamicPromptBuilder` trait.
//!
//! Covers Step 1.5 test dimensions:
//! - split_static_dynamic: with boundary marker, without marker, empty
//! - DynamicPromptBuilder: normal path, override path, no-appends

use super::{split_static_dynamic, DynamicPromptContext, PromptOverrides, RequestContext};
use std::path::Path;

// ── split_static_dynamic ──────────────────────────────────────────────────

#[test]
fn test_split_static_dynamic_with_marker() {
    let input = "static content\n__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__\ndynamic content";
    let (s, d) = split_static_dynamic(input);
    assert_eq!(s.as_deref(), Some("static content"));
    assert_eq!(d.as_deref(), Some("dynamic content"));
}

#[test]
fn test_split_static_dynamic_no_marker() {
    let input = "all static content here";
    let (s, d) = split_static_dynamic(input);
    assert_eq!(s.as_deref(), Some("all static content here"));
    assert!(d.is_none());
}

#[test]
fn test_split_static_dynamic_empty_string() {
    let (s, d) = split_static_dynamic("");
    assert!(s.is_none());
    assert!(d.is_none());
}

#[test]
fn test_split_static_dynamic_marker_at_start() {
    let input = "__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__\nonly dynamic";
    let (s, d) = split_static_dynamic(input);
    assert!(s.is_none());
    assert_eq!(d.as_deref(), Some("only dynamic"));
}

#[test]
fn test_split_static_dynamic_marker_at_end() {
    let input = "only static\n__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__";
    let (s, d) = split_static_dynamic(input);
    assert_eq!(s.as_deref(), Some("only static"));
    assert!(d.is_none());
}

#[test]
fn test_split_static_dynamic_whitespace_around_marker() {
    let input = "  static  \n__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__\n  dynamic  ";
    let (s, d) = split_static_dynamic(input);
    // trim_end on static part, trim_start on dynamic part
    assert_eq!(s.as_deref(), Some("  static"));
    assert_eq!(d.as_deref(), Some("dynamic  "));
}

#[test]
fn test_split_static_dynamic_multiple_markers_uses_first() {
    let input = "a\n__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__\nb\n__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__\nc";
    let (s, d) = split_static_dynamic(input);
    assert_eq!(s.as_deref(), Some("a"));
    assert_eq!(
        d.as_deref(),
        Some("b\n__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__\nc")
    );
}

// ── DynamicPromptContext construction ─────────────────────────────────────

#[test]
fn test_dynamic_prompt_context_fields() {
    let ctx = RequestContext {
        sender_id: "ou_test".into(),
        channel: "feishu".into(),
        timestamp: 1000,
    };
    let dpctx = DynamicPromptContext {
        system_prompt: Some("static"),
        ctx: &ctx,
        workdir: Path::new("/tmp"),
        system_appends: &[],
        session_created_at: 999,
        session_mode: super::SessionMode::Normal,
        overrides: None,
        user_input: None,
    };
    assert_eq!(dpctx.system_prompt, Some("static"));
    assert_eq!(dpctx.ctx.sender_id, "ou_test");
    assert_eq!(dpctx.ctx.channel, "feishu");
    assert_eq!(dpctx.ctx.timestamp, 1000);
    assert_eq!(dpctx.session_mode, super::SessionMode::Normal);
    assert!(dpctx.user_input.is_none());
}

#[test]
fn test_dynamic_prompt_context_with_appends_and_overrides() {
    let ctx = RequestContext::default();
    let overrides = PromptOverrides {
        override_prompt: Some("override".into()),
        agent_prompt: None,
        custom_prompt: None,
    };
    let appends = vec!["append1".to_string(), "append2".to_string()];
    let dpctx = DynamicPromptContext {
        system_prompt: None,
        ctx: &ctx,
        workdir: Path::new("/workspace"),
        system_appends: &appends,
        session_created_at: 0,
        session_mode: super::SessionMode::Plan,
        overrides: Some(&overrides),
        user_input: Some("fix the bug"),
    };
    assert!(dpctx.system_prompt.is_none());
    assert_eq!(dpctx.system_appends.len(), 2);
    assert_eq!(dpctx.system_appends[0], "append1");
    assert!(dpctx.overrides.is_some());
    assert_eq!(dpctx.user_input, Some("fix the bug"));
    assert_eq!(dpctx.session_mode, super::SessionMode::Plan);
}
