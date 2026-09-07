//! Slash command executor types and traits.
//!
//! Re-exports executor types from `closeclaw-common`. The canonical definitions
//! live in `closeclaw-common::executor` because the `closeclaw-gateway` crate
//! cannot depend on `closeclaw-slash` (cycle: gateway → slash → tools → gateway).

pub use closeclaw_common::executor::{
    CompactionError, CompactionResult, ReplyAction, SideEffectContext, SlashEffectExecutor,
    SlashResultExecutor,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SlashResult;
    use closeclaw_common::processor::ContentBlock;
    use closeclaw_common::session_lookup::SessionLookup;

    struct MockLookup;
    #[async_trait::async_trait]
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

    struct MockExecutor;
    #[async_trait::async_trait]
    impl SlashEffectExecutor for MockExecutor {
        async fn execute_stop(&self, _: &str, _: bool, _: bool) {}
        async fn execute_new_session(&self, _: &str, _: &str) -> String {
            "new-session".into()
        }
        async fn execute_compact(
            &self,
            _: &str,
            _: Option<String>,
        ) -> Result<CompactionResult, CompactionError> {
            Ok(CompactionResult {
                performed: false,
                original_tokens: 0,
                compacted_tokens: 0,
                message: String::new(),
                before_char_count: 0,
                after_char_count: 0,
                before_token_count: 0,
                after_token_count: 0,
                boundary_message: String::new(),
                is_auto: false,
            })
        }
        async fn execute_system_append(
            &self,
            _: &str,
            _: &closeclaw_common::slash_router::SystemAppendAction,
        ) -> usize {
            0
        }
        async fn execute_set_reasoning(
            &self,
            _: &str,
            _: closeclaw_common::ReasoningLevel,
        ) -> Option<closeclaw_common::ReasoningLevel> {
            None
        }
        async fn execute_set_verbosity(&self, _: &str, _: closeclaw_common::VerbosityLevel) {}
        async fn execute_set_mode(&self, _: &str, _: &str) {}
        async fn execute_exec(&self, _: &str, _: &str, _: &str) -> Vec<ContentBlock> {
            vec![]
        }
    }

    /// Configurable mock for reasoning return values.
    struct ReasoningMockExecutor {
        effective: std::sync::Mutex<Option<closeclaw_common::ReasoningLevel>>,
    }

    impl ReasoningMockExecutor {
        fn new(effective: Option<closeclaw_common::ReasoningLevel>) -> Self {
            Self {
                effective: std::sync::Mutex::new(effective),
            }
        }
    }

    #[async_trait::async_trait]
    impl SlashEffectExecutor for ReasoningMockExecutor {
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
        async fn execute_system_append(
            &self,
            _: &str,
            _: &closeclaw_common::slash_router::SystemAppendAction,
        ) -> usize {
            0
        }
        async fn execute_set_reasoning(
            &self,
            _: &str,
            _: closeclaw_common::ReasoningLevel,
        ) -> Option<closeclaw_common::ReasoningLevel> {
            *self.effective.lock().unwrap()
        }
        async fn execute_set_verbosity(&self, _: &str, _: closeclaw_common::VerbosityLevel) {}
        async fn execute_set_mode(&self, _: &str, _: &str) {}
        async fn execute_exec(&self, _: &str, _: &str, _: &str) -> Vec<ContentBlock> {
            vec![]
        }
    }

    fn make_reasoning_ctx(
        exec: std::sync::Arc<ReasoningMockExecutor>,
    ) -> (SideEffectContext, tokio::sync::mpsc::Receiver<ReplyAction>) {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let ctx = SideEffectContext {
            session_id: "slash-reasoning-test".into(),
            channel: "test".into(),
            session_lookup: std::sync::Arc::new(MockLookup),
            reply_tx: tx,
            executor: exec,
        };
        (ctx, rx)
    }

    /// Smoke test: verify re-exported types are usable and that
    /// `SlashResult::Reply` can be dispatched through `SlashResultExecutor`.
    #[tokio::test]
    async fn smoke_slash_result_executor_reexport() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<ReplyAction>(4);
        let ctx = SideEffectContext {
            session_id: "smoke-test-session".into(),
            channel: "test".into(),
            session_lookup: std::sync::Arc::new(MockLookup),
            reply_tx: tx,
            executor: std::sync::Arc::new(MockExecutor),
        };
        let result = SlashResult::Reply("hello from smoke test".into());
        result.execute(&ctx).await;
        let action = rx.recv().await.expect("expected a ReplyAction");
        match action {
            ReplyAction::Reply(blocks) => {
                assert_eq!(blocks.len(), 1);
                match &blocks[0] {
                    ContentBlock::Text(t) => assert_eq!(t, "hello from smoke test"),
                    other => panic!("expected ContentBlock::Text, got {:?}", other),
                }
            }
            other => panic!("expected ReplyAction::Reply, got {:?}", other),
        }
    }

    /// No downgrade: requested == effective → "推理深度已设为 {effective}（含 provider 降级后的值）".
    #[tokio::test]
    async fn test_set_reasoning_no_downgrade_slash() {
        let exec = std::sync::Arc::new(ReasoningMockExecutor::new(Some(
            closeclaw_common::ReasoningLevel::High,
        )));
        let (ctx, mut rx) = make_reasoning_ctx(exec);
        SlashResult::SetReasoning {
            level: closeclaw_common::ReasoningLevel::High,
        }
        .execute(&ctx)
        .await;
        drop(ctx);

        let action = rx.recv().await.expect("expected a ReplyAction");
        match action {
            ReplyAction::Reply(blocks) => {
                assert!(
                    matches!(&blocks[0], ContentBlock::Text(t) if t == "推理深度已设为 High（含 provider 降级后的值）"),
                    "no-downgrade reply should contain parenthetical, got: {:?}",
                    &blocks[0],
                );
            }
            other => panic!("expected ReplyAction::Reply, got {:?}", other),
        }
    }

    /// Downgrade: requested=Max, effective=High → downgrade explanation.
    #[tokio::test]
    async fn test_set_reasoning_downgrade_slash() {
        let exec = std::sync::Arc::new(ReasoningMockExecutor::new(Some(
            closeclaw_common::ReasoningLevel::High,
        )));
        let (ctx, mut rx) = make_reasoning_ctx(exec);
        SlashResult::SetReasoning {
            level: closeclaw_common::ReasoningLevel::Max,
        }
        .execute(&ctx)
        .await;
        drop(ctx);

        let action = rx.recv().await.expect("expected a ReplyAction");
        match action {
            ReplyAction::Reply(blocks) => {
                assert!(
                    matches!(&blocks[0], ContentBlock::Text(t) if t == "推理深度已设为 High（原请求 Max 已按供应商能力降级）"),
                    "downgrade reply should contain effective level and explanation, got: {:?}",
                    &blocks[0],
                );
            }
            other => panic!("expected ReplyAction::Reply, got {:?}", other),
        }
    }

    /// Off + provider supports closing → "推理输出已关闭".
    #[tokio::test]
    async fn test_set_reasoning_off_provider_supports_closing_slash() {
        let exec = std::sync::Arc::new(ReasoningMockExecutor::new(Some(
            closeclaw_common::ReasoningLevel::Off,
        )));
        let (ctx, mut rx) = make_reasoning_ctx(exec);
        SlashResult::SetReasoning {
            level: closeclaw_common::ReasoningLevel::Off,
        }
        .execute(&ctx)
        .await;
        drop(ctx);

        let action = rx.recv().await.expect("expected a ReplyAction");
        match action {
            ReplyAction::Reply(blocks) => {
                assert!(
                    matches!(&blocks[0], ContentBlock::Text(t) if t == "推理输出已关闭"),
                    "Off+supports-closing should reply '推理输出已关闭', got: {:?}",
                    &blocks[0],
                );
            }
            other => panic!("expected ReplyAction::Reply, got {:?}", other),
        }
    }

    /// None (session not found) → legacy fallback.
    #[tokio::test]
    async fn test_set_reasoning_none_fallback_slash() {
        let exec = std::sync::Arc::new(ReasoningMockExecutor::new(None));
        let (ctx, mut rx) = make_reasoning_ctx(exec);
        SlashResult::SetReasoning {
            level: closeclaw_common::ReasoningLevel::Max,
        }
        .execute(&ctx)
        .await;
        drop(ctx);

        let action = rx.recv().await.expect("expected a ReplyAction");
        match action {
            ReplyAction::Reply(blocks) => {
                assert!(
                    matches!(&blocks[0], ContentBlock::Text(t) if t == "推理深度已设为 Max（含 provider 降级后的值）"),
                    "None fallback should use legacy reply, got: {:?}",
                    &blocks[0],
                );
            }
            other => panic!("expected ReplyAction::Reply, got {:?}", other),
        }
    }
}
