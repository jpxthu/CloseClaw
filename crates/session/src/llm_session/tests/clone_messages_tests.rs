use super::*;
use closeclaw_common::UnifiedResponse;

#[test]
fn test_clone_messages_from() {
    let mut source_session = ConversationSession::new("src_1".into(), "gpt-4o".into(), tmp_path());
    source_session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("hello from parent".into())],
        usage: UnifiedUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: Some(2),
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
    });
    source_session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("second response".into())],
        usage: UnifiedUsage {
            prompt_tokens: 2,
            completion_tokens: 1,
            total_tokens: Some(3),
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
    });

    let mut child_session = ConversationSession::new("child_1".into(), "gpt-4o".into(), tmp_path());
    assert_eq!(child_session.messages().len(), 0);

    child_session.clone_messages_from(&source_session.messages());

    assert_eq!(child_session.messages().len(), 2);
    assert_eq!(child_session.messages()[0].role, "assistant");
    assert_eq!(child_session.messages()[1].role, "assistant");
}

#[test]
fn test_clone_messages_from_empty() {
    let mut child_session =
        ConversationSession::new("child_empty".into(), "gpt-4o".into(), tmp_path());
    assert_eq!(child_session.messages().len(), 0);

    child_session.clone_messages_from(&[]);

    // No side effects: still empty.
    assert_eq!(child_session.messages().len(), 0);
}

#[test]
fn test_clone_messages_from_preserves_timestamp() {
    let ts1 = Utc::now();
    let ts2 = Utc::now();
    let source_msgs = vec![
        SessionMessage {
            role: "user".into(),
            content_blocks: vec![ContentBlock::Text("msg1".into())],
            timestamp: ts1,
        },
        SessionMessage {
            role: "assistant".into(),
            content_blocks: vec![ContentBlock::Text("msg2".into())],
            timestamp: ts2,
        },
    ];

    let mut child_session =
        ConversationSession::new("child_ts".into(), "gpt-4o".into(), tmp_path());
    child_session.clone_messages_from(&source_msgs);

    assert_eq!(child_session.messages().len(), 2);
    assert_eq!(child_session.messages()[0].timestamp, ts1);
    assert_eq!(child_session.messages()[1].timestamp, ts2);
}
