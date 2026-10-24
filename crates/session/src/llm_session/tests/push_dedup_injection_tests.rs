//! White-box guard tests for the push dedup injection rescan (plan
//! Step 1.2, issue #3072).
//!
//! When the caller declares `current_turn_pre_appended`, injected
//! messages (`inject_tool_result` / `inject_workflow_message` /
//! `inject_system_message`) may legally sit between the pre-appended
//! current-turn user message and the history tail; the guard must
//! scan past them instead of blinding on the tail (the #3072 double
//! send). Without the declaration the guard stays tail-only (no
//! rescan) so the pending drain never loses a same-text message from
//! an earlier round.
//!
//! Covered behavior dimensions:
//! 1. Injection cover (tool / workflow / system / stacked): declared
//!    invoke skips the push — exactly one copy of the pre-appended
//!    message is sent.
//! 2. Under the cover, a non-equal invoke content still pushes.
//! 3. Declaration false keeps the tail-only judgment: push behind
//!    injections (cross-round safety).
//! 4. Empty `tool` placeholder shape: `inject_tool_result` converts
//!    to an empty placeholder tool message plus the promoted
//!    ToolResult message — both are skipped by the rescan.
//! 5. The rescan stops at the first non-injection message: an
//!    assistant message there (role mismatch) still pushes; a blocks
//!    declaration form behind the cover still matches.
//! 6. Corner: declared invoke on an empty history — the rescan finds
//!    no non-injection message (find → None) and still pushes exactly
//!    one user message.

use crate::llm_session::ConversationSession;
use closeclaw_common::{ContentBlock, InternalMessage};

use super::{tmp_path, tool_placeholder, tool_result_msg, user_blocks_msg, user_msg, workflow_msg};

// ---------------------------------------------------------------------------

fn make_session() -> ConversationSession {
    ConversationSession::new(
        "push-dedup-injection".into(),
        "test-model".into(),
        tmp_path(),
    )
}

fn system_msg(content: &str) -> InternalMessage {
    InternalMessage {
        role: "system".to_string(),
        content: content.to_string(),
        content_blocks: None,
        tool_call_id: None,
    }
}

fn inject_tool_result(session: &mut ConversationSession) {
    session.inject_tool_result("call_1", "tool output");
}

// ---------------------------------------------------------------------------
// ① Injection cover: declared invoke skips the push
// ---------------------------------------------------------------------------

/// tool 注入覆盖（#3072 复现形态）：预 append 后 `inject_tool_result`
/// 把尾部顶成 tool 消息，声明 true 时守卫回扫命中预 append 消息 → 不再
/// 双写。
#[test]
fn test_tool_injection_cover_skips_push() {
    let mut session = make_session();
    session.append_user_message("tell me a joke");
    inject_tool_result(&mut session);
    let messages = session.build_llm_messages_with_listing("tell me a joke", None, true);
    assert_eq!(
        messages,
        vec![
            user_msg("tell me a joke"),
            tool_placeholder(),
            tool_result_msg("call_1", "tool output"),
        ],
        "injection-covered pre-appended message: no duplicate push"
    );
}

/// workflow 注入覆盖变体：回扫跳过 workflow 消息命中预 append 消息。
#[test]
fn test_workflow_injection_cover_skips_push() {
    let mut session = make_session();
    session.append_user_message("tell me a joke");
    session.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");
    let messages = session.build_llm_messages_with_listing("tell me a joke", None, true);
    assert_eq!(
        messages,
        vec![
            user_msg("tell me a joke"),
            workflow_msg("Verify Step 0 (Step 0):\nCheck output"),
        ],
        "workflow-covered pre-appended message: no duplicate push"
    );
}

/// system 注入覆盖变体：回扫跳过 system 消息命中预 append 消息。
#[test]
fn test_system_injection_cover_skips_push() {
    let mut session = make_session();
    session.append_user_message("tell me a joke");
    session.inject_system_message("retry instruction".to_string());
    let messages = session.build_llm_messages_with_listing("tell me a joke", None, true);
    assert_eq!(
        messages,
        vec![user_msg("tell me a joke"), system_msg("retry instruction"),],
        "system-covered pre-appended message: no duplicate push"
    );
}

