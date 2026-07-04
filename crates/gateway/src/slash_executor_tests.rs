//! Unit tests for [`SlashResultExecutor`] (migrated from common crate).
//!
//! Covers the execute side-effect dispatch for all [`SlashResult`] variants:
//! - Reply — sends text block
//! - SetMode — sends confirmation, no executor call
//! - NewSession — calls executor, sends reply
//! - Stop — calls executor, sends reply
//! - Unknown — sends unknown command text

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::slash_executor::{ReplyAction, SideEffectContext, SlashEffectExecutor};
use closeclaw_common::processor::ContentBlock;
use closeclaw_common::session_lookup::{PendingMessage, SessionLookup};
use closeclaw_common::session_types::ReasoningLevel;
use closeclaw_common::slash_router::{SlashResult, SystemAppendAction};
use closeclaw_common::verbosity::VerbosityLevel;

use crate::slash_executor::SlashResultExecutor;

// ---------------------------------------------------------------------------
// Mock implementations
// ---------------------------------------------------------------------------

struct MockSessionLookup;

#[async_trait]
impl SessionLookup for MockSessionLookup {
    async fn get_parent_of(&self, _child_id: &str) -> Option<String> {
        None
    }
    async fn get_chat_id(&self, _session_id: &str) -> Option<String> {
        Some("agent-007".into())
    }
    async fn push_pending_message(
        &self,
        _session_id: &str,
        _msg: PendingMessage,
    ) -> Result<(), String> {
        Ok(())
    }
}

/// Records every executor call so tests can assert on them.
struct MockExecutor {
    new_session_called: std::sync::Mutex<bool>,
    stop_called: std::sync::Mutex<bool>,
    compact_called: std::sync::Mutex<bool>,
    compact_instruction: std::sync::Mutex<Option<String>>,
    system_append_called: std::sync::Mutex<bool>,
    system_append_action: std::sync::Mutex<Option<SystemAppendAction>>,
    set_reasoning_called: std::sync::Mutex<bool>,
    set_verbosity_called: std::sync::Mutex<bool>,
    exec_output: Vec<ContentBlock>,
}

impl MockExecutor {
    fn new() -> Self {
        Self {
            new_session_called: std::sync::Mutex::new(false),
            stop_called: std::sync::Mutex::new(false),
            compact_called: std::sync::Mutex::new(false),
            compact_instruction: std::sync::Mutex::new(None),
            system_append_called: std::sync::Mutex::new(false),
            system_append_action: std::sync::Mutex::new(None),
            set_reasoning_called: std::sync::Mutex::new(false),
            set_verbosity_called: std::sync::Mutex::new(false),
            exec_output: vec![ContentBlock::Text("exec output".into())],
        }
    }
}

#[async_trait]
impl SlashEffectExecutor for MockExecutor {
    async fn execute_stop(&self, _session_id: &str) {
        *self.stop_called.lock().unwrap() = true;
    }

    async fn execute_new_session(&self, _session_id: &str, _channel: &str) {
        *self.new_session_called.lock().unwrap() = true;
    }

    async fn execute_compact(&self, _session_id: &str, instruction: Option<String>) {
        *self.compact_called.lock().unwrap() = true;
        *self.compact_instruction.lock().unwrap() = instruction;
    }

    async fn execute_system_append(&self, _session_id: &str, action: &SystemAppendAction) {
        *self.system_append_called.lock().unwrap() = true;
        *self.system_append_action.lock().unwrap() = Some(action.clone());
    }

    async fn execute_set_reasoning(&self, _session_id: &str, _level: ReasoningLevel) {
        *self.set_reasoning_called.lock().unwrap() = true;
    }

    async fn execute_set_verbosity(&self, _session_id: &str, _level: VerbosityLevel) {
        *self.set_verbosity_called.lock().unwrap() = true;
    }

