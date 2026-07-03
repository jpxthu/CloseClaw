//! Unit tests for SlashResult::Exec and SideEffectContext.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::processor::ContentBlock;
use crate::session_lookup::SessionLookup;
use crate::slash_router::{ReplyAction, SideEffectContext, SlashEffectExecutor, SlashResult};

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
        Some("mock_chat_id".to_string())
    }
    async fn push_pending_message(
        &self,
        _session_id: &str,
        _msg: crate::session_lookup::PendingMessage,
    ) -> Result<(), String> {
        Ok(())
    }
}

/// Mock executor that tracks calls and can be configured to pass or reject.
struct MockExecutor {
    exec_call_count: Arc<AtomicUsize>,
    /// If true, execute_exec returns approval content; otherwise rejection.
    allow_exec: bool,
}

impl MockExecutor {
    fn new(allow_exec: bool) -> (Self, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        (
            Self {
                exec_call_count: counter.clone(),
                allow_exec,
            },
            counter,
        )
    }
}

#[async_trait]
impl SlashEffectExecutor for MockExecutor {
    async fn execute_stop(&self, _session_id: &str) {}
    async fn execute_new_session(&self, _session_id: &str, _channel: &str) {}
    async fn execute_compact(&self, _session_id: &str, _instruction: Option<String>) {}
    async fn execute_system_append(
        &self,
        _session_id: &str,
        _action: &crate::slash_router::SystemAppendAction,
    ) {
    }
    async fn execute_set_reasoning(
        &self,
        _session_id: &str,
        _level: crate::session_types::ReasoningLevel,
    ) {
    }
    async fn execute_set_verbosity(
        &self,
        _session_id: &str,
        _level: crate::verbosity::VerbosityLevel,
    ) {
    }

    async fn execute_exec(
        &self,
        _session_id: &str,
        _agent_id: &str,
        command: &str,
    ) -> Vec<ContentBlock> {
        self.exec_call_count.fetch_add(1, Ordering::SeqCst);
        if self.allow_exec {
            vec![ContentBlock::Text(format!("output of: {}", command))]
        } else {
            vec![ContentBlock::Text("permission denied".to_string())]
        }
    }
}

fn make_ctx(
    executor: Arc<dyn SlashEffectExecutor>,
) -> (SideEffectContext, mpsc::Receiver<ReplyAction>) {
    let (tx, rx) = mpsc::channel(16);
    let session_manager: Arc<dyn SessionLookup> = Arc::new(MockSessionLookup);
    let ctx = SideEffectContext::new(
        "sess_1".to_string(),
        "feishu".to_string(),
        session_manager,
        tx,
        executor,
    );
    (ctx, rx)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_exec_permission_pass() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (exec, call_counter) = MockExecutor::new(true);
    let executor: Arc<dyn SlashEffectExecutor> = Arc::new(exec);
    let (ctx, mut rx) = make_ctx(executor);

    let result = SlashResult::Exec {
        command: "echo hello".to_string(),
    };

    rt.block_on(result.execute(&ctx));

    assert_eq!(call_counter.load(Ordering::SeqCst), 1);

    let action = rt.block_on(rx.recv()).expect("expected a reply action");
    match action {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert_eq!(
                blocks[0],
                ContentBlock::Text("output of: echo hello".to_string())
            );
        }
        other => panic!("expected Reply action, got {:?}", other),
    }
}

#[test]
fn test_exec_permission_reject() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (exec, call_counter) = MockExecutor::new(false);
    let executor: Arc<dyn SlashEffectExecutor> = Arc::new(exec);
    let (ctx, mut rx) = make_ctx(executor);

    let result = SlashResult::Exec {
        command: "rm -rf /".to_string(),
    };

    rt.block_on(result.execute(&ctx));

    assert_eq!(call_counter.load(Ordering::SeqCst), 1);

    let action = rt.block_on(rx.recv()).expect("expected a reply action");
    match action {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert_eq!(
                blocks[0],
                ContentBlock::Text("permission denied".to_string())
            );
        }
        other => panic!("expected Reply action, got {:?}", other),
    }
}

#[test]
fn test_reply_variant() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (exec, _) = MockExecutor::new(true);
    let executor: Arc<dyn SlashEffectExecutor> = Arc::new(exec);
    let (ctx, mut rx) = make_ctx(executor);

    let result = SlashResult::Reply("help text".to_string());
    rt.block_on(result.execute(&ctx));

    let action = rt.block_on(rx.recv()).expect("expected reply");
    match action {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks, vec![ContentBlock::Text("help text".to_string())]);
        }
        other => panic!("expected Reply, got {:?}", other),
    }
}

#[test]
fn test_set_mode_variant() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (exec, _) = MockExecutor::new(true);
    let executor: Arc<dyn SlashEffectExecutor> = Arc::new(exec);
    let (ctx, mut rx) = make_ctx(executor);

    let result = SlashResult::SetMode("plan".to_string());
    rt.block_on(result.execute(&ctx));

    let action = rt.block_on(rx.recv()).expect("expected reply");
    match action {
        ReplyAction::Reply(blocks) => {
            assert_eq!(
                blocks,
                vec![ContentBlock::Text("Mode set to: plan".to_string())]
            );
        }
        other => panic!("expected Reply, got {:?}", other),
    }
}

#[test]
fn test_unknown_variant() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (exec, _) = MockExecutor::new(true);
    let executor: Arc<dyn SlashEffectExecutor> = Arc::new(exec);
    let (ctx, mut rx) = make_ctx(executor);

    let result = SlashResult::Unknown("nope".to_string());
    rt.block_on(result.execute(&ctx));

    let action = rt.block_on(rx.recv()).expect("expected reply");
    match action {
        ReplyAction::Reply(blocks) => {
            assert_eq!(
                blocks,
                vec![ContentBlock::Text("Unknown command: /nope".to_string())]
            );
        }
        other => panic!("expected Reply, got {:?}", other),
    }
}

#[test]
fn test_exec_gets_agent_id_from_session_lookup() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    struct CapturingExecutor {
        captured_agent_id: Arc<std::sync::Mutex<Option<String>>>,
    }

    #[async_trait]
    impl SlashEffectExecutor for CapturingExecutor {
        async fn execute_stop(&self, _: &str) {}
        async fn execute_new_session(&self, _: &str, _: &str) {}
        async fn execute_compact(&self, _: &str, _: Option<String>) {}
        async fn execute_system_append(
            &self,
            _: &str,
            _: &crate::slash_router::SystemAppendAction,
        ) {
        }
        async fn execute_set_reasoning(&self, _: &str, _: crate::session_types::ReasoningLevel) {}
        async fn execute_set_verbosity(&self, _: &str, _: crate::verbosity::VerbosityLevel) {}

        async fn execute_exec(
            &self,
            _session_id: &str,
            agent_id: &str,
            _command: &str,
        ) -> Vec<ContentBlock> {
            *self.captured_agent_id.lock().unwrap() = Some(agent_id.to_string());
            vec![ContentBlock::Text("done".to_string())]
        }
    }

    let captured = Arc::new(std::sync::Mutex::new(None));
    let executor: Arc<dyn SlashEffectExecutor> = Arc::new(CapturingExecutor {
        captured_agent_id: captured.clone(),
    });
    let (ctx, mut rx) = make_ctx(executor);

    let result = SlashResult::Exec {
        command: "ls".to_string(),
    };
    rt.block_on(result.execute(&ctx));
    let _ = rt.block_on(rx.recv());

    let id = captured.lock().unwrap().clone();
    assert_eq!(id.as_deref(), Some("mock_chat_id"));
}