/// 连续多种注入覆盖：回扫跳过全部连续注入类消息后命中预 append 消息。
/// 注意转换排序：ToolResult 提升块统一追加在整条历史之后，故真实形态
/// 为 [user, tool 占位, system, workflow, tool(result)]。
#[test]
fn test_stacked_injections_cover_skips_push() {
    let mut session = make_session();
    session.append_user_message("tell me a joke");
    inject_tool_result(&mut session);
    session.inject_system_message("summary note".to_string());
    session.inject_workflow_message("Jump Step 0 (Step 0):\nProceed?");
    let messages = session.build_llm_messages_with_listing("tell me a joke", None, true);
    assert_eq!(
        messages,
        vec![
            user_msg("tell me a joke"),
            tool_placeholder(),
            system_msg("summary note"),
            workflow_msg("Jump Step 0 (Step 0):\nProceed?"),
            tool_result_msg("call_1", "tool output"),
        ],
        "stacked injections are all skipped by the rescan"
    );
}

// ---------------------------------------------------------------------------
// ② Under the cover: non-equal content still pushes
// ---------------------------------------------------------------------------

/// 覆盖层下内容不等价：第一条非注入消息 role==user 但内容不同 → 照常
/// push，待发消息不丢。
#[test]
fn test_injection_cover_pushes_when_content_differs() {
    let mut session = make_session();
    session.append_user_message("earlier question");
    inject_tool_result(&mut session);
    let messages = session.build_llm_messages_with_listing("different question", None, true);
    assert_eq!(
        messages,
        vec![
            user_msg("earlier question"),
            tool_placeholder(),
            tool_result_msg("call_1", "tool output"),
            user_msg("different question"),
        ],
        "mismatched user message behind the cover: push happens"
    );
}

/// 覆盖层下第一条非注入消息是 assistant（role 不匹配）→ 停止回扫并照常
/// push。
#[test]
fn test_injection_cover_stops_at_assistant_and_pushes() {
    let mut session = make_session();
    session.append_user_message("earlier question");
    session.append_transcript("assistant", vec![ContentBlock::Text("ok".into())]);
    inject_tool_result(&mut session);
    let messages = session.build_llm_messages_with_listing("new question", None, true);
    assert_eq!(
        messages,
        vec![
            user_msg("earlier question"),
            InternalMessage {
                role: "assistant".to_string(),
                content: "ok".to_string(),
                content_blocks: None,
                tool_call_id: None,
            },
            tool_placeholder(),
            tool_result_msg("call_1", "tool output"),
            user_msg("new question"),
        ],
        "assistant behind the cover stops the rescan: push happens"
    );
}

// ---------------------------------------------------------------------------
// ③ Declaration false: no rescan (cross-round safety)
// ---------------------------------------------------------------------------

/// 声明 false 不回扫：历史 [user("q"), tool 注入] 直接 invoke 同文本 "q"
/// → 照常 push，不误删（pending drain 跨轮安全）。
#[test]
fn test_declaration_false_keeps_push_behind_injections() {
    let mut session = make_session();
    session.append_user_message("hi");
    inject_tool_result(&mut session);
    let messages = session.build_llm_messages_with_listing("hi", None, false);
    assert_eq!(
        messages,
        vec![
            user_msg("hi"),
            tool_placeholder(),
            tool_result_msg("call_1", "tool output"),
            user_msg("hi"),
        ],
        "without the declaration the tail-only judgment keeps the push"
    );
}

// ---------------------------------------------------------------------------
// ④ Blocks declaration form behind the cover
// ---------------------------------------------------------------------------

/// blocks 声明形态（图轮）+ 注入覆盖：回扫命中空展平文本的 blocks 消息 →
/// 不 push 展平文本副本。
#[test]
fn test_blocks_declaration_under_injection_cover_skips_push() {
    let mut session = make_session();
    let blocks = vec![ContentBlock::Image {
        name: "img_1".into(),
        url: "data:image/png;base64,QUFB".into(),
    }];
    session.append_user_content_blocks(blocks.clone());
    inject_tool_result(&mut session);
    let flattened = "[image: img_1] photo.png\nQUFB";
    let messages = session.build_llm_messages_with_listing(flattened, None, true);
    assert_eq!(
        messages,
        vec![
            user_blocks_msg(&blocks),
            tool_placeholder(),
            tool_result_msg("call_1", "tool output"),
        ],
        "blocks declaration form matches behind the cover: no flattened duplicate"
    );
}

// ---------------------------------------------------------------------------
// ⑤ Corner: empty history with the declaration
// ---------------------------------------------------------------------------

/// 空历史 corner case（声明 true）：回扫 `find` 无非注入消息可判（返回
/// None）→ 照常 push，出站恰 1 条 user 消息，不 panic 也不静默丢弃。
#[test]
fn test_empty_history_with_declaration_pushes_single_user_message() {
    let session = make_session();
    let messages = session.build_llm_messages_with_listing("first message", None, true);
    assert_eq!(
        messages,
        vec![user_msg("first message")],
        "empty history with the declaration still pushes exactly one user message"
    );
}
