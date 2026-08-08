// Unit tests for SlashResultExecutor — covers all SlashResult variant execute() behavior.
//
// Uses MockSlashEffectExecutor to verify side-effect dispatch and
// MockSessionLookup for session queries.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::executor::{
    CompactionError, CompactionResult, ReplyAction, SideEffectContext, SlashEffectExecutor,
    SlashResultExecutor,
};
use crate::processor::ContentBlock;
use crate::session_lookup::{PendingMessage, SessionLookup};
use crate::slash_router::{SlashResult, SystemAppendAction};
use crate::{ReasoningLevel, VerbosityLevel};

// ── Mock SlashEffectExecutor ──────────────────────────────────────────

/// Call recorded by mock executor for assertion.
#[derive(Debug, Clone, PartialEq)]
enum ExecutorCall {
    Stop(String, bool, bool),
    NewSession(String, String),
    Compact(String, Option<String>),
    SystemAppend(String, SystemAppendAction),
    SetReasoning(String, ReasoningLevel),
    SetVerbosity(String, VerbosityLevel),
    SetMode(String, String),
    Exec(String, String, String),
}

struct MockSlashEffectExecutor {
    calls: Arc<Mutex<Vec<ExecutorCall>>>,
    reply_rx: mpsc::Receiver<ReplyAction>,
    reply_tx: mpsc::Sender<ReplyAction>,
}

impl MockSlashEffectExecutor {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel(32);
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            reply_rx: rx,
            reply_tx: tx,
        }
    }

    /// Drain all pending ReplyActions from the receiver.
    async fn drain_replies(&mut self) -> Vec<ReplyAction> {
        let mut out = Vec::new();
        while let Ok(action) = self.reply_rx.try_recv() {
            out.push(action);
        }
        out
    }
}

#[async_trait]
impl SlashEffectExecutor for MockSlashEffectExecutor {
    async fn execute_stop(&self, session_id: &str, cascade: bool, force: bool) {
        self.calls
            .lock()
            .unwrap()
            .push(ExecutorCall::Stop(session_id.to_string(), cascade, force));
    }

    async fn execute_new_session(&self, session_id: &str, channel: &str) -> String {
        self.calls.lock().unwrap().push(ExecutorCall::NewSession(
            session_id.to_string(),
            channel.to_string(),
        ));
        "new-session-id".to_string()
    }

    async fn execute_compact(
        &self,
        session_id: &str,
        instruction: Option<String>,
    ) -> Result<CompactionResult, CompactionError> {
        self.calls.lock().unwrap().push(ExecutorCall::Compact(
            session_id.to_string(),
            instruction.clone(),
        ));
        Ok(CompactionResult {
            performed: true,
            original_tokens: 1000,
            compacted_tokens: 500,
            message: "Compacted".to_string(),
            before_char_count: 10000,
            after_char_count: 5000,
            before_token_count: 1000,
            after_token_count: 500,
            boundary_message: String::new(),
            is_auto: false,
        })
    }

    async fn execute_system_append(&self, session_id: &str, action: &SystemAppendAction) -> usize {
        self.calls.lock().unwrap().push(ExecutorCall::SystemAppend(
            session_id.to_string(),
            action.clone(),
        ));
        1
    }

    async fn execute_set_reasoning(&self, session_id: &str, level: ReasoningLevel) {
        self.calls
            .lock()
            .unwrap()
            .push(ExecutorCall::SetReasoning(session_id.to_string(), level));
    }

    async fn execute_set_verbosity(&self, session_id: &str, level: VerbosityLevel) {
        self.calls
            .lock()
            .unwrap()
            .push(ExecutorCall::SetVerbosity(session_id.to_string(), level));
    }

    async fn execute_set_mode(&self, session_id: &str, mode: &str) {
        self.calls.lock().unwrap().push(ExecutorCall::SetMode(
            session_id.to_string(),
            mode.to_string(),
        ));
    }

    async fn execute_exec(
        &self,
        session_id: &str,
        agent_id: &str,
        command: &str,
    ) -> Vec<ContentBlock> {
        self.calls.lock().unwrap().push(ExecutorCall::Exec(
            session_id.to_string(),
            agent_id.to_string(),
            command.to_string(),
        ));
        vec![ContentBlock::Text(format!("output: {command}"))]
    }
}

// ── Mock SessionLookup ────────────────────────────────────────────────

struct MockSessionLookup {
    pending_messages: Arc<Mutex<Vec<PendingMessage>>>,
    chat_id: Option<String>,
}

