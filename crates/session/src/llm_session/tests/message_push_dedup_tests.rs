//! Tests for the user-message push dedup guard in
//! `build_llm_messages_with_listing` (plan Step 1.5).
//!
//! The gateway production path pre-appends the user message to history
//! before invoking; the build-side push must not add a second copy
//! (the sent list must stay the Thinking-cleaned append-style history,
//! which the cache prefix check's tail-append judgment relies on).
//!
//! Covered behavior dimensions:
//! 1. Production path, two rounds (pre-append + invoke full chain):
//!    round 2 is a tail-append extension → eligible → the numeric check
//!    fires on a whole-round cache miss.
//! 2. Callers that did not pre-append still get the push (empty
//!    history, assistant tail, different content, role mismatch).
//! 3. Consecutive identical texts in different rounds are not
//!    mistakenly deleted (each round keeps exactly one copy).

use std::sync::Arc;

use async_trait::async_trait;

use crate::llm_session::session_chat::ChatSession;
use crate::llm_session::ConversationSession;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::{
    ContentBlock, InternalMessage, LLMError, LlmCaller, StreamEvent, UnifiedResponse, UnifiedUsage,
};

use super::tmp_path;

/// Provider cache TTL magnitude default (cache-adapter doc :27: 5 min).
const TTL_MS: u64 = 300_000;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_session() -> ConversationSession {
    ConversationSession::new("message-push-dedup".into(), "test-model".into(), tmp_path())
        .with_system_prompt("static system")
}

fn make_usage(prompt: u32, completion: u32, cache_read: Option<u32>) -> UnifiedUsage {
    UnifiedUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: Some(prompt + completion),
        reasoning_tokens: None,
        cache_read_tokens: cache_read,
        cache_write_tokens: None,
    }
}

