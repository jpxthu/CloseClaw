use super::*;

// ── find_assistant_text_by_content ─────────────────────────────────────────

#[test]
fn test_find_assistant_text_by_content_match() {
    use closeclaw_common::ContentBlock;

    let messages = vec![
        SessionMessage {
            role: "user".into(),
            content_blocks: vec![ContentBlock::Text("hello".into())],
            timestamp: chrono::Utc::now(),
        },
        SessionMessage {
            role: "assistant".into(),
            content_blocks: vec![ContentBlock::Text("response text".into())],
            timestamp: chrono::Utc::now(),
        },
    ];

    let result = ConversationSession::find_assistant_text_by_content(&messages, "response text");
    assert_eq!(result.as_deref(), Some("response text"));
}

#[test]
fn test_find_assistant_text_by_content_no_match() {
    use closeclaw_common::ContentBlock;

    let messages = vec![SessionMessage {
        role: "assistant".into(),
        content_blocks: vec![ContentBlock::Text("other text".into())],
        timestamp: chrono::Utc::now(),
    }];

    let result = ConversationSession::find_assistant_text_by_content(&messages, "target text");
    assert!(result.is_none());
}

#[test]
fn test_find_assistant_text_by_content_empty_messages() {
    let messages: Vec<SessionMessage> = vec![];
    let result = ConversationSession::find_assistant_text_by_content(&messages, "anything");
    assert!(result.is_none());
}

#[test]
fn test_find_assistant_text_by_content_ignores_non_assistant() {
    use closeclaw_common::ContentBlock;

    let messages = vec![SessionMessage {
        role: "user".into(),
        content_blocks: vec![ContentBlock::Text("target text".into())],
        timestamp: chrono::Utc::now(),
    }];

    let result = ConversationSession::find_assistant_text_by_content(&messages, "target text");
    assert!(result.is_none());
}

#[test]
fn test_find_assistant_text_by_content_multi_block() {
    use closeclaw_common::ContentBlock;

    let messages = vec![SessionMessage {
        role: "assistant".into(),
        content_blocks: vec![
            ContentBlock::Text("part1".into()),
            ContentBlock::Thinking {
                thinking: "thinking".into(),
                signature: None,
            },
            ContentBlock::Text("part2".into()),
        ],
        timestamp: chrono::Utc::now(),
    }];

    // Text blocks are joined; Thinking blocks are excluded.
    let result = ConversationSession::find_assistant_text_by_content(&messages, "part1\npart2");
    assert_eq!(result.as_deref(), Some("part1\npart2"));
}
