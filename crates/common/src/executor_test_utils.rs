// Shared test utilities for executor tests.
//
// Contains mock implementations used by `executor_tests.rs`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::executor::{
    CompactionError, CompactionResult, ReplyAction, SideEffectContext, SlashEffectExecutor,
};
use crate::processor::ContentBlock;
use crate::session_lookup::{PendingMessage, SessionLookup};
use crate::slash_router::SystemAppendAction;
use crate::{ReasoningLevel, VerbosityLevel};

// ── Mock SlashEffectExecutor ──────────────────────────────────────────

/// Call recorded by mock executor for assertion.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ExecutorCall {
    Stop(String, bool, bool),
    NewSession(String, String),
    Compact(String, Option<String>),
    SystemAppend(String, SystemAppendAction),
    SetReasoning(String, ReasoningLevel),
    SetVerbosity(String, VerbosityLevel),
    SetMode(String, String),
    Exec(String, String, String),
}

/// Configurable mock for `execute_set_reasoning` return values.
pub(crate) struct MockSlashEffectExecutor {
    pub(crate) calls: Arc<Mutex<Vec<ExecutorCall>>>,
    pub(crate) reply_rx: Mutex<mpsc::Receiver<ReplyAction>>,
    pub(crate) reply_tx: mpsc::Sender<ReplyAction>,
    reasoning_effective: Option<Arc<Mutex<Option<ReasoningLevel>>>>,
}

impl MockSlashEffectExecutor {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::channel(32);
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            reply_rx: Mutex::new(rx),
            reply_tx: tx,
            reasoning_effective: None,
        }
    }

    pub(crate) fn with_reasoning(effective: Option<ReasoningLevel>) -> Self {
        let (tx, rx) = mpsc::channel(32);
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            reply_rx: Mutex::new(rx),
            reply_tx: tx,
            reasoning_effective: Some(Arc::new(Mutex::new(effective))),
        }
    }

    /// Drain all pending ReplyActions from the receiver.
    pub(crate) fn drain_replies(&self) -> Vec<ReplyAction> {
        let mut out = Vec::new();
        while let Ok(action) = self.reply_rx.lock().unwrap().try_recv() {
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

    async fn execute_set_reasoning(
        &self,
        session_id: &str,
        level: ReasoningLevel,
    ) -> Option<ReasoningLevel> {
        self.calls
            .lock()
            .unwrap()
            .push(ExecutorCall::SetReasoning(session_id.to_string(), level));
        self.reasoning_effective
            .as_ref()
            .map(|c| *c.lock().unwrap())
            .unwrap_or(Some(level))
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

pub(crate) struct MockSessionLookup {
    pending_messages: Arc<Mutex<Vec<PendingMessage>>>,
    chat_id: Option<String>,
    plan_state: Arc<Mutex<Option<crate::PlanState>>>,
    /// Tracks whether `clear_plan_state` was called.
    clear_called: Arc<Mutex<bool>>,
    /// Tracks `set_plan_state` call count.
    set_plan_state_calls: Arc<Mutex<u32>>,
}

impl MockSessionLookup {
    pub(crate) fn new(chat_id: Option<String>) -> Self {
        Self {
            pending_messages: Arc::new(Mutex::new(Vec::new())),
            chat_id,
            plan_state: Arc::new(Mutex::new(None)),
            clear_called: Arc::new(Mutex::new(false)),
            set_plan_state_calls: Arc::new(Mutex::new(0)),
        }
    }

    pub(crate) fn with_pending(pending: Arc<Mutex<Vec<PendingMessage>>>) -> Self {
        Self {
            pending_messages: pending,
            chat_id: None,
            plan_state: Arc::new(Mutex::new(None)),
            clear_called: Arc::new(Mutex::new(false)),
            set_plan_state_calls: Arc::new(Mutex::new(0)),
        }
    }

    /// Create with an existing plan_state and return a shared handle for assertions.
    pub(crate) fn with_plan_state(
        state: crate::PlanState,
    ) -> (Self, Arc<Mutex<Option<crate::PlanState>>>) {
        let plan = Arc::new(Mutex::new(Some(state)));
        let mock = Self {
            pending_messages: Arc::new(Mutex::new(Vec::new())),
            chat_id: None,
            plan_state: plan.clone(),
            clear_called: Arc::new(Mutex::new(false)),
            set_plan_state_calls: Arc::new(Mutex::new(0)),
        };
        (mock, plan)
    }

    /// Return a handle to the clear_called flag for test assertions.
    pub(crate) fn clear_called_handle(&self) -> Arc<Mutex<bool>> {
        self.clear_called.clone()
    }

    /// Return a handle to the set_plan_state call count.
    pub(crate) fn set_plan_state_calls_handle(&self) -> Arc<Mutex<u32>> {
        self.set_plan_state_calls.clone()
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
        self.plan_state.lock().unwrap().clone()
    }

    async fn set_plan_state(&self, _session_id: &str, plan_state: crate::PlanState) {
        *self.plan_state.lock().unwrap() = Some(plan_state);
        *self.set_plan_state_calls.lock().unwrap() += 1;
    }

    async fn clear_plan_state(&self, _session_id: &str) {
        *self.plan_state.lock().unwrap() = None;
        *self.clear_called.lock().unwrap() = true;
    }

    async fn set_session_mode(&self, _session_id: &str, _mode: crate::SessionMode) {}
}

// ── Mock that always returns a compact error ──────────────────────────

pub(crate) struct MockSlashEffectExecutorError;

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
    async fn execute_set_reasoning(&self, _: &str, _: ReasoningLevel) -> Option<ReasoningLevel> {
        None
    }
    async fn execute_set_verbosity(&self, _: &str, _: VerbosityLevel) {}
    async fn execute_set_mode(&self, _: &str, _: &str) {}
    async fn execute_exec(&self, _: &str, _: &str, _: &str) -> Vec<ContentBlock> {
        Vec::new()
    }
}

// ── Helper to build SideEffectContext ─────────────────────────────────

pub(crate) fn make_ctx(
    executor: Arc<MockSlashEffectExecutor>,
    session_id: &str,
    channel: &str,
    session_lookup: Arc<dyn SessionLookup>,
) -> SideEffectContext {
    SideEffectContext {
        session_id: session_id.to_string(),
        channel: channel.to_string(),
        session_lookup,
        reply_tx: executor.reply_tx.clone(),
        executor,
    }
}