fn canned_response(text: &str) -> UnifiedResponse {
    UnifiedResponse {
        content_blocks: vec![ContentBlock::Text(text.into())],
        usage: make_usage(100_000, 50, Some(100_000)),
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn user_msg(content: &str) -> InternalMessage {
    InternalMessage {
        role: "user".to_string(),
        content: content.to_string(),
        content_blocks: None,
        tool_call_id: None,
    }
}

fn assistant_msg(content: &str) -> InternalMessage {
    InternalMessage {
        role: "assistant".to_string(),
        content: content.to_string(),
        content_blocks: None,
        tool_call_id: None,
    }
}

/// Fake caller returning a canned response and capturing the last
/// request (default trait `call_with_target` reports no target —
/// fake-path degradation must not panic).
struct RecordingCaller {
    response: UnifiedResponse,
    last_request: std::sync::Mutex<Option<InternalRequest>>,
}

impl RecordingCaller {
    fn new(response: UnifiedResponse) -> Arc<Self> {
        Arc::new(Self {
            response,
            last_request: std::sync::Mutex::new(None),
        })
    }

    fn last_request(&self) -> Option<InternalRequest> {
        self.last_request.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmCaller for RecordingCaller {
    async fn call(&self, request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        *self.last_request.lock().unwrap() = Some(request);
        Ok(self.response.clone())
    }

    async fn call_streaming(
        &self,
        _request: InternalRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamEvent, LLMError>> + Send>>,
        LLMError,
    > {
        Err(LLMError::ApiError("not implemented".into()))
    }
}

// ---------------------------------------------------------------------------
// ① Production path: pre-append + invoke, two rounds
// ---------------------------------------------------------------------------

/// 生产路径两轮对话（预 append + invoke 全链路）：第二轮快照判定为参与
/// （Eligible），整轮未命中时数值校验可触发。
#[tokio::test]
async fn production_path_two_rounds_round2_eligible_and_numeric_check_fires() {
    let mut session = make_session();
    let caller = RecordingCaller::new(canned_response("ok"));
    session.set_llm_caller(caller.clone());

    // ── Round 1: gateway pre-appends the user message, then invokes ──
    session.append_user_message("round-1 question");
    let _ = session.invoke_llm_with_target("round-1 question").await;
    let sent1 = caller
        .last_request()
        .expect("round 1 request captured")
        .messages;
    assert_eq!(
        sent1,
        vec![user_msg("round-1 question")],
        "round 1 sends exactly one copy of the pre-appended message"
    );

    // Unified advancement point: judge + advance + accumulate.
    let usage1 = make_usage(100_000, 50, Some(100_000));
    let info1 = session.check_cache_anomaly_and_advance(&usage1, Some("p/m"), now_ms(), TTL_MS);
    assert!(info1.is_none(), "round 1 is a new baseline");
    session.accumulate_usage(&usage1, Some("p/m"));

    // ── Round 2: assistant response in history, pre-append, invoke ──
    session.append_response(canned_response("ok"));
    session.append_user_message("round-2 question");
    let _ = session.invoke_llm_with_target("round-2 question").await;
    let sent2 = caller
        .last_request()
        .expect("round 2 request captured")
        .messages;
    assert_eq!(
        sent2,
        vec![
            user_msg("round-1 question"),
            assistant_msg("ok"),
            user_msg("round-2 question"),
        ],
        "round 2 sends the appended history plus one copy of the new message"
    );

    // Whole-round cache miss on a comparable round → numeric check fires.
    let usage2 = make_usage(100_000, 50, Some(0));
    let info2 = session.check_cache_anomaly_and_advance(&usage2, Some("p/m"), now_ms(), TTL_MS);
    let info = info2
        .expect("round 2 must be eligible (tail-append extension): the numeric check must run");
    assert_eq!(info.previous_prompt_tokens, 100_000);
    assert_eq!(info.current_cache_read_tokens, 0);
    assert_eq!(info.deviation_tokens, 100_000);
    session.accumulate_usage(&usage2, Some("p/m"));
}

// ---------------------------------------------------------------------------
// ② Callers that did not pre-append still get the push
// ---------------------------------------------------------------------------

/// 空 history：消息照常 push。
#[test]
fn build_pushes_into_empty_history() {
    let session = make_session();
    let messages = session.build_llm_messages_with_listing("hello", None);
    assert_eq!(messages, vec![user_msg("hello")]);
}

/// history 尾部为 assistant：消息照常 push（未预 append 调用方）。
#[test]
fn build_pushes_when_tail_is_assistant() {
    let mut session = make_session();
    session.append_response(canned_response("prior response"));
    let messages = session.build_llm_messages_with_listing("new question", None);
    assert_eq!(
        messages,
        vec![assistant_msg("prior response"), user_msg("new question")]
    );
}

/// history 尾部 user 内容不同：消息照常 push。
#[test]
fn build_pushes_when_tail_content_differs() {
    let mut session = make_session();
    session.append_user_message("earlier question");
    let messages = session.build_llm_messages_with_listing("different question", None);
    assert_eq!(
        messages,
        vec![user_msg("earlier question"), user_msg("different question")]
    );
}

/// history 尾部 role 不同（内容碰巧相同）：消息照常 push。
#[test]
fn build_pushes_when_tail_role_differs() {
    let mut session = make_session();
    session.append_transcript("assistant", vec![ContentBlock::Text("same text".into())]);
    let messages = session.build_llm_messages_with_listing("same text", None);
    assert_eq!(
        messages,
        vec![assistant_msg("same text"), user_msg("same text")]
    );
}

/// 守卫生效边界：history 尾部与待 push 消息全等（角色+内容）→ 跳过 push。
#[test]
fn build_skips_push_when_tail_fully_equal() {
    let mut session = make_session();
    session.append_user_message("same text");
    let messages = session.build_llm_messages_with_listing("same text", None);
    assert_eq!(
        messages,
        vec![user_msg("same text")],
        "identical tail: only the pre-appended history copy is sent"
    );
}

// ---------------------------------------------------------------------------
// ③ Consecutive identical texts across rounds are not deleted
// ---------------------------------------------------------------------------

/// 两轮发送相同文本：每轮各保留一条预 append 消息，仅去掉冗余 push 副本。
#[tokio::test]
async fn consecutive_identical_texts_across_rounds_not_deleted() {
    let mut session = make_session();
    let caller = RecordingCaller::new(canned_response("ok"));
    session.set_llm_caller(caller.clone());

    session.append_user_message("hi");
    let _ = session.invoke_llm_with_target("hi").await;
    let sent1 = caller
        .last_request()
        .expect("round 1 request captured")
        .messages;
    assert_eq!(sent1, vec![user_msg("hi")], "round 1: exactly one copy");

    session.append_response(canned_response("ok"));
    session.append_user_message("hi");
    let _ = session.invoke_llm_with_target("hi").await;
    let sent2 = caller
        .last_request()
        .expect("round 2 request captured")
        .messages;
    assert_eq!(
        sent2,
        vec![user_msg("hi"), assistant_msg("ok"), user_msg("hi")],
        "round 2: both rounds' identical texts survive"
    );
}
