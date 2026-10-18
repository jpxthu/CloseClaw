use super::*;
use crate::persistence::PendingMessage;
use crate::run_health::TranscriptOp;
use closeclaw_common::{UnifiedResponse, UnifiedUsage};

mod announce_dedup_tests;
mod announce_queue_tests;
mod append_messages_tests;
mod bootstrap_mode_tests;
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

// ── build_api_request with system_prompt ───────────────────────────────────

#[test]
fn test_build_api_request_includes_system_prompt() {
    let session = ConversationSession::new("sess_4".into(), "gpt-4o".into(), tmp_path())
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
    let mut session = ConversationSession::new("sess_5".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("Who are you?".into())],
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
    let req = session.build_api_request();
    assert!(!req.messages.is_empty());
    assert!(!req.messages.iter().any(|m| m.role == "system"));
}

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

// ── reasoning_level tests ─────────────────────────────────────────────────

use crate::persistence::ReasoningLevel;
use closeclaw_common::ContentBlock;

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

mod file_mtime_tests;
mod file_read_range_tests;
mod kv_cache_tests;
mod last_message_at_persist_tests;
mod stats_integration;
mod thinking_clean_tests;
mod tool_session_persist_tests;

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

// ── build_api_request ToolResult separation ──────────────────────────────

#[test]
fn test_build_api_request_separates_tool_result_messages() {
    let mut session = ConversationSession::new("sess_tool_sep".into(), "gpt-4o".into(), tmp_path());
    // Assistant response with text + tool use
    session.append_response(UnifiedResponse {
        content_blocks: vec![
            ContentBlock::Text("Let me check.".into()),
            ContentBlock::ToolUse {
                id: "call_1".into(),
                name: "search".into(),
                input: "query".into(),
            },
        ],
        usage: UnifiedUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: Some(2),
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: Some("tool_calls".into()),
        retry_attempts: 0,
    });
    // Append tool result
    session.append_tool_result("call_1".into(), "result: 42".into());

    let req = session.build_api_request();
    // Should have: system (none), assistant, tool = 2 messages
    assert_eq!(req.messages.len(), 2);

    // First message: assistant with non-tool blocks only
    let assistant_msg = &req.messages[0];
    assert_eq!(assistant_msg.role, "assistant");
    assert!(!assistant_msg.content.is_empty());
    assert!(assistant_msg.tool_call_id.is_none());

    // Second message: independent tool result
    let tool_msg = &req.messages[1];
    assert_eq!(tool_msg.role, "tool");
    assert_eq!(tool_msg.content, "result: 42");
    assert_eq!(tool_msg.tool_call_id.as_deref(), Some("call_1"));
}

#[test]
fn test_build_api_request_multiple_tool_results_appended_in_order() {
    let mut session =
        ConversationSession::new("sess_tool_multi".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![
            ContentBlock::Text("Thinking...".into()),
            ContentBlock::ToolUse {
                id: "call_a".into(),
                name: "tool_a".into(),
                input: "{}".into(),
            },
            ContentBlock::ToolUse {
                id: "call_b".into(),
                name: "tool_b".into(),
                input: "{}".into(),
            },
        ],
        usage: UnifiedUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: Some(2),
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: Some("tool_calls".into()),
        retry_attempts: 0,
    });
    session.append_tool_result("call_a".into(), "result_a".into());
    session.append_tool_result("call_b".into(), "result_b".into());

    let req = session.build_api_request();
    // assistant + 2 tool results = 3 messages
    assert_eq!(req.messages.len(), 3);
    assert_eq!(req.messages[0].role, "assistant");
    assert_eq!(req.messages[1].role, "tool");
    assert_eq!(req.messages[1].tool_call_id.as_deref(), Some("call_a"));
    assert_eq!(req.messages[1].content, "result_a");
    assert_eq!(req.messages[2].role, "tool");
    assert_eq!(req.messages[2].tool_call_id.as_deref(), Some("call_b"));
    assert_eq!(req.messages[2].content, "result_b");
}

#[test]
fn test_build_api_request_no_tool_result_preserves_original_behavior() {
    let mut session = ConversationSession::new("sess_no_tool".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("Hello".into())],
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

    let req = session.build_api_request();
    assert_eq!(req.messages.len(), 1);
    assert_eq!(req.messages[0].role, "assistant");
    assert_eq!(req.messages[0].content, "Hello");
    assert!(req.messages[0].tool_call_id.is_none());
}

// ── build_api_request Audio / File flatten tokens ────────────────────────

#[test]
fn test_build_api_request_audio_block_flattens_to_audio_token() {
    let mut session =
        ConversationSession::new("sess_audio_tok".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Audio {
            name: "clip.mp3".into(),
            url: "data:audio/mpeg;base64,AAA".into(),
        }],
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
    let req = session.build_api_request();
    assert_eq!(req.messages.len(), 1);
    assert_eq!(req.messages[0].content, "[audio: clip.mp3]");
    assert!(
        req.messages[0].content_blocks.is_none(),
        "Audio blocks flatten to text; only Image blocks are kept structured"
    );
}

#[test]
fn test_build_api_request_file_block_flattens_to_file_token() {
    let mut session = ConversationSession::new("sess_file_tok".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::File {
            name: "report.pdf".into(),
            url: "data:application/pdf;base64,AAA".into(),
        }],
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
    let req = session.build_api_request();
    assert_eq!(req.messages.len(), 1);
    assert_eq!(req.messages[0].content, "[file: report.pdf]");
    assert!(req.messages[0].content_blocks.is_none());
}

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
