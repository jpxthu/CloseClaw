//! Shared test utilities for slash executor tests.
//!
//! Contains `ReasoningConfigMockExecutor`, `make_reasoning_ctx` and the
//! assertion helpers `reply_blocks` / `text_block_at` / `first_text_block`
//! used by `slash_executor_tests.rs`,
//! `slash_executor_system_append_tests.rs` and
//! `outbound_streaming_checkpoint_tests.rs`, plus the reply-channel helper
//! `drain_actions` used by the first two.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use closeclaw_common::executor::{ReplyAction, SideEffectContext, SlashEffectExecutor};
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
    async fn execute_stop(&self, _: &str, _: Option<&str>) {}
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
) -> (SideEffectContext, mpsc::Receiver<ReplyAction>) {
    let (tx, rx) = mpsc::channel(16);
    let ctx = SideEffectContext {
        session_id: "sess-reasoning".into(),
        channel: "feishu".into(),
        sender_id: None,
        session_lookup: Arc::new(MockLookup),
        reply_tx: tx,
        executor: exec,
    };
    (ctx, rx)
}

// ---------------------------------------------------------------------------
// Reply channel helper
// ---------------------------------------------------------------------------

/// Drain all remaining [`ReplyAction`]s from the reply channel.
///
/// Reads the receiver until the channel is closed (all senders dropped) and
/// returns the collected actions in send order.
pub(crate) async fn drain_actions(rx: &mut mpsc::Receiver<ReplyAction>) -> Vec<ReplyAction> {
    let mut actions = Vec::new();
    while let Some(a) = rx.recv().await {
        actions.push(a);
    }
    actions
}

// ---------------------------------------------------------------------------
// Assertion helpers
// ---------------------------------------------------------------------------

/// Return the text of the [`ContentBlock`] at `index`.
///
/// Panics with an expectation/actual message when `index` is out of bounds
/// or the block at `index` is not [`ContentBlock::Text`].
pub(crate) fn text_block_at(blocks: &[ContentBlock], index: usize) -> &str {
    assert!(
        index < blocks.len(),
        "expected ContentBlock at index {index}, got: slice of length {}",
        blocks.len()
    );
    match &blocks[index] {
        ContentBlock::Text(t) => t.as_str(),
        other => panic!("expected ContentBlock::Text, got: {other:?}"),
    }
}

/// Return the text of the first [`ContentBlock`].
///
/// Panics with an expectation/actual message when `blocks` is empty or the
/// first block is not [`ContentBlock::Text`].
pub(crate) fn first_text_block(blocks: &[ContentBlock]) -> &str {
    assert!(
        !blocks.is_empty(),
        "expected at least one ContentBlock, got: empty slice"
    );
    text_block_at(blocks, 0)
}

