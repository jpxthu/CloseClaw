use super::*;
use crate::llm::types::UnifiedUsage;
use crate::session::persistence::PendingMessage;
use std::sync::Arc;
use std::thread;

// ── llm_busy state ─────────────────────────────────────────────────────────

#[test]
fn test_is_llm_busy_default_false() {
    let session = ConversationSession::new("sess_busy".into(), "gpt-4o".into());
    assert!(!session.is_llm_busy());
}

#[test]
fn test_set_llm_busy_true() {
    let session = ConversationSession::new("sess_busy".into(), "gpt-4o".into());
    session.set_llm_busy(true);
    assert!(session.is_llm_busy());
}

#[test]
fn test_set_llm_busy_false_recovers() {
    let session = ConversationSession::new("sess_busy".into(), "gpt-4o".into());
    session.set_llm_busy(true);
    session.set_llm_busy(false);
    assert!(!session.is_llm_busy());
}

#[test]
fn test_set_llm_busy_concurrent_no_panic() {
    let session = Arc::new(ConversationSession::new(
        "sess_concurrent".into(),
        "gpt-4o".into(),
    ));
    let handles: Vec<_> = (0..4)
        .map(|i| {
            let s = Arc::clone(&session);
            thread::spawn(move || {
                s.set_llm_busy(i % 2 == 0);
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread panicked");
    }
}

// ── pending_messages queue ──────────────────────────────────────────────────

#[test]
fn test_pending_initial_state() {
    let session = ConversationSession::new("sess_pending".into(), "gpt-4o".into());
    assert_eq!(session.pending_count(), 0);
    assert!(!session.has_pending());
}

#[test]
fn test_push_pending_sets_has_pending_and_increments_count() {
    let mut session = ConversationSession::new("sess_pending".into(), "gpt-4o".into());
    assert_eq!(session.pending_count(), 0);
    session.push_pending(PendingMessage::new("msg_1".into(), "hello".into()));
    assert!(session.has_pending());
    assert_eq!(session.pending_count(), 1);
    session.push_pending(PendingMessage::new("msg_2".into(), "world".into()));
    assert_eq!(session.pending_count(), 2);
}

#[test]
fn test_pop_pending_fifo_order() {
    let mut session = ConversationSession::new("sess_fifo".into(), "gpt-4o".into());
    session.push_pending(PendingMessage::new("msg_A".into(), "first".into()));
    session.push_pending(PendingMessage::new("msg_B".into(), "second".into()));
    let first = session.pop_pending();
    assert!(first.is_some());
    assert_eq!(first.unwrap().message_id, "msg_A");
    let second = session.pop_pending();
    assert!(second.is_some());
    assert_eq!(second.unwrap().message_id, "msg_B");
}

#[test]
fn test_pop_pending_returns_none_when_empty() {
    let mut session = ConversationSession::new("sess_empty".into(), "gpt-4o".into());
    assert!(session.pop_pending().is_none());
}

#[test]
fn test_get_pending_messages_does_not_consume_queue() {
    let mut session = ConversationSession::new("sess_get".into(), "gpt-4o".into());
    session.push_pending(PendingMessage::new("msg_1".into(), "hello".into()));
    session.push_pending(PendingMessage::new("msg_2".into(), "world".into()));

    let msgs = session.get_pending_messages();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].message_id, "msg_1");
    assert_eq!(msgs[1].message_id, "msg_2");

    // Queue is unmodified after get_pending_messages
    assert_eq!(session.pending_count(), 2);

    // Verify they are clones (can modify returned Vec without affecting queue)
    let popped = session.pop_pending().unwrap();
    assert_eq!(popped.message_id, "msg_1");
    assert_eq!(session.pending_count(), 1);
}

#[test]
fn test_get_pending_messages_empty() {
    let session = ConversationSession::new("sess_empty_get".into(), "gpt-4o".into());
    let msgs = session.get_pending_messages();
    assert!(msgs.is_empty());
}

#[test]
fn test_restore_pending_messages_only_sent_false() {
    let mut session = ConversationSession::new("sess_restore".into(), "gpt-4o".into());

    let mut sent_msg = PendingMessage::new("sent_1".into(), "already sent".into());
    sent_msg.mark_sent();
    let unsent_msg = PendingMessage::new("unsent_1".into(), "not sent yet".into());

    session.restore_pending_messages(vec![sent_msg, unsent_msg]);

    assert_eq!(session.pending_count(), 1);
    let restored = session.pop_pending().unwrap();
    assert_eq!(restored.message_id, "unsent_1");
    assert!(!restored.sent);
}

#[test]
fn test_restore_pending_messages_skips_all_sent() {
    let mut session = ConversationSession::new("sess_restore_skip".into(), "gpt-4o".into());

    let mut msg1 = PendingMessage::new("a".into(), "a".into());
    msg1.mark_sent();
    let mut msg2 = PendingMessage::new("b".into(), "b".into());
    msg2.mark_sent();

    session.restore_pending_messages(vec![msg1, msg2]);
    assert_eq!(session.pending_count(), 0);
}

#[test]
fn test_restore_pending_messages_empty_vec_no_op() {
    let mut session = ConversationSession::new("sess_restore_empty".into(), "gpt-4o".into());
    session.push_pending(PendingMessage::new("existing".into(), "existing".into()));

    session.restore_pending_messages(vec![]);
    assert_eq!(session.pending_count(), 1);
    let popped = session.pop_pending().unwrap();
    assert_eq!(popped.message_id, "existing");
}

// ── SessionMessage serde roundtrip ────────────────────────────────────────

#[test]
fn test_session_message_serde_roundtrip() {
    let msg = SessionMessage {
        role: "user".into(),
        content_blocks: vec![
            ContentBlock::Text("hello".into()),
            ContentBlock::ToolUse {
                id: "call_1".into(),
                name: "get_weather".into(),
                input: r#"{"city":"Tokyo"}"#.into(),
            },
        ],
        timestamp: Utc::now(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: SessionMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(msg.role, parsed.role);
    assert_eq!(msg.content_blocks, parsed.content_blocks);
}

// ── ConversationSession initial state ─────────────────────────────────────

#[test]
fn test_conversation_session_new() {
    let session = ConversationSession::new("sess_42".into(), "gpt-4o".into());
    assert_eq!(session.messages().len(), 0);
    assert_eq!(session.turn_count(), 0);
    assert!(session.system_prompt().is_none());
}

// ── append_response adds a message ─────────────────────────────────────────

#[test]
fn test_append_response_adds_message() {
    let mut session = ConversationSession::new("sess_1".into(), "gpt-4o".into());
    let response = UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("Hi there!".into())],
        usage: UnifiedUsage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: Some(3),
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    };
    session.append_response(response);
    assert_eq!(session.messages().len(), 1);
    assert_eq!(session.messages()[0].role, "assistant");
}

// ── append_tool_result increments turn ────────────────────────────────────

#[test]
fn test_append_tool_result_increments_turn() {
    let mut session = ConversationSession::new("sess_2".into(), "gpt-4o".into());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("Using tool...".into())],
        usage: UnifiedUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: Some(2),
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    });
    assert_eq!(session.turn_count(), 0);
    session.append_tool_result("call_x".into(), "tool output".into());
    assert_eq!(session.turn_count(), 1);
}

