use super::*;
use crate::persistence::PendingMessage;
use crate::run_health::TranscriptOp;
use closeclaw_common::{ContentBlock, UnifiedResponse, UnifiedUsage};

mod announce_dedup_tests;
mod announce_queue_tests;
mod append_messages_tests;
mod bootstrap_mode_tests;
mod build_api_request_tests;
mod cache_prefix_check_tests;
mod child_busy_tracking_tests;
mod clone_messages_tests;
mod compaction_protection_tests;
mod deferred_mode_switch_tests;
mod dynamic_layer_tests;
mod exec_state_tests;
mod extract_pending_tool_calls_tests;
mod graceful_stop_pending_tool_tests;
mod kill_doubles;
mod llm_busy_count_tests;
mod llm_gate_check_tests;
mod log_capture;
use self::log_capture::{capture_logs, capture_logs_async, is_installed};
mod log_capture_tests;
mod manual_background_tests;
mod memory_injection_dedup_tests;
mod message_push_dedup_tests;
mod pending_state_tests;
mod progress_removal_session_tests;
mod reasoning_level_tests;
mod rebuild_system_prompt_tests;
mod recent_tool_calls_tests;
mod session_exec_tests;
mod session_injection_role_tests;
mod session_llm_tests;
mod skill_listing_incremental_tests;
mod skill_listing_injection_event_tests;
mod skill_listing_tests;
mod step1_3_conditional_injection_tests;
mod step1_4_behavior_tests;
mod step1_5_ut_tests;
mod stop_kill_path_tests;
mod stop_tests;
mod streaming_tests;
mod system_appends_tests;
mod test_helpers;
use self::test_helpers::{
    assistant_msg, make_usage, make_usage_basic, now_ms, snapshot, user_msg, TTL_MS,
};
mod unified_queue_tests;
mod waiting_state_tests;
mod workflow_session_tests;

// ── apply_transcript_op() ──────────────────────────────────────────────

#[test]
fn test_apply_transcript_op_overwrites_existing() {
    let mut session = ConversationSession::new("s3".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("old".into())],
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
    session.apply_transcript_op(TranscriptOp::Rewrite, new_msgs);
    assert_eq!(session.messages().len(), 2);
    assert_eq!(session.messages()[0].role, "user");
    assert_eq!(session.messages()[1].role, "assistant");
}

#[test]
fn test_apply_transcript_op_empty_vec_clears() {
    let mut session = ConversationSession::new("s4".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("msg".into())],
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
    assert_eq!(session.messages().len(), 1);
    session.apply_transcript_op(TranscriptOp::Rewrite, vec![]);
    assert_eq!(session.messages().len(), 0);
}

// ── apply_transcript_op snapshot behavior ───────────────────────────────

#[test]
fn test_apply_transcript_op_rewrite_creates_snapshot() {
    let mut session = ConversationSession::new("s5".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("before".into())],
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
    assert_eq!(session.snapshot_count(), None);
    session.apply_transcript_op(
        TranscriptOp::Rewrite,
        vec![SessionMessage {
            role: "user".into(),
            content_blocks: vec![ContentBlock::Text("after".into())],
            timestamp: Utc::now(),
        }],
    );
    assert_eq!(session.snapshot_count(), Some(1));
    assert_eq!(session.messages().len(), 1);
    assert_eq!(
        session.messages()[0].content_blocks[0],
        ContentBlock::Text("after".into())
    );
}

#[test]
fn test_apply_transcript_op_append_no_snapshot() {
    let mut session = ConversationSession::new("s6".into(), "gpt-4o".into(), tmp_path());
    session.append_transcript("user", vec![ContentBlock::Text("hello".into())]);
    assert_eq!(session.snapshot_count(), None);
    assert_eq!(session.messages().len(), 1);
}

#[test]
fn test_apply_transcript_op_partial_rewrite_creates_snapshot() {
    let mut session = ConversationSession::new("s7".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("before".into())],
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
    assert_eq!(session.snapshot_count(), None);
    session.apply_transcript_op(
        TranscriptOp::PartialRewrite,
        vec![SessionMessage {
            role: "system".into(),
            content_blocks: vec![ContentBlock::Text("new system".into())],
            timestamp: Utc::now(),
        }],
    );
    assert_eq!(session.snapshot_count(), Some(1));
    assert_eq!(session.messages().len(), 1);
    assert_eq!(session.messages()[0].role, "system");
}

mod file_mtime_tests;
mod file_read_range_tests;
mod kv_cache_tests;
mod last_message_at_persist_tests;
mod stats_integration;
mod thinking_clean_tests;
mod tool_session_persist_tests;

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