    async fn execute_exec(
        &self,
        _session_id: &str,
        _agent_id: &str,
        _command: &str,
    ) -> Vec<ContentBlock> {
        self.exec_output.clone()
    }
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn make_ctx() -> (
    SideEffectContext,
    mpsc::Receiver<ReplyAction>,
    Arc<MockExecutor>,
) {
    let (tx, rx) = mpsc::channel(16);
    let executor = Arc::new(MockExecutor::new());
    let ctx = SideEffectContext {
        session_id: "sess-1".into(),
        channel: "feishu".into(),
        session_manager: Arc::new(MockSessionLookup),
        reply_tx: tx,
        executor: executor.clone(),
    };
    (ctx, rx, executor)
}

async fn drain_actions(rx: &mut mpsc::Receiver<ReplyAction>) -> Vec<ReplyAction> {
    let mut actions = Vec::new();
    while let Some(a) = rx.recv().await {
        actions.push(a);
    }
    actions
}

// ---------------------------------------------------------------------------
// Tests — Reply
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_reply_sends_text_block() {
    let (ctx, mut rx, _exec) = make_ctx();
    SlashResult::Reply("hello world".into()).execute(&ctx).await;
    drop(ctx);

    let actions = drain_actions(&mut rx).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert!(matches!(&blocks[0], ContentBlock::Text(t) if t == "hello world"));
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Tests — SetMode
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_set_mode_sends_confirmation() {
    let (ctx, mut rx, _exec) = make_ctx();
    SlashResult::SetMode("plan".into()).execute(&ctx).await;
    drop(ctx);

    let actions = drain_actions(&mut rx).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert!(matches!(&blocks[0], ContentBlock::Text(t) if t == "Mode set to: plan"));
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Tests — NewSession
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_new_session_calls_executor_and_sends_reply() {
    let (ctx, mut rx, exec) = make_ctx();
    SlashResult::NewSession.execute(&ctx).await;
    drop(ctx);

    assert!(*exec.new_session_called.lock().unwrap());

    let actions = drain_actions(&mut rx).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert!(matches!(&blocks[0], ContentBlock::Text(t) if t == "已创建新 session"));
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Tests — Stop
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_stop_calls_executor_and_sends_reply() {
    let (ctx, mut rx, exec) = make_ctx();
    SlashResult::Stop.execute(&ctx).await;
    drop(ctx);

    assert!(*exec.stop_called.lock().unwrap());

    let actions = drain_actions(&mut rx).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert!(matches!(&blocks[0], ContentBlock::Text(t) if t == "已停止当前任务"));
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Tests — Compact
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_compact_calls_executor_and_sends_reply() {
    let (ctx, mut rx, exec) = make_ctx();
    SlashResult::Compact { instruction: None }
        .execute(&ctx)
        .await;
    drop(ctx);

    assert!(*exec.compact_called.lock().unwrap());
    assert_eq!(*exec.compact_instruction.lock().unwrap(), None);

    let actions = drain_actions(&mut rx).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert!(matches!(&blocks[0], ContentBlock::Text(t) if t == "对话历史已压缩"));
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_compact_with_instruction_sends_reply() {
    let (ctx, mut rx, exec) = make_ctx();
    SlashResult::Compact {
        instruction: Some("keep recent".into()),
    }
    .execute(&ctx)
    .await;
    drop(ctx);

    assert!(*exec.compact_called.lock().unwrap());
    assert_eq!(
        *exec.compact_instruction.lock().unwrap(),
        Some("keep recent".into())
    );

    let actions = drain_actions(&mut rx).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert!(matches!(&blocks[0], ContentBlock::Text(t) if t == "对话历史已压缩"));
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Tests — SystemAppend
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_system_append_add_calls_executor_and_sends_reply() {
    let (ctx, mut rx, exec) = make_ctx();
    SlashResult::SystemAppend {
        action: SystemAppendAction::Add("rule: be concise".into()),
    }
    .execute(&ctx)
    .await;
    drop(ctx);

    assert!(*exec.system_append_called.lock().unwrap());
    assert!(matches!(
        *exec.system_append_action.lock().unwrap(),
        Some(SystemAppendAction::Add(ref s)) if s == "rule: be concise"
    ));

    let actions = drain_actions(&mut rx).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert!(matches!(&blocks[0], ContentBlock::Text(t) if t == "已追加指令"));
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_system_append_clear_calls_executor_and_sends_reply() {
    let (ctx, mut rx, exec) = make_ctx();
    SlashResult::SystemAppend {
        action: SystemAppendAction::Clear,
    }
    .execute(&ctx)
    .await;
    drop(ctx);

    assert!(*exec.system_append_called.lock().unwrap());
    assert!(matches!(
        *exec.system_append_action.lock().unwrap(),
        Some(SystemAppendAction::Clear)
    ));

    let actions = drain_actions(&mut rx).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert!(matches!(&blocks[0], ContentBlock::Text(t) if t == "已清除追加指令"));
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Tests — Exec
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_exec_calls_execute_exec_and_sends_result() {
    let (ctx, mut rx, _exec) = make_ctx();
    SlashResult::Exec {
        command: "ls -la".into(),
    }
    .execute(&ctx)
    .await;
    drop(ctx);

    let actions = drain_actions(&mut rx).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert!(matches!(&blocks[0], ContentBlock::Text(t) if t == "exec output"));
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Tests — Unknown
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_unknown_sends_unknown_command_text() {
    let (ctx, mut rx, _exec) = make_ctx();
    SlashResult::Unknown("nope".into()).execute(&ctx).await;
    drop(ctx);

    let actions = drain_actions(&mut rx).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert!(matches!(&blocks[0], ContentBlock::Text(t) if t == "Unknown command: /nope"));
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Tests — SetReasoning
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_set_reasoning_calls_executor_and_sends_reply() {
    let (ctx, mut rx, exec) = make_ctx();
    SlashResult::SetReasoning {
        level: ReasoningLevel::Max,
    }
    .execute(&ctx)
    .await;
    drop(ctx);

    assert!(*exec.set_reasoning_called.lock().unwrap());

    let actions = drain_actions(&mut rx).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert!(matches!(&blocks[0], ContentBlock::Text(t) if t == "推理深度已设置为 max"));
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Tests — SetVerbosity
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_set_verbosity_calls_executor_and_sends_reply() {
    let (ctx, mut rx, exec) = make_ctx();
    SlashResult::SetVerbosity {
        level: VerbosityLevel::Off,
    }
    .execute(&ctx)
    .await;
    drop(ctx);

    assert!(*exec.set_verbosity_called.lock().unwrap());

    let actions = drain_actions(&mut rx).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert!(matches!(&blocks[0], ContentBlock::Text(t) if t == "输出详细度已设置为 off"));
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}