// ── append_response with empty blocks does NOT increment turn ──────────────

#[test]
fn test_append_response_empty_blocks_no_turn_increment() {
    let mut session = ConversationSession::new("sess_3".into(), "gpt-4o".into());
    session.append_response(UnifiedResponse {
        content_blocks: vec![],
        usage: UnifiedUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: Some(0),
            reasoning_tokens: None,
        },
        finish_reason: None,
    });
    assert_eq!(session.messages().len(), 0);
    assert_eq!(session.turn_count(), 0);
}

// ── build_api_request with system_prompt ───────────────────────────────────

#[test]
fn test_build_api_request_includes_system_prompt() {
    let session = ConversationSession::new("sess_4".into(), "gpt-4o".into())
        .with_system_prompt("You are helpful.");
    let req = session.build_api_request();
    assert!(req
        .messages
        .iter()
        .any(|m| m.role == "system" && m.content.contains("helpful")));
}

// ── build_api_request without system_prompt ────────────────────────────────

#[test]
fn test_build_api_request_without_system_prompt() {
    let mut session = ConversationSession::new("sess_5".into(), "gpt-4o".into());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("Who are you?".into())],
        usage: UnifiedUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: Some(2),
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    });
    let req = session.build_api_request();
    assert!(!req.messages.is_empty());
    assert!(!req.messages.iter().any(|m| m.role == "system"));
}

// ── Multiple turns ──────────────────────────────────────────────────────────

#[test]
fn test_conversation_session_multiple_turns() {
    let mut session = ConversationSession::new("sess_6".into(), "gpt-4o".into());

    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("First response".into())],
        usage: UnifiedUsage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: Some(3),
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    });
    assert_eq!(session.messages().len(), 1);
    assert_eq!(session.turn_count(), 0);

    session.append_tool_result("call_1".into(), "result A".into());
    assert_eq!(session.turn_count(), 1);

    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("Second response".into())],
        usage: UnifiedUsage {
            prompt_tokens: 1,
            completion_tokens: 3,
            total_tokens: Some(4),
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    });
    assert_eq!(session.messages().len(), 2);
    assert_eq!(session.turn_count(), 1);

    session.append_tool_result("call_2".into(), "result B".into());
    assert_eq!(session.turn_count(), 2);
    assert_eq!(session.messages().len(), 2);
}

// ── model() getter ───────────────────────────────────────────────────────

#[test]
fn test_model_returns_model_name() {
    let session = ConversationSession::new("s1".into(), "glm-5.1".into());
    assert_eq!(session.model(), "glm-5.1");
}

#[test]
fn test_model_returns_empty_string() {
    let session = ConversationSession::new("s2".into(), String::new());
    assert_eq!(session.model(), "");
}

// ── replace_messages() ───────────────────────────────────────────────────

#[test]
fn test_replace_messages_overwrites_existing() {
    let mut session = ConversationSession::new("s3".into(), "gpt-4o".into());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("old".into())],
        usage: UnifiedUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: Some(2),
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    });
    assert_eq!(session.messages().len(), 1);

    let new_msgs = vec![
        SessionMessage {
            role: "user".into(),
            content_blocks: vec![ContentBlock::Text("new user".into())],
            timestamp: Utc::now(),
        },
        SessionMessage {
            role: "assistant".into(),
            content_blocks: vec![ContentBlock::Text("new asst".into())],
            timestamp: Utc::now(),
        },
    ];
    session.replace_messages(new_msgs);
    assert_eq!(session.messages().len(), 2);
    assert_eq!(session.messages()[0].role, "user");
    assert_eq!(session.messages()[1].role, "assistant");
}

#[test]
fn test_replace_messages_empty_vec_clears() {
    let mut session = ConversationSession::new("s4".into(), "gpt-4o".into());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("msg".into())],
        usage: UnifiedUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: Some(2),
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".into()),
    });
    assert_eq!(session.messages().len(), 1);
    session.replace_messages(vec![]);
    assert_eq!(session.messages().len(), 0);
}
