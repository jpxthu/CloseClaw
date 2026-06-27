//! Migrated tests from handlers_tests.rs (moved here to keep files ≤500 lines).

use std::sync::Arc;

use crate::session::persistence::ReasoningLevel;
use crate::slash::dispatcher::SlashDispatcher;
use crate::slash::handler::SlashHandler;
use crate::slash::registry::HandlerRegistry;

use super::handlers_tests::MockHandler;

#[test]
fn test_clear_handler_commands_and_description() {
    // Verify static metadata via trait interface
    let h = MockHandler {
        cmds: vec!["clear"],
        desc: "清除 system prompt 静态层缓存并触发重建",
        imm: true,
        reply_text: String::new(),
    };
    assert_eq!(h.commands(), &["clear"]);
    assert_eq!(h.description(), "清除 system prompt 静态层缓存并触发重建");
    assert!(h.immediate("clear"));
}

#[test]
fn test_dispatcher_is_immediate_unknown() {
    let registry = HandlerRegistry::new();
    let dispatcher = SlashDispatcher::new(registry);
    assert!(!dispatcher.is_immediate("nonexistent"));
}

#[test]
fn test_dispatcher_all_handlers() {
    let registry = HandlerRegistry::new();
    registry.register(Arc::new(MockHandler {
        cmds: vec!["x"],
        desc: "x desc",
        imm: false,
        reply_text: String::new(),
    }));
    registry.register(Arc::new(MockHandler {
        cmds: vec!["y"],
        desc: "y desc",
        imm: true,
        reply_text: String::new(),
    }));
    let dispatcher = SlashDispatcher::new(registry);
    let handlers = dispatcher.all_handlers();
    assert_eq!(handlers.len(), 2);
}

#[test]
fn test_reasoning_level_getter_setter_symmetry() {
    let mut s = closeclaw_llm::session::ConversationSession::new(
        "test-sym".to_owned(),
        "test-model".to_owned(),
        std::path::PathBuf::from("/tmp"),
    );
    assert_eq!(s.reasoning_level(), ReasoningLevel::High);
    for &lv in &[
        ReasoningLevel::Low,
        ReasoningLevel::Medium,
        ReasoningLevel::High,
        ReasoningLevel::Max,
    ] {
        s.set_reasoning_level(lv);
        assert_eq!(s.reasoning_level(), lv);
    }
}

#[test]
fn test_reasoning_level_with_builder() {
    let s = closeclaw_llm::session::ConversationSession::new(
        "test-b".to_owned(),
        "test-model".to_owned(),
        std::path::PathBuf::from("/tmp"),
    )
    .with_reasoning_level(ReasoningLevel::High);
    assert_eq!(s.reasoning_level(), ReasoningLevel::High);
}
