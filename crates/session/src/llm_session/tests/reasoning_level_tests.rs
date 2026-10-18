use super::*;
use crate::persistence::ReasoningLevel;

// ── reasoning_level tests ─────────────────────────────────────────────────

#[test]
fn test_conversation_session_default_reasoning_level() {
    let session = ConversationSession::new("s_rl_default".into(), "gpt-4o".into(), tmp_path());
    assert_eq!(session.reasoning_level, ReasoningLevel::High);
}

#[test]
fn test_conversation_session_with_reasoning_level() {
    let session = ConversationSession::new("s_rl_custom".into(), "gpt-4o".into(), tmp_path())
        .with_reasoning_level(ReasoningLevel::Low);
    assert_eq!(session.reasoning_level, ReasoningLevel::Low);
}

#[test]
fn test_build_api_request_includes_reasoning_level() {
    let session = ConversationSession::new("s_rl_req".into(), "gpt-4o".into(), tmp_path())
        .with_reasoning_level(ReasoningLevel::Max);
    let req = session.build_api_request();
    assert_eq!(req.reasoning_level, ReasoningLevel::Max);
}

#[test]
fn test_build_api_request_default_reasoning_level() {
    let session = ConversationSession::new("s_rl_def_req".into(), "gpt-4o".into(), tmp_path());
    let req = session.build_api_request();
    assert_eq!(req.reasoning_level, ReasoningLevel::High);
}

#[test]
fn test_build_api_request_reasoning_level_medium() {
    let session = ConversationSession::new("s_rl_med".into(), "gpt-4o".into(), tmp_path())
        .with_reasoning_level(ReasoningLevel::Medium);
    let req = session.build_api_request();
    assert_eq!(req.reasoning_level, ReasoningLevel::Medium);
}

// ── effective_reasoning_level tests ──────────────────────────────────────

#[test]
fn test_effective_reasoning_level_defaults_to_reasoning_level() {
    let session = ConversationSession::new("s_erl1".into(), "gpt-4o".into(), tmp_path());
    assert_eq!(session.effective_reasoning_level(), ReasoningLevel::High);
}

#[test]
fn test_effective_reasoning_level_after_set() {
    let mut session = ConversationSession::new("s_erl2".into(), "gpt-4o".into(), tmp_path());
    session.set_effective_reasoning_level(ReasoningLevel::Low);
    assert_eq!(session.effective_reasoning_level(), ReasoningLevel::Low);
}

#[test]
fn test_set_reasoning_level_resets_effective() {
    let mut session = ConversationSession::new("s_erl3".into(), "gpt-4o".into(), tmp_path());
    session.set_effective_reasoning_level(ReasoningLevel::Medium);
    assert_eq!(session.effective_reasoning_level(), ReasoningLevel::Medium);
    // Setting reasoning_level should reset effective to None (fallback).
    session.set_reasoning_level(ReasoningLevel::High);
    assert_eq!(session.effective_reasoning_level(), ReasoningLevel::High);
}
