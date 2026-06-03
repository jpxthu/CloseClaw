use super::*;
use crate::llm::types::{UnifiedResponse, UnifiedUsage};
use std::path::PathBuf;

#[test]
fn test_clean_thinking_mixed_text_and_thinking_unchanged() {
    // Mixed messages: trailing Thinking on last assistant is removed
    let messages = vec![
        SessionMessage {
            role: "user".into(),
            content_blocks: vec![ContentBlock::Text("hello".into())],
            timestamp: Utc::now(),
        },
        SessionMessage {
            role: "assistant".into(),
            content_blocks: vec![
                ContentBlock::Text("reply".into()),
                ContentBlock::Thinking("thought".into()),
            ],
            timestamp: Utc::now(),
        },
    ];
    let cleaned = ConversationSession::clean_thinking_content(&messages);
    assert_eq!(cleaned.len(), 2);
    assert_eq!(cleaned[1].content_blocks.len(), 1);
    assert!(matches!(&cleaned[1].content_blocks[0], ContentBlock::Text(t) if t == "reply"));
}

#[test]
fn test_clean_thinking_pure_thinking_removed() {
    // Pure-Thinking assistant messages are removed entirely
    let messages = vec![
        SessionMessage {
            role: "user".into(),
            content_blocks: vec![ContentBlock::Text("hi".into())],
            timestamp: Utc::now(),
        },
        SessionMessage {
            role: "assistant".into(),
            content_blocks: vec![ContentBlock::Thinking("only thinking".into())],
            timestamp: Utc::now(),
        },
    ];
    let cleaned = ConversationSession::clean_thinking_content(&messages);
    assert_eq!(cleaned.len(), 1);
    assert_eq!(cleaned[0].role, "user");
}

#[test]
fn test_clean_thinking_trailing_removed_middle_kept() {
    // Trailing Thinking removed, middle Thinking preserved
    let messages = vec![SessionMessage {
        role: "assistant".into(),
        content_blocks: vec![
            ContentBlock::Text("start".into()),
            ContentBlock::Thinking("middle thought".into()),
            ContentBlock::Text("end".into()),
            ContentBlock::Thinking("trailing thought".into()),
        ],
        timestamp: Utc::now(),
    }];
    let cleaned = ConversationSession::clean_thinking_content(&messages);
    assert_eq!(cleaned.len(), 1);
    assert_eq!(cleaned[0].content_blocks.len(), 3);
    assert!(matches!(&cleaned[0].content_blocks[0], ContentBlock::Text(t) if t == "start"));
    assert!(matches!(
        &cleaned[0].content_blocks[1],
        ContentBlock::Thinking(_)
    ));
    assert!(matches!(&cleaned[0].content_blocks[2], ContentBlock::Text(t) if t == "end"));
}

#[test]
fn test_clean_thinking_all_thinking_replaced_with_empty_text() {
    // Trailing Thinking stripped; remaining Text content is kept
    let messages = vec![SessionMessage {
        role: "assistant".into(),
        content_blocks: vec![
            ContentBlock::Text("real content".into()),
            ContentBlock::Thinking("trailing".into()),
        ],
        timestamp: Utc::now(),
    }];
    let cleaned = ConversationSession::clean_thinking_content(&messages);
    assert_eq!(cleaned.len(), 1);
    assert_eq!(cleaned[0].content_blocks.len(), 1);
    assert!(matches!(&cleaned[0].content_blocks[0], ContentBlock::Text(t) if t == "real content"));
}

#[test]
fn test_clean_thinking_last_assistant_only_thinking_becomes_empty_text() {
    // Empty Text + all trailing Thinking → after trim only empty Text remains
    let messages = vec![SessionMessage {
        role: "assistant".into(),
        content_blocks: vec![
            ContentBlock::Text(String::new()),
            ContentBlock::Thinking("t1".into()),
            ContentBlock::Thinking("t2".into()),
        ],
        timestamp: Utc::now(),
    }];
    let cleaned = ConversationSession::clean_thinking_content(&messages);
    assert_eq!(cleaned.len(), 1);
    assert_eq!(cleaned[0].content_blocks.len(), 1);
    assert!(matches!(&cleaned[0].content_blocks[0], ContentBlock::Text(t) if t.is_empty()));
}

#[test]
fn test_clean_thinking_isolation_original_unchanged() {
    // clean_thinking_content does not modify original messages
    let messages = vec![SessionMessage {
        role: "assistant".into(),
        content_blocks: vec![ContentBlock::Thinking("secret".into())],
        timestamp: Utc::now(),
    }];
    let original_len = messages[0].content_blocks.len();
    let _cleaned = ConversationSession::clean_thinking_content(&messages);
    assert_eq!(messages[0].content_blocks.len(), original_len);
    assert!(matches!(&messages[0].content_blocks[0], ContentBlock::Thinking(t) if t == "secret"));
}

#[test]
fn test_build_api_request_does_not_modify_self_messages() {
    // build_api_request isolation: self.messages unchanged after call
    let mut session =
        ConversationSession::new("s_iso".into(), "gpt-4o".into(), PathBuf::from("/tmp"));
    session.append_response(UnifiedResponse {
        content_blocks: vec![
            ContentBlock::Text("hi".into()),
            ContentBlock::Thinking("think".into()),
        ],
        usage: UnifiedUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: Some(2),
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: Some("stop".into()),
    });
    let original_count = session.messages().len();
    let original_blocks = session.messages()[0].content_blocks.len();
    let _req = session.build_api_request();
    assert_eq!(session.messages().len(), original_count);
    assert_eq!(session.messages()[0].content_blocks.len(), original_blocks);
}
