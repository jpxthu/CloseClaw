//! Unit tests for the failed-turn completion signal (Step 1.15).
//!
//! `finish_llm` → `clear_busy_and_send` error arm
//! (`session_handler_announce.rs`) must emit an EMPTY `("", [])` payload on
//! the handler output channel when the LLM caller returns `Err`, so waiting
//! chat callers are finalized instead of blocking until the turn-completion
//! timeout (120s hang otherwise). The consumer half of the contract —
//! turning that empty payload into `finish_turns()` — is locked by
//! `daemon::chat_rpc_tests::test_turn_completion_consumer_finalizes_failed_empty_payload_turn`.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::processor::{StreamEvent, UnifiedResponse};
use closeclaw_common::{LLMError, LlmCaller};
use tokio::time::Instant;

use crate::outbound::StreamResult;
use crate::session_handler_recovery_tests::{make_output_tx, make_sm};
use crate::session_manager::test_helpers::make_msg;
use crate::SessionMessageHandler;

/// Deterministic `LlmCaller` that always fails — stands in for an
/// unreachable/broken provider so the turn ends in the error arm.
#[derive(Default)]
struct FailingCaller {
    calls: AtomicU32,
}

#[async_trait]
impl LlmCaller for FailingCaller {
    async fn call(&self, _request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(LLMError::ApiError("injected llm failure".to_string()))
    }

    async fn call_streaming(
        &self,
        _request: InternalRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamEvent, LLMError>> + Send>>,
        LLMError,
    > {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(LLMError::ApiError("injected llm failure".to_string()))
    }
}

/// Drive a turn whose LLM caller returns `Err` and assert the failure arm
/// emits the empty turn-completion payload.
#[tokio::test]
async fn test_failed_llm_turn_emits_empty_completion_payload() {
    let sm = make_sm();
    let caller = Arc::new(FailingCaller::default());
    sm.set_llm_caller(caller.clone()).await;
    let sid = sm
        .find_or_create("ch", &make_msg(), None)
        .await
        .expect("session creation must succeed");

    // Drive the injected caller through the session's real LLM entry point.
    let cs = sm
        .get_conversation_session(&sid)
        .await
        .expect("conversation session");
    let raw = cs.write().await.invoke_llm("hello").await;
    let result: Result<StreamResult, LLMError> = raw.map(Into::into);
    assert!(result.is_err(), "injected LLM caller must fail this turn");
    assert_eq!(
        caller.calls.load(Ordering::SeqCst),
        1,
        "the injected failing caller must be the one invoked"
    );

    // Reuse the OutputTx harness from the recovery tests.
    let (output_tx, mut rx) = make_output_tx(true);
    SessionMessageHandler::finish_llm(&sm, &sid, result, Instant::now(), &output_tx, &None, None)
        .await;

    // The error arm awaits the send inline — no sleep/polling needed.
    let (text, blocks) = rx
        .try_recv()
        .expect("failed turn must emit the completion signal");
    assert_eq!(text, "", "failure payload must carry empty text");
    assert!(
        blocks.is_empty(),
        "failure payload must carry no content blocks"
    );
}
