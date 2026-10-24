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
//! 4. Image turns (blocks-form pre-append): the explicit
//!    pre-appended-blocks declaration dedups the flattened-text push;
//!    the round after an image turn is a tail-append extension again
//!    (eligible for the cache check); callers without the declaration
//!    and declaration-vs-reality mismatches still push.

use std::sync::Arc;

use async_trait::async_trait;

use crate::llm_session::session_chat::ChatSession;
use crate::llm_session::ConversationSession;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::{
    ContentBlock, InternalMessage, LLMError, LlmCaller, StreamEvent, UnifiedResponse,
};

use super::{assistant_msg, make_usage_basic as make_usage, now_ms, tmp_path, user_msg, TTL_MS};

// ---------------------------------------------------------------------------

fn make_session() -> ConversationSession {
    ConversationSession::new("message-push-dedup".into(), "test-model".into(), tmp_path())
        .with_system_prompt("static system")
}

fn canned_response(text: &str) -> UnifiedResponse {
    UnifiedResponse {
        content_blocks: vec![ContentBlock::Text(text.into())],
        usage: make_usage(100_000, 50, Some(100_000)),
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
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
async fn test_production_path_two_rounds_round2_eligible_and_numeric_check_fires() {
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
fn test_build_pushes_into_empty_history() {
    let session = make_session();
    let messages = session.build_llm_messages_with_listing("hello", None, false);
    assert_eq!(messages, vec![user_msg("hello")]);
}

/// history 尾部为 assistant：消息照常 push（未预 append 调用方）。
#[test]
fn test_build_pushes_when_tail_is_assistant() {
    let mut session = make_session();
    session.append_response(canned_response("prior response"));
    let messages = session.build_llm_messages_with_listing("new question", None, false);
    assert_eq!(
        messages,
        vec![assistant_msg("prior response"), user_msg("new question")]
    );
}

/// history 尾部 user 内容不同：消息照常 push。
#[test]
fn test_build_pushes_when_tail_content_differs() {
    let mut session = make_session();
    session.append_user_message("earlier question");
    let messages = session.build_llm_messages_with_listing("different question", None, false);
    assert_eq!(
        messages,
        vec![user_msg("earlier question"), user_msg("different question")]
    );
}

/// history 尾部 role 不同（内容碰巧相同）：消息照常 push。
#[test]
fn test_build_pushes_when_tail_role_differs() {
    let mut session = make_session();
    session.append_transcript("assistant", vec![ContentBlock::Text("same text".into())]);
    let messages = session.build_llm_messages_with_listing("same text", None, false);
    assert_eq!(
        messages,
        vec![assistant_msg("same text"), user_msg("same text")]
    );
}

/// 守卫生效边界：history 尾部与待 push 消息全等（角色+内容）→ 跳过 push。
#[test]
fn test_build_skips_push_when_tail_fully_equal() {
    let mut session = make_session();
    session.append_user_message("same text");
    let messages = session.build_llm_messages_with_listing("same text", None, false);
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
async fn test_consecutive_identical_texts_across_rounds_not_deleted() {
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

// ---------------------------------------------------------------------------
// ④ Image turns: blocks-form pre-append (explicit declaration)
// ---------------------------------------------------------------------------

/// Small inline image: the gateway pre-appends this exact block shape
/// (`build_context_content_blocks` → `ContentBlock::Image` with data URI)
/// while the invoke content is the gateway-flattened text
/// `「[image: key] 后缀\nbase64」` (`format_media_tokens_with_inline`).
fn inline_image_blocks() -> Vec<ContentBlock> {
    vec![ContentBlock::Image {
        name: "img_1".into(),
        url: "data:image/png;base64,QUFB".into(),
    }]
}

/// The blocks-form tail as it appears in the sent list: Image blocks are
/// preserved as structured `content_blocks` and excluded from the text
/// flatten (`convert_history_to_internal`), so `content` is empty.
fn user_blocks_msg(blocks: &[ContentBlock]) -> InternalMessage {
    InternalMessage {
        role: "user".to_string(),
        content: String::new(),
        content_blocks: Some(blocks.to_vec()),
        tool_call_id: None,
    }
}

/// 图轮（blocks 预 append + invoke 全链路，带显式声明）：发送列表无临时副本。
///
/// 历史尾部为 blocks 形态（Image 块被排除出文本展平 → content=""），
/// invoke 待发内容是 media 展平文本「[image: key] 后缀\nbase64」——字符串
/// 全等结构性不命中；调用方以 pre-appended-blocks 变体显式声明「待发内容
/// 已在历史尾部」，守卫按同一逻辑消息跳过 push。
#[tokio::test]
async fn test_image_round_blocks_pre_append_sends_no_duplicate_copy() {
    let mut session = make_session();
    let caller = RecordingCaller::new(canned_response("ok"));
    session.set_llm_caller(caller.clone());

    let blocks = inline_image_blocks();
    session.append_user_content_blocks(blocks.clone());
    let flattened = "[image: img_1] photo.png\nQUFB";
    let _ = session
        .invoke_llm_with_target_pre_appended_blocks(flattened)
        .await;
    let sent = caller
        .last_request()
        .expect("image round request captured")
        .messages;
    assert_eq!(
        sent,
        vec![user_blocks_msg(&blocks)],
        "image round sends exactly the pre-appended blocks message, no flattened duplicate"
    );
    assert_eq!(
        sent[0].content, "",
        "image blocks stay out of the text flatten"
    );
}

/// 图轮的下一轮（纯文本追加）恢复尾部追加判定 Eligible：走预 append +
/// invoke 全链路，第二轮快照判定为参与，数值校验可触发。
#[tokio::test]
async fn test_round_after_image_turn_is_tail_append_eligible() {
    let mut session = make_session();
    let caller = RecordingCaller::new(canned_response("ok"));
    session.set_llm_caller(caller.clone());

    // ── Round 1: image turn (blocks pre-append + declared invoke) ──
    let blocks = inline_image_blocks();
    session.append_user_content_blocks(blocks.clone());
    let _ = session
        .invoke_llm_with_target_pre_appended_blocks("[image: img_1] photo.png\nQUFB")
        .await;
    let usage1 = make_usage(100_000, 50, Some(100_000));
    let info1 = session.check_cache_anomaly_and_advance(&usage1, Some("p/m"), now_ms(), TTL_MS);
    assert!(info1.is_none(), "image round is a new baseline");
    session.accumulate_usage(&usage1, Some("p/m"));

    // ── Round 2: plain text turn (text pre-append + plain invoke) ──
    session.append_response(canned_response("ok"));
    session.append_user_message("follow-up question");
    let _ = session.invoke_llm_with_target("follow-up question").await;
    let sent2 = caller
        .last_request()
        .expect("round 2 request captured")
        .messages;
    assert_eq!(
        sent2,
        vec![
            user_blocks_msg(&blocks),
            assistant_msg("ok"),
            user_msg("follow-up question"),
        ],
        "round 2 sends the appended history plus one copy of the new text message"
    );

    // Tail-append extension of round 1 → eligible → numeric check fires.
    let usage2 = make_usage(100_000, 50, Some(0));
    let info2 = session.check_cache_anomaly_and_advance(&usage2, Some("p/m"), now_ms(), TTL_MS);
    let info = info2.expect("round after the image turn must be eligible (tail-append extension)");
    assert_eq!(info.previous_prompt_tokens, 100_000);
    assert_eq!(info.current_cache_read_tokens, 0);
    assert_eq!(info.deviation_tokens, 100_000);
    session.accumulate_usage(&usage2, Some("p/m"));
}

/// 未预 append 调用方（pending drain 等，无显式声明）在 blocks 尾部仍照常
/// push——声明缺省时守卫不得误删待发消息。
#[test]
fn test_build_pushes_when_tail_is_blocks_without_declaration() {
    let mut session = make_session();
    session.append_user_content_blocks(inline_image_blocks());
    let messages =
        session.build_llm_messages_with_listing("[image: img_1] photo.png\nQUFB", None, false);
    assert_eq!(
        messages,
        vec![
            user_blocks_msg(&inline_image_blocks()),
            user_msg("[image: img_1] photo.png\nQUFB"),
        ],
        "without the declaration the flattened text is pushed as usual"
    );
}

/// 守卫收紧（Step 1.7 ③）：blocks 尾部带非空展平文本（图 + 文字消息）时，
/// 即使有显式声明也照常 push——声明分支只认「空展平文本」的 blocks 尾部，
/// 防 decl 声明误删带文字的 blocks 消息（其文本可能与 invoke 内容不同）。
#[test]
fn test_pre_appended_declaration_pushes_when_blocks_tail_has_text() {
    let mut session = make_session();
    session.append_user_content_blocks(vec![
        ContentBlock::Image {
            name: "img_1".into(),
            url: "data:image/png;base64,QUFB".into(),
        },
        ContentBlock::Text("caption text".into()),
    ]);
    let messages = session.build_llm_messages_with_listing(
        "[image: img_1] photo.png\nQUFB\ncaption text",
        None,
        true,
    );
    assert_eq!(
        messages.len(),
        2,
        "blocks tail with non-empty flattened text must not match the \
         declaration branch: push happens"
    );
    assert_eq!(
        messages[0].content, "caption text",
        "the blocks tail's flattened text is preserved"
    );
    assert_eq!(
        messages[1].content, "[image: img_1] photo.png\nQUFB\ncaption text",
        "the invoke content is not dropped"
    );
}

/// 声明与尾部事实不符（尾部非 blocks 形态，如压缩在 append 与 invoke 之间
/// 重写历史）→ 退回全等守卫，照常 push，待发消息不丢失。
#[tokio::test]
async fn test_pre_appended_declaration_falls_back_when_tail_not_blocks() {
    let mut session = make_session();
    let caller = RecordingCaller::new(canned_response("ok"));
    session.set_llm_caller(caller.clone());

    session.append_user_message("rewritten tail text");
    let _ = session
        .invoke_llm_with_target_pre_appended_blocks("[image: img_1] photo.png\nQUFB")
        .await;
    let sent = caller.last_request().expect("request captured").messages;
    assert_eq!(
        sent,
        vec![
            user_msg("rewritten tail text"),
            user_msg("[image: img_1] photo.png\nQUFB")
        ],
        "declaration without a blocks tail falls back to the equality guard: push happens"
    );
}
