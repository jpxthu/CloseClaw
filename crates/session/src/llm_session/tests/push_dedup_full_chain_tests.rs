//! Full-chain regression tests for the push dedup injection rescan
//! (plan Step 1.3, issue #3072).
//!
//! Unlike the white-box guard tests in `push_dedup_injection_tests`
//! (which call `build_llm_messages_with_listing` directly), these go
//! through the real invoke path with a recording fake LLM caller, so
//! the asserted outbound request is exactly what would be sent to the
//! provider. The seam mirrors `message_push_dedup_tests` (RecordingCaller
//! + canned response).
//!
//! Covered behavior dimensions:
//! 1. Tool injection cover (#3072 reproduction): pre-append →
//!    `inject_tool_result` → declared invoke sends exactly one copy and
//!    the history itself holds exactly one copy (rules out a
//!    history-side double-append path).
//! 2. Workflow injection cover variant: same shape with
//!    `inject_workflow_message`.
//! 3. Pending-drain safety: the undeclared invoke (declaration false)
//!    behind injections still pushes — the drained message is never
//!    mistakenly deleted.
//! 4. Blocks-form pre-append (image round) under injection cover: no
//!    flattened duplicate is sent.
//!
//! The existing `consecutive_identical_texts_across_rounds_not_deleted`
//! and friends in `message_push_dedup_tests` remain the cross-round
//! regression suite and keep passing unchanged.

use std::sync::Arc;

use async_trait::async_trait;

use crate::llm_session::session_chat::ChatSession;
use crate::llm_session::ConversationSession;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::{ContentBlock, LLMError, LlmCaller, StreamEvent, UnifiedResponse};

use super::{
    make_usage_basic as make_usage, tmp_path, tool_placeholder, tool_result_msg, user_blocks_msg,
    user_msg, workflow_msg,
};

// ---------------------------------------------------------------------------

fn make_session() -> ConversationSession {
    ConversationSession::new(
        "push-dedup-full-chain".into(),
        "test-model".into(),
        tmp_path(),
    )
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

/// Number of transcript entries whose role is `user` and whose blocks
/// contain a `Text` block equal to `text` — the history-side copy
/// count of the current-turn message.
fn history_user_text_count(session: &ConversationSession, text: &str) -> usize {
    session
        .messages()
        .iter()
        .filter(|msg| msg.role == "user")
        .flat_map(|msg| msg.content_blocks.iter())
        .filter(|block| matches!(block, ContentBlock::Text(t) if t == text))
        .count()
}

// ---------------------------------------------------------------------------
// ① Tool injection cover: #3072 reproduction, full chain
// ---------------------------------------------------------------------------

/// #3072 复现回归（全链路）：预 append user 消息后 `inject_tool_result`
/// 把转换后列表尾部顶成 tool 消息（空占位 + 提升的 ToolResult），声明
/// invoke 不得再 push 副本——出站恰 1 份，且历史中该消息本身恰 1 份
/// （排除「历史双 append」并存路径）。
#[tokio::test]
async fn tool_injection_cover_sends_single_copy_and_history_holds_one() {
    let mut session = make_session();
    let caller = RecordingCaller::new(canned_response("ok"));
    session.set_llm_caller(caller.clone());

    session.append_user_message("tell me a joke");
    session.inject_tool_result("call_1", "tool output");
    let _ = session
        .invoke_llm_with_target_pre_appended_blocks("tell me a joke")
        .await;

    let sent = caller.last_request().expect("request captured").messages;
    assert_eq!(
        sent,
        vec![
            user_msg("tell me a joke"),
            tool_placeholder(),
            tool_result_msg("call_1", "tool output"),
        ],
        "injection-covered pre-appended message is sent exactly once (#3072)"
    );
    assert_eq!(
        history_user_text_count(&session, "tell me a joke"),
        1,
        "history itself carries exactly one copy: no double-append path"
    );
}

// ---------------------------------------------------------------------------
// ② Workflow injection cover variant
// ---------------------------------------------------------------------------

/// workflow 注入覆盖变体（全链路）：`inject_workflow_message` 把尾部顶成
/// workflow 消息，声明 invoke 回扫命中预 append 消息 → 出站恰 1 份。
#[tokio::test]
async fn workflow_injection_cover_sends_single_copy() {
    let mut session = make_session();
    let caller = RecordingCaller::new(canned_response("ok"));
    session.set_llm_caller(caller.clone());

    session.append_user_message("tell me a joke");
    session.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");
    let _ = session
        .invoke_llm_with_target_pre_appended_blocks("tell me a joke")
        .await;

    let sent = caller.last_request().expect("request captured").messages;
    assert_eq!(
        sent,
        vec![
            user_msg("tell me a joke"),
            workflow_msg("Verify Step 0 (Step 0):\nCheck output"),
        ],
        "workflow-covered pre-appended message is sent exactly once"
    );
    assert_eq!(
        history_user_text_count(&session, "tell me a joke"),
        1,
        "history itself carries exactly one copy"
    );
}

// ---------------------------------------------------------------------------
// ③ Pending-drain safety: undeclared invoke still pushes
// ---------------------------------------------------------------------------

/// pending drain 安全（全链路）：不声明场景（声明 false，pending drain
/// 路径）历史尾部区域为 [user("q"), tool 注入] 时直接 invoke 同文本
/// "q" → 照常 push，用户消息不被误删（跨轮安全）。
#[tokio::test]
async fn undeclared_invoke_behind_injection_keeps_push() {
    let mut session = make_session();
    let caller = RecordingCaller::new(canned_response("ok"));
    session.set_llm_caller(caller.clone());

    session.append_user_message("hi");
    session.inject_tool_result("call_1", "tool output");
    let _ = session.invoke_llm_with_target("hi").await;

    let sent = caller.last_request().expect("request captured").messages;
    assert_eq!(
        sent,
        vec![
            user_msg("hi"),
            tool_placeholder(),
            tool_result_msg("call_1", "tool output"),
            user_msg("hi"),
        ],
        "undeclared invoke pushes the drained message: no silent drop"
    );
}

// ---------------------------------------------------------------------------
// ④ Blocks-form pre-append under injection cover
// ---------------------------------------------------------------------------

/// 图轮（blocks 预 append + 注入覆盖，全链路）：Image 块排除出文本展平
/// （content=""），invoke 内容是网关展平文本；声明 invoke 回扫按声明
/// 分支命中 blocks 消息 → 不 push 展平文本副本，出站恰 1 份。
#[tokio::test]
async fn blocks_pre_append_under_injection_cover_sends_single_copy() {
    let mut session = make_session();
    let caller = RecordingCaller::new(canned_response("ok"));
    session.set_llm_caller(caller.clone());

    let blocks = vec![ContentBlock::Image {
        name: "img_1".into(),
        url: "data:image/png;base64,QUFB".into(),
    }];
    session.append_user_content_blocks(blocks.clone());
    session.inject_tool_result("call_1", "tool output");
    let _ = session
        .invoke_llm_with_target_pre_appended_blocks("[image: img_1] photo.png\nQUFB")
        .await;

    let sent = caller.last_request().expect("request captured").messages;
    assert_eq!(
        sent,
        vec![
            user_blocks_msg(&blocks),
            tool_placeholder(),
            tool_result_msg("call_1", "tool output"),
        ],
        "blocks-form pre-append under the cover sends no flattened duplicate"
    );
    assert_eq!(
        sent[0].content, "",
        "image blocks stay out of the text flatten"
    );
}