impl MockSessionLookup {
    fn new(chat_id: Option<String>) -> Self {
        Self {
            pending_messages: Arc::new(Mutex::new(Vec::new())),
            chat_id,
        }
    }

    fn with_pending(pending: Arc<Mutex<Vec<PendingMessage>>>) -> Self {
        Self {
            pending_messages: pending,
            chat_id: None,
        }
    }
}

#[async_trait]
impl SessionLookup for MockSessionLookup {
    async fn get_parent_of(&self, _child_id: &str) -> Option<String> {
        None
    }

    async fn get_chat_id(&self, _session_id: &str) -> Option<String> {
        self.chat_id.clone()
    }

    async fn push_pending_message(
        &self,
        _session_id: &str,
        msg: PendingMessage,
    ) -> Result<(), String> {
        self.pending_messages.lock().unwrap().push(msg);
        Ok(())
    }

    async fn get_plan_state(&self, _session_id: &str) -> Option<crate::PlanState> {
        None
    }

    async fn set_plan_state(&self, _session_id: &str, _plan_state: crate::PlanState) {}

    async fn set_session_mode(&self, _session_id: &str, _mode: crate::SessionMode) {}
}

// ── Helper to build SideEffectContext ─────────────────────────────────

fn make_ctx(
    executor: &MockSlashEffectExecutor,
    session_id: &str,
    channel: &str,
    session_manager: Arc<dyn SessionLookup>,
) -> SideEffectContext {
    SideEffectContext {
        session_id: session_id.to_string(),
        channel: channel.to_string(),
        session_manager,
        reply_tx: executor.reply_tx.clone(),
        executor: Arc::new(MockSlashEffectExecutorRef {
            calls: executor.calls.clone(),
        }),
    }
}

/// A thin wrapper that shares the same Arc<Mutex<Vec>> as the real mock
/// so the executor recorded by SideEffectContext writes to the same log.
struct MockSlashEffectExecutorRef {
    calls: Arc<Mutex<Vec<ExecutorCall>>>,
}

#[async_trait]
impl SlashEffectExecutor for MockSlashEffectExecutorRef {
    async fn execute_stop(&self, session_id: &str, cascade: bool, force: bool) {
        self.calls
            .lock()
            .unwrap()
            .push(ExecutorCall::Stop(session_id.to_string(), cascade, force));
    }

    async fn execute_new_session(&self, session_id: &str, channel: &str) -> String {
        self.calls.lock().unwrap().push(ExecutorCall::NewSession(
            session_id.to_string(),
            channel.to_string(),
        ));
        "new-session-id".to_string()
    }

    async fn execute_compact(
        &self,
        session_id: &str,
        instruction: Option<String>,
    ) -> Result<CompactionResult, CompactionError> {
        self.calls.lock().unwrap().push(ExecutorCall::Compact(
            session_id.to_string(),
            instruction.clone(),
        ));
        Ok(CompactionResult {
            performed: true,
            original_tokens: 1000,
            compacted_tokens: 500,
            message: "Compacted".to_string(),
            before_char_count: 10000,
            after_char_count: 5000,
            before_token_count: 1000,
            after_token_count: 500,
            boundary_message: String::new(),
            is_auto: false,
        })
    }

    async fn execute_system_append(&self, session_id: &str, action: &SystemAppendAction) -> usize {
        self.calls.lock().unwrap().push(ExecutorCall::SystemAppend(
            session_id.to_string(),
            action.clone(),
        ));
        1
    }

    async fn execute_set_reasoning(&self, session_id: &str, level: ReasoningLevel) {
        self.calls
            .lock()
            .unwrap()
            .push(ExecutorCall::SetReasoning(session_id.to_string(), level));
    }

    async fn execute_set_verbosity(&self, session_id: &str, level: VerbosityLevel) {
        self.calls
            .lock()
            .unwrap()
            .push(ExecutorCall::SetVerbosity(session_id.to_string(), level));
    }

    async fn execute_set_mode(&self, session_id: &str, mode: &str) {
        self.calls.lock().unwrap().push(ExecutorCall::SetMode(
            session_id.to_string(),
            mode.to_string(),
        ));
    }

    async fn execute_exec(
        &self,
        session_id: &str,
        agent_id: &str,
        command: &str,
    ) -> Vec<ContentBlock> {
        self.calls.lock().unwrap().push(ExecutorCall::Exec(
            session_id.to_string(),
            agent_id.to_string(),
            command.to_string(),
        ));
        vec![ContentBlock::Text(format!("output: {command}"))]
    }
}

// ── Test: Reply variant ───────────────────────────────────────────────

