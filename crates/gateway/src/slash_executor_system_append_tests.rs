//! Unit tests for system append reply format (Step 1.3).
//!
//! Covers incrementing `#N` on Add, zero/non-zero counts on Clear,
//! and exact reply format verification.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use closeclaw_common::executor::{ReplyAction, SideEffectContext, SlashEffectExecutor};
use closeclaw_common::processor::ContentBlock;
use closeclaw_common::session_lookup::PendingMessage;
use closeclaw_common::slash_router::{SlashResult, SystemAppendAction};
use closeclaw_session::compaction::{CompactionError, CompactionResult};

use closeclaw_common::executor::SlashResultExecutor;

// ── Minimal mock (SessionLookup) ──────────────────────────────────────

struct MockSessionLookup;

#[async_trait]
impl closeclaw_common::session_lookup::SessionLookup for MockSessionLookup {
    async fn get_parent_of(&self, _: &str) -> Option<String> {
        None
    }
    async fn get_chat_id(&self, _: &str) -> Option<String> {
        Some("agent-007".into())
    }
    async fn push_pending_message(&self, _: &str, _: PendingMessage) -> Result<(), String> {
        Ok(())
    }
    async fn get_plan_state(&self, _: &str) -> Option<closeclaw_common::PlanState> {
        None
    }
    async fn set_plan_state(&self, _: &str, _: closeclaw_common::PlanState) {}
    async fn set_session_mode(&self, _: &str, _: closeclaw_common::SessionMode) {}
}

// ── Counting mock ─────────────────────────────────────────────────────

/// Mock executor that tracks `execute_system_append` call count and returns
/// the 1-based index (for Add) or cumulative count (for Clear).
struct CountingMockExecutor {
    system_append_call_count: std::sync::atomic::AtomicUsize,
}

impl CountingMockExecutor {
    fn new() -> Self {
        Self {
            system_append_call_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl SlashEffectExecutor for CountingMockExecutor {
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
    async fn execute_system_append(&self, _: &str, action: &SystemAppendAction) -> usize {
        let n = self
            .system_append_call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        match action {
            SystemAppendAction::Add(_) => n,    // 1-based index
            SystemAppendAction::Clear => n * 2, // simulate N items cleared
        }
    }
    async fn execute_set_reasoning(&self, _: &str, _: closeclaw_common::ReasoningLevel) {}
    async fn execute_set_verbosity(&self, _: &str, _: closeclaw_common::VerbosityLevel) {}
    async fn execute_set_mode(&self, _: &str, _: &str) {}
    async fn execute_exec(&self, _: &str, _: &str, _: &str) -> Vec<ContentBlock> {
        vec![]
    }
}

// ── Helpers ───────────────────────────────────────────────────────────

async fn drain_actions(rx: &mut mpsc::Receiver<ReplyAction>) -> Vec<ReplyAction> {
    let mut actions = Vec::new();
    while let Some(a) = rx.recv().await {
        actions.push(a);
    }
    actions
}

fn make_counting_ctx(
    exec: Arc<CountingMockExecutor>,
) -> (SideEffectContext, mpsc::Receiver<ReplyAction>) {
    let (tx, rx) = mpsc::channel(16);
    let ctx = SideEffectContext {
        session_id: "sess-counting".into(),
        channel: "feishu".into(),
        session_lookup: Arc::new(MockSessionLookup),
        reply_tx: tx,
        executor: exec,
    };
    (ctx, rx)
}

// ── Tests ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_system_append_add_increments_number() {
    // Three sequential Add actions should produce #1, #2, #3.
    let executor: Arc<CountingMockExecutor> = Arc::new(CountingMockExecutor::new());
    let make_ctx = || make_counting_ctx(executor.clone());

    // First add → #1
    let (ctx1, mut rx1) = make_ctx();
    SlashResult::SystemAppend {
        action: SystemAppendAction::Add("first".into()),
    }
    .execute(&ctx1)
    .await;
    drop(ctx1);

    let actions = drain_actions(&mut rx1).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert!(
                matches!(&blocks[0], ContentBlock::Text(t) if t == "已追加指令 #1"),
                "first add should be #1, got: {:?}",
                &blocks[0],
            );
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }

    // Second add → #2
    let (ctx2, mut rx2) = make_ctx();
    SlashResult::SystemAppend {
        action: SystemAppendAction::Add("second".into()),
    }
    .execute(&ctx2)
    .await;
    drop(ctx2);

    let actions = drain_actions(&mut rx2).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert!(
                matches!(&blocks[0], ContentBlock::Text(t) if t == "已追加指令 #2"),
                "second add should be #2, got: {:?}",
                &blocks[0],
            );
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }

