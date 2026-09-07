//! Shared test utilities for slash executor tests.
//!
//! Contains `ReasoningConfigMockExecutor` and `make_reasoning_ctx`
//! used by `slash_executor_tests.rs` and `slash_permission_tests.rs`.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use closeclaw_common::executor::{SideEffectContext, SlashEffectExecutor};
use closeclaw_common::processor::ContentBlock;
use closeclaw_common::session_lookup::SessionLookup;
use closeclaw_common::slash_router::SystemAppendAction;
use closeclaw_common::verbosity::VerbosityLevel;
use closeclaw_session::compaction::{CompactionError, CompactionResult};
use closeclaw_session::persistence::ReasoningLevel;

// ---------------------------------------------------------------------------
// Mock SessionLookup (minimal, for test context construction)
// ---------------------------------------------------------------------------

struct MockLookup;

#[async_trait]
impl SessionLookup for MockLookup {
    async fn get_parent_of(&self, _: &str) -> Option<String> {
        None
    }
    async fn get_chat_id(&self, _: &str) -> Option<String> {
        None
    }
    async fn push_pending_message(
        &self,
        _: &str,
        _: closeclaw_common::session_lookup::PendingMessage,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn get_plan_state(&self, _: &str) -> Option<closeclaw_common::PlanState> {
        None
    }
    async fn set_plan_state(&self, _: &str, _: closeclaw_common::PlanState) {}
    async fn set_session_mode(&self, _: &str, _: closeclaw_common::SessionMode) {}
}

// ---------------------------------------------------------------------------
// ReasoningConfigMockExecutor
// ---------------------------------------------------------------------------

/// Configurable mock for `execute_set_reasoning` that returns a preset
/// effective level.
pub(crate) struct ReasoningConfigMockExecutor {
    set_reasoning_called: std::sync::Mutex<bool>,
    reasoning_effective: std::sync::Mutex<Option<ReasoningLevel>>,
}

impl ReasoningConfigMockExecutor {
    pub(crate) fn new(effective: Option<ReasoningLevel>) -> Self {
        Self {
            set_reasoning_called: std::sync::Mutex::new(false),
            reasoning_effective: std::sync::Mutex::new(effective),
        }
    }

    pub(crate) fn was_called(&self) -> bool {
        *self.set_reasoning_called.lock().unwrap()
    }
}

#[async_trait]
impl SlashEffectExecutor for ReasoningConfigMockExecutor {
    async fn execute_stop(&self, _: &str, _: bool, _: bool) {}
    async fn execute_new_session(&self, _: &str, _: &str) -> String {
        "mock-id".into()
    }
    async fn execute_compact(
        &self,
        _: &str,
        _: Option<String>,
    ) -> Result<CompactionResult, CompactionError> {
        unimplemented!()
    }
    async fn execute_system_append(&self, _: &str, _: &SystemAppendAction) -> usize {
        0
    }
    async fn execute_set_reasoning(
        &self,
        _: &str,
        _level: ReasoningLevel,
    ) -> Option<ReasoningLevel> {
        *self.set_reasoning_called.lock().unwrap() = true;
        *self.reasoning_effective.lock().unwrap()
    }
    async fn execute_set_verbosity(&self, _: &str, _: VerbosityLevel) {}
    async fn execute_set_mode(&self, _: &str, _: &str) {}
    async fn execute_exec(&self, _: &str, _: &str, _: &str) -> Vec<ContentBlock> {
        vec![]
    }
}

// ---------------------------------------------------------------------------
// make_reasoning_ctx
// ---------------------------------------------------------------------------

/// Build a [`SideEffectContext`] with the given reasoning mock executor.
pub(crate) fn make_reasoning_ctx(
    exec: Arc<ReasoningConfigMockExecutor>,
) -> (
    SideEffectContext,
    mpsc::Receiver<closeclaw_common::executor::ReplyAction>,
) {
    let (tx, rx) = mpsc::channel(16);
    let ctx = SideEffectContext {
        session_id: "sess-reasoning".into(),
        channel: "feishu".into(),
        session_lookup: Arc::new(MockLookup),
        reply_tx: tx,
        executor: exec,
    };
    (ctx, rx)
}