#[tokio::test]
async fn test_reply_produces_reply_action_with_correct_text() {
    let mock = MockSlashEffectExecutor::new();
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(&mock, "s1", "feishu", sm);

    SlashResult::Reply("hello world".into()).execute(&ctx).await;

    let mut mock2 = mock;
    let replies = mock2.drain_replies().await;
    assert_eq!(replies.len(), 1);
    match &replies[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0], ContentBlock::Text("hello world".into()));
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
    assert!(mock2.calls.lock().unwrap().is_empty());
}

// ── Test: Stop variant ────────────────────────────────────────────────

#[tokio::test]
async fn test_stop_calls_execute_stop_with_correct_params() {
    let mock = MockSlashEffectExecutor::new();
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(&mock, "s2", "feishu", sm);

    SlashResult::Stop {
        cascade: true,
        force: false,
    }
    .execute(&ctx)
    .await;

    let calls = mock.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], ExecutorCall::Stop("s2".into(), true, false));
    drop(calls);

    let mut mock2 = mock;
    let replies = mock2.drain_replies().await;
    assert_eq!(replies.len(), 1);
    match &replies[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks[0], ContentBlock::Text("已停止当前任务".into()));
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}

// ── Test: SetMode with initial_input ──────────────────────────────────

#[tokio::test]
async fn test_set_mode_with_initial_input_sets_mode_and_injects_pending() {
    let mock = MockSlashEffectExecutor::new();
    let pending = Arc::new(Mutex::new(Vec::<PendingMessage>::new()));
    let sm: Arc<dyn SessionLookup> = Arc::new(MockSessionLookup::with_pending(pending.clone()));
    let ctx = make_ctx(&mock, "s3", "feishu", sm);

    SlashResult::SetMode {
        mode: "plan".into(),
        plan_file_path: None,
        initial_input: Some("do something".into()),
        reply_message: None,
    }
    .execute(&ctx)
    .await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::SetMode("s3".into(), "plan".into())
    );

    let msgs = pending.lock().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "do something");
    assert_eq!(msgs[0].role.as_deref(), Some("user"));
}

// ── Test: SetMode without initial_input ───────────────────────────────

#[tokio::test]
async fn test_set_mode_without_initial_input_no_pending_message() {
    let mock = MockSlashEffectExecutor::new();
    let pending = Arc::new(Mutex::new(Vec::<PendingMessage>::new()));
    let sm: Arc<dyn SessionLookup> = Arc::new(MockSessionLookup::with_pending(pending.clone()));
    let ctx = make_ctx(&mock, "s4", "feishu", sm);

    SlashResult::SetMode {
        mode: "auto".into(),
        plan_file_path: None,
        initial_input: None,
        reply_message: Some("已切换到 auto 模式".into()),
    }
    .execute(&ctx)
    .await;

    assert!(pending.lock().unwrap().is_empty());
}

// ── Test: SetMode custom reply_message ────────────────────────────────