    // Third add → #3
    let (ctx3, mut rx3) = make_ctx();
    SlashResult::SystemAppend {
        action: SystemAppendAction::Add("third".into()),
    }
    .execute(&ctx3)
    .await;
    drop(ctx3);

    let actions = drain_actions(&mut rx3).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert!(
                matches!(&blocks[0], ContentBlock::Text(t) if t == "已追加指令 #3"),
                "third add should be #3, got: {:?}",
                &blocks[0],
            );
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_system_append_clear_zero_items() {
    // Clear on empty list → executor returns 0 → "已清除 0 条追加指令".
    struct ZeroClearMockExecutor;

    #[async_trait]
    impl SlashEffectExecutor for ZeroClearMockExecutor {
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
        async fn execute_set_reasoning(&self, _: &str, _: closeclaw_common::ReasoningLevel) {}
        async fn execute_set_verbosity(&self, _: &str, _: closeclaw_common::VerbosityLevel) {}
        async fn execute_set_mode(&self, _: &str, _: &str) {}
        async fn execute_exec(&self, _: &str, _: &str, _: &str) -> Vec<ContentBlock> {
            vec![]
        }
    }

    let (tx, mut rx) = mpsc::channel(16);
    let ctx = SideEffectContext {
        session_id: "sess-zero".into(),
        channel: "feishu".into(),
        session_lookup: Arc::new(MockSessionLookup),
        reply_tx: tx,
        executor: Arc::new(ZeroClearMockExecutor),
    };
    SlashResult::SystemAppend {
        action: SystemAppendAction::Clear,
    }
    .execute(&ctx)
    .await;
    drop(ctx);

    let actions = drain_actions(&mut rx).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert!(
                matches!(&blocks[0], ContentBlock::Text(t) if t == "已清除 0 条追加指令"),
                "clear on empty list should show 0, got: {:?}",
                &blocks[0],
            );
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_system_append_clear_count_reflected_in_reply() {
    // Second Clear call → executor returns 4 (2*2), verifying the
    // count from executor is embedded in the reply string.
    let exec_arc: Arc<CountingMockExecutor> = Arc::new(CountingMockExecutor::new());
    let make_ctx = || make_counting_ctx(exec_arc.clone());

    // First call to increment counter
    let (ctx1, _) = make_ctx();
    SlashResult::SystemAppend {
        action: SystemAppendAction::Clear,
    }
    .execute(&ctx1)
    .await;
    drop(ctx1);

    // Second call → returns 4
    let (ctx2, mut rx2) = make_ctx();
    SlashResult::SystemAppend {
        action: SystemAppendAction::Clear,
    }
    .execute(&ctx2)
    .await;
    drop(ctx2);

    let actions = drain_actions(&mut rx2).await;
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            assert_eq!(blocks.len(), 1);
            assert!(
                matches!(&blocks[0], ContentBlock::Text(t) if t == "已清除 4 条追加指令"),
                "second clear should reflect count 4, got: {:?}",
                &blocks[0],
            );
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}

#[tokio::test]
async fn test_system_append_add_reply_format_uses_count() {
    // Verify the reply format exactly matches "已追加指令 #{N}" where N
    // is the value returned by execute_system_append.
    let executor: Arc<CountingMockExecutor> = Arc::new(CountingMockExecutor::new());
    let (ctx, mut rx) = make_counting_ctx(executor);
    SlashResult::SystemAppend {
        action: SystemAppendAction::Add("test content".into()),
    }
    .execute(&ctx)
    .await;
    drop(ctx);

    let actions = drain_actions(&mut rx).await;
    match &actions[0] {
        ReplyAction::Reply(blocks) => {
            let text = match &blocks[0] {
                ContentBlock::Text(t) => t.as_str(),
                other => panic!("expected Text block, got {other:?}"),
            };
            assert_eq!(text, "已追加指令 #1");
            assert!(
                text.starts_with("已追加指令 #"),
                "reply should start with '已追加指令 #', got: {text}"
            );
        }
        other => panic!("expected ReplyAction::Reply, got {other:?}"),
    }
}
