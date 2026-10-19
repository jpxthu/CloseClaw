use super::*;
use crate::run_health::TranscriptOp;
use closeclaw_common::{ContentBlock, UnifiedResponse};

// ── apply_transcript_op() ──────────────────────────────────────────────

#[test]
fn test_apply_transcript_op_overwrites_existing() {
    let mut session = ConversationSession::new("s3".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("old".into())],
        usage: make_usage_basic(1, 1, None),
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
        usage: make_usage_basic(1, 1, None),
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
        usage: make_usage_basic(1, 1, None),
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
        usage: make_usage_basic(1, 1, None),
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