#[tokio::test]
async fn test_set_mode_custom_reply_message() {
    let mock = MockSlashEffectExecutor::new();
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(&mock, "s5", "feishu", sm);

    SlashResult::SetMode {
        mode: "plan".into(),
        plan_file_path: None,
        initial_input: None,
        reply_message: Some("自定义回复".into()),
    }
    .execute(&ctx)
    .await;

    let mut mock2 = mock;
    let replies = mock2.drain_replies().await;
    assert_eq!(replies.len(), 1);
    match &replies[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks[0], ContentBlock::Text("自定义回复".into()));
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: NewSession variant ──────────────────────────────────────────

#[tokio::test]
async fn test_new_session_creates_session_and_replies() {
    let mock = MockSlashEffectExecutor::new();
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(&mock, "s6", "telegram", sm);

    SlashResult::NewSession.execute(&ctx).await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::NewSession("s6".into(), "telegram".into())
    );

    let mut mock2 = mock;
    let replies = mock2.drain_replies().await;
    match &replies[0] {
        ReplyAction::Reply(blocks) => match &blocks[0] {
            ContentBlock::Text(t) => assert!(t.contains("new-session-id")),
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: Compact success ─────────────────────────────────────────────

#[tokio::test]
async fn test_compact_success_replies_with_message() {
    let mock = MockSlashEffectExecutor::new();
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(&mock, "s7", "feishu", sm);

    SlashResult::Compact {
        instruction: Some("summarize".into()),
    }
    .execute(&ctx)
    .await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::Compact("s7".into(), Some("summarize".into()))
    );

    let mut mock2 = mock;
    let replies = mock2.drain_replies().await;
    match &replies[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks[0], ContentBlock::Text("Compacted".into()));
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: Compact error ───────────────────────────────────────────────

#[tokio::test]
async fn test_compact_error_replies_with_failure_message() {
    // Build a mock that returns an error from execute_compact
    let mock = MockSlashEffectExecutorError;
    let sm = Arc::new(MockSessionLookup::new(None));
    let (tx, mut rx) = mpsc::channel::<ReplyAction>(32);
    let ctx = SideEffectContext {
        session_id: "s8".into(),
        channel: "feishu".into(),
        session_manager: sm,
        reply_tx: tx,
        executor: Arc::new(mock),
    };

    SlashResult::Compact { instruction: None }
        .execute(&ctx)
        .await;

    let reply = rx.try_recv().unwrap();
    match reply {
        ReplyAction::Reply(blocks) => match &blocks[0] {
            ContentBlock::Text(t) => assert!(t.starts_with("Compact failed:")),
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Reply, got {other:?}"),
    }
}

/// Mock that always returns a compact error.
struct MockSlashEffectExecutorError;

#[async_trait]
impl SlashEffectExecutor for MockSlashEffectExecutorError {
    async fn execute_stop(&self, _: &str, _: bool, _: bool) {}
    async fn execute_new_session(&self, _: &str, _: &str) -> String {
        String::new()
    }
    async fn execute_compact(
        &self,
        _: &str,
        _: Option<String>,
    ) -> Result<CompactionResult, CompactionError> {
        Err(CompactionError::LLMCallFailed("mock failure".into()))
    }
    async fn execute_system_append(&self, _: &str, _: &SystemAppendAction) -> usize {
        0
    }
    async fn execute_set_reasoning(&self, _: &str, _: ReasoningLevel) {}
    async fn execute_set_verbosity(&self, _: &str, _: VerbosityLevel) {}
    async fn execute_set_mode(&self, _: &str, _: &str) {}
    async fn execute_exec(&self, _: &str, _: &str, _: &str) -> Vec<ContentBlock> {
        Vec::new()
    }
}

// ── Test: SystemAppend Add ────────────────────────────────────────────

#[tokio::test]
async fn test_system_append_add_replies_with_index() {
    let mock = MockSlashEffectExecutor::new();
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(&mock, "s9", "feishu", sm);

    SlashResult::SystemAppend {
        action: SystemAppendAction::Add("be helpful".into()),
    }
    .execute(&ctx)
    .await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::SystemAppend("s9".into(), SystemAppendAction::Add("be helpful".into()))
    );

    let mut mock2 = mock;
    let replies = mock2.drain_replies().await;
    match &replies[0] {
        ReplyAction::Reply(blocks) => match &blocks[0] {
            ContentBlock::Text(t) => assert!(t.contains("#1")),
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: SystemAppend Clear ──────────────────────────────────────────

#[tokio::test]
async fn test_system_append_clear_replies_with_count() {
    let mock = MockSlashEffectExecutor::new();
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(&mock, "s10", "feishu", sm);

    SlashResult::SystemAppend {
        action: SystemAppendAction::Clear,
    }
    .execute(&ctx)
    .await;

    let mut mock2 = mock;
    let replies = mock2.drain_replies().await;
    match &replies[0] {
        ReplyAction::Reply(blocks) => match &blocks[0] {
            ContentBlock::Text(t) => assert!(t.contains("1")),
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: Exec variant ────────────────────────────────────────────────

#[tokio::test]
async fn test_exec_calls_execute_exec_and_replies() {
    let mock = MockSlashEffectExecutor::new();
    let sm = Arc::new(MockSessionLookup::new(Some("agent-42".into())));
    let ctx = make_ctx(&mock, "s11", "feishu", sm);

    SlashResult::Exec {
        command: "ls -la".into(),
        requires_permission: true,
    }
    .execute(&ctx)
    .await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::Exec("s11".into(), "agent-42".into(), "ls -la".into())
    );

    let mut mock2 = mock;
    let replies = mock2.drain_replies().await;
    match &replies[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks[0], ContentBlock::Text("output: ls -la".into()));
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: Exec with no chat_id fallback ───────────────────────────────

#[tokio::test]
async fn test_exec_falls_back_to_empty_agent_id() {
    let mock = MockSlashEffectExecutor::new();
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(&mock, "s12", "feishu", sm);

    SlashResult::Exec {
        command: "pwd".into(),
        requires_permission: false,
    }
    .execute(&ctx)
    .await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::Exec("s12".into(), String::new(), "pwd".into())
    );
}

// ── Test: SetReasoning variant ────────────────────────────────────────

#[tokio::test]
async fn test_set_reasoning_calls_executor_and_replies() {
    let mock = MockSlashEffectExecutor::new();
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(&mock, "s13", "feishu", sm);

    SlashResult::SetReasoning {
        level: ReasoningLevel::Max,
    }
    .execute(&ctx)
    .await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::SetReasoning("s13".into(), ReasoningLevel::Max)
    );

    let mut mock2 = mock;
    let replies = mock2.drain_replies().await;
    match &replies[0] {
        ReplyAction::Reply(blocks) => match &blocks[0] {
            ContentBlock::Text(t) => assert!(t.contains("Max")),
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: SetVerbosity variant ────────────────────────────────────────

#[tokio::test]
async fn test_set_verbosity_calls_executor_and_replies() {
    let mock = MockSlashEffectExecutor::new();
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(&mock, "s14", "feishu", sm);

    SlashResult::SetVerbosity {
        level: VerbosityLevel::Off,
    }
    .execute(&ctx)
    .await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::SetVerbosity("s14".into(), VerbosityLevel::Off)
    );

    let mut mock2 = mock;
    let replies = mock2.drain_replies().await;
    match &replies[0] {
        ReplyAction::Reply(blocks) => match &blocks[0] {
            ContentBlock::Text(t) => assert!(t.contains("off")),
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: InjectMeta variant ──────────────────────────────────────────

#[tokio::test]
async fn test_inject_meta_appends_system_and_replies() {
    let mock = MockSlashEffectExecutor::new();
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(&mock, "s15", "feishu", sm);

    SlashResult::InjectMeta {
        content: "skill body here".into(),
    }
    .execute(&ctx)
    .await;

    assert_eq!(
        mock.calls.lock().unwrap()[0],
        ExecutorCall::SystemAppend(
            "s15".into(),
            SystemAppendAction::Add("skill body here".into())
        )
    );

    let mut mock2 = mock;
    let replies = mock2.drain_replies().await;
    match &replies[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks[0], ContentBlock::Text("技能已加载".into()));
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Test: Unknown variant ─────────────────────────────────────────────

#[tokio::test]
async fn test_unknown_command_replies_with_error() {
    let mock = MockSlashEffectExecutor::new();
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(&mock, "s16", "feishu", sm);

    SlashResult::Unknown("nonexistent".into())
        .execute(&ctx)
        .await;

    let mut mock2 = mock;
    let replies = mock2.drain_replies().await;
    assert_eq!(replies.len(), 1);
    match &replies[0] {
        ReplyAction::Reply(blocks) => match &blocks[0] {
            ContentBlock::Text(t) => assert_eq!(t, "Unknown command: /nonexistent"),
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Reply, got {other:?}"),
    }
    assert!(mock2.calls.lock().unwrap().is_empty());
}

// ── Test: PermissionOp is no-op ──────────────────────────────────────

#[tokio::test]
async fn test_permission_op_is_noop() {
    use crate::permission_op::PermissionOperation;

    let mock = MockSlashEffectExecutor::new();
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(&mock, "s17", "feishu", sm);

    SlashResult::PermissionOp {
        op: PermissionOperation::AddFileWhitelist {
            agent: "test-agent".into(),
            op: "read".into(),
            paths: vec![],
        },
    }
    .execute(&ctx)
    .await;

    let mut mock2 = mock;
    assert!(mock2.calls.lock().unwrap().is_empty());
    assert!(mock2.drain_replies().await.is_empty());
}

// ── Test: UserApprove is no-op ───────────────────────────────────────

#[tokio::test]
async fn test_user_approve_is_noop() {
    let mock = MockSlashEffectExecutor::new();
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(&mock, "s18", "feishu", sm);

    SlashResult::UserApprove {
        request_id: "req-1".into(),
        initial_permissions: vec![],
    }
    .execute(&ctx)
    .await;

    let mut mock2 = mock;
    assert!(mock2.calls.lock().unwrap().is_empty());
    assert!(mock2.drain_replies().await.is_empty());
}

// ── Test: UserReject is no-op ────────────────────────────────────────

#[tokio::test]
async fn test_user_reject_is_noop() {
    let mock = MockSlashEffectExecutor::new();
    let sm = Arc::new(MockSessionLookup::new(None));
    let ctx = make_ctx(&mock, "s19", "feishu", sm);

    SlashResult::UserReject {
        request_id: "req-2".into(),
    }
    .execute(&ctx)
    .await;

    let mut mock2 = mock;
    assert!(mock2.calls.lock().unwrap().is_empty());
    assert!(mock2.drain_replies().await.is_empty());
}