/// Return the content blocks of the first [`ReplyAction`].
///
/// Panics with an expectation/actual message when `actions` is empty or the
/// first action is not [`ReplyAction::Reply`].
pub(crate) fn reply_blocks(actions: &[ReplyAction]) -> &[ContentBlock] {
    assert!(
        !actions.is_empty(),
        "expected at least one ReplyAction, got: empty slice"
    );
    match &actions[0] {
        ReplyAction::Reply(blocks) => blocks.as_slice(),
        other => panic!("expected ReplyAction::Reply, got: {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_text_block_returns_text_of_single_block() {
        let blocks = vec![ContentBlock::Text("hello world".into())];
        assert_eq!(first_text_block(&blocks), "hello world");
    }

    #[test]
    fn test_first_text_block_returns_first_block_when_multiple() {
        let blocks = vec![
            ContentBlock::Text("first".into()),
            ContentBlock::Text("second".into()),
        ];
        assert_eq!(first_text_block(&blocks), "first");
    }

    #[test]
    #[should_panic(expected = "expected ContentBlock::Text, got: Thinking")]
    fn test_first_text_block_panics_on_non_text_variant() {
        let blocks = vec![ContentBlock::Thinking {
            thinking: "reasoning trace".into(),
            signature: None,
        }];
        let _ = first_text_block(&blocks);
    }

    #[test]
    #[should_panic(expected = "expected at least one ContentBlock, got: empty slice")]
    fn test_first_text_block_panics_on_empty_slice() {
        let blocks: Vec<ContentBlock> = vec![];
        let _ = first_text_block(&blocks);
    }

    #[test]
    fn test_text_block_at_returns_block_at_index() {
        let blocks = vec![
            ContentBlock::Text("first".into()),
            ContentBlock::Text("second".into()),
        ];
        assert_eq!(text_block_at(&blocks, 0), "first");
        assert_eq!(text_block_at(&blocks, 1), "second");
    }

    #[test]
    #[should_panic(expected = "expected ContentBlock at index 1, got: slice of length 1")]
    fn test_text_block_at_panics_on_out_of_bounds() {
        let blocks = vec![ContentBlock::Text("only".into())];
        let _ = text_block_at(&blocks, 1);
    }

    #[test]
    #[should_panic(expected = "expected ContentBlock::Text, got: Thinking")]
    fn test_text_block_at_panics_on_non_text_variant() {
        let blocks = vec![ContentBlock::Thinking {
            thinking: "reasoning trace".into(),
            signature: None,
        }];
        let _ = text_block_at(&blocks, 0);
    }

    #[test]
    fn test_reply_blocks_returns_blocks_of_reply_action() {
        let actions = vec![ReplyAction::Reply(vec![
            ContentBlock::Text("first".into()),
            ContentBlock::Text("second".into()),
        ])];
        assert_eq!(
            reply_blocks(&actions),
            [
                ContentBlock::Text("first".into()),
                ContentBlock::Text("second".into())
            ]
        );
    }

    #[test]
    fn test_reply_blocks_returns_first_action_when_multiple() {
        let actions = vec![
            ReplyAction::Reply(vec![ContentBlock::Text("first".into())]),
            ReplyAction::Nothing,
        ];
        assert_eq!(reply_blocks(&actions), [ContentBlock::Text("first".into())]);
    }

    #[test]
    #[should_panic(expected = "expected ReplyAction::Reply, got: TriggerCompact")]
    fn test_reply_blocks_panics_on_trigger_compact() {
        let actions = vec![ReplyAction::TriggerCompact { instruction: None }];
        let _ = reply_blocks(&actions);
    }

    #[test]
    #[should_panic(expected = "expected ReplyAction::Reply, got: Nothing")]
    fn test_reply_blocks_panics_on_nothing() {
        let actions = vec![ReplyAction::Nothing];
        let _ = reply_blocks(&actions);
    }

    #[test]
    #[should_panic(expected = "expected at least one ReplyAction, got: empty slice")]
    fn test_reply_blocks_panics_on_empty_actions() {
        let actions: Vec<ReplyAction> = vec![];
        let _ = reply_blocks(&actions);
    }

    #[tokio::test]
    async fn test_drain_actions_collects_multiple_actions_in_send_order() {
        let (tx, mut rx) = mpsc::channel(16);
        tx.send(ReplyAction::Reply(vec![ContentBlock::Text("first".into())]))
            .await
            .unwrap();
        tx.send(ReplyAction::TriggerCompact { instruction: None })
            .await
            .unwrap();
        tx.send(ReplyAction::Nothing).await.unwrap();
        drop(tx);

        let actions = drain_actions(&mut rx).await;

        assert_eq!(
            actions,
            vec![
                ReplyAction::Reply(vec![ContentBlock::Text("first".into())]),
                ReplyAction::TriggerCompact { instruction: None },
                ReplyAction::Nothing,
            ]
        );
    }

    #[tokio::test]
    async fn test_drain_actions_returns_empty_vec_when_channel_closed_empty() {
        let (tx, mut rx) = mpsc::channel::<ReplyAction>(16);
        drop(tx);

        let actions = drain_actions(&mut rx).await;

        assert!(actions.is_empty());
    }

    #[tokio::test]
    async fn test_drain_actions_collects_buffered_actions_after_sender_drop() {
        let (tx, mut rx) = mpsc::channel(2);
        tx.send(ReplyAction::Nothing).await.unwrap();
        tx.send(ReplyAction::TriggerCompact { instruction: None })
            .await
            .unwrap();
        drop(tx);

        let actions = drain_actions(&mut rx).await;

        assert_eq!(
            actions,
            vec![
                ReplyAction::Nothing,
                ReplyAction::TriggerCompact { instruction: None },
            ]
        );
    }
}
