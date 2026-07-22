//! Tests for session-level memory injection deduplication.
//!
//! Verifies that `ConversationSession::set_memory_injection` correctly
//! deduplicates by `task_id` while allowing injections without a
//! `task_id` to pass through.

use std::sync::Arc;

use crate::llm_session::{ConversationSession, InjectionPosition, MemoryInjection};
use async_trait::async_trait;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::processor::UnifiedResponse;
use closeclaw_common::{LLMError, LlmCaller};

use super::tmp_path;

fn canned_response(text: &str) -> UnifiedResponse {
    UnifiedResponse {
        content_blocks: vec![closeclaw_common::processor::ContentBlock::Text(text.into())],
        usage: closeclaw_common::processor::UnifiedUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: Some(2),
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
    }
}

struct FakeLlmCaller {
    response: UnifiedResponse,
    last_request: std::sync::Mutex<Option<InternalRequest>>,
}

impl FakeLlmCaller {
    fn new(text: &str) -> Self {
        Self {
            response: canned_response(text),
            last_request: std::sync::Mutex::new(None),
        }
    }
}

#[async_trait]
impl LlmCaller for FakeLlmCaller {
    async fn call(&self, request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        *self.last_request.lock().unwrap() = Some(request);
        Ok(self.response.clone())
    }

    async fn call_streaming(
        &self,
        _request: InternalRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures::Stream<
                        Item = Result<closeclaw_common::processor::StreamEvent, LLMError>,
                    > + Send,
            >,
        >,
        LLMError,
    > {
        Err(LLMError::ApiError("not implemented".into()))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_new_session_activation_set_empty() {
    let session = ConversationSession::new("s_activation_empty".into(), "m".into(), tmp_path());
    assert!(
        session.activated_conditional_skills().is_empty(),
        "new session should have an empty activation set"
    );
}

#[tokio::test]
async fn test_different_task_ids_both_inject() {
    let mut session = ConversationSession::new("s_diff_task".into(), "m".into(), tmp_path());
    let caller: Arc<dyn LlmCaller> = Arc::new(FakeLlmCaller::new("ok"));
    session.set_llm_caller(caller);

    // First injection with task_id "alpha" — accepted
    let mut inj1 =
        MemoryInjection::new("context from alpha".into(), InjectionPosition::AfterCurrent);
    inj1.task_id = Some("alpha".into());
    assert!(session.set_memory_injection(inj1));

    // Second injection with task_id "beta" — also accepted (different id)
    let mut inj2 =
        MemoryInjection::new("context from beta".into(), InjectionPosition::AfterCurrent);
    inj2.task_id = Some("beta".into());
    assert!(session.set_memory_injection(inj2));

    // Consume — gets "beta" (latest write wins)
    let taken = session.take_memory_injection();
    assert!(taken.is_some());
    assert_eq!(taken.unwrap().content, "context from beta");

    // Third injection with task_id "alpha" — rejected because
    // "alpha" was already accepted (recorded in injected_task_ids)
    let mut inj3 = MemoryInjection::new("alpha again".into(), InjectionPosition::AfterCurrent);
    inj3.task_id = Some("alpha".into());
    assert!(
        !session.set_memory_injection(inj3),
        "already-recorded task_id should be deduplicated"
    );
}

#[tokio::test]
async fn test_same_task_id_dedup() {
    let mut session = ConversationSession::new("s_same_task".into(), "m".into(), tmp_path());
    let caller: Arc<dyn LlmCaller> = Arc::new(FakeLlmCaller::new("ok"));
    session.set_llm_caller(caller);

    // First injection with task_id "gamma" — accepted
    let mut inj1 = MemoryInjection::new("first gamma".into(), InjectionPosition::AfterCurrent);
    inj1.task_id = Some("gamma".into());
    assert!(session.set_memory_injection(inj1));

    // Second injection with same task_id "gamma" — rejected
    let mut inj2 = MemoryInjection::new("second gamma".into(), InjectionPosition::AfterCurrent);
    inj2.task_id = Some("gamma".into());
    assert!(
        !session.set_memory_injection(inj2),
        "same task_id should be deduplicated"
    );

    // Consume — first injection still in slot
    let taken = session.take_memory_injection();
    assert!(taken.is_some());
    assert_eq!(taken.unwrap().content, "first gamma");
}

#[tokio::test]
async fn test_none_task_id_no_dedup() {
    let mut session = ConversationSession::new("s_none_task".into(), "m".into(), tmp_path());
    let caller: Arc<dyn LlmCaller> = Arc::new(FakeLlmCaller::new("ok"));
    session.set_llm_caller(caller);

    // First injection without task_id — accepted
    let inj1 = MemoryInjection::new("first none".into(), InjectionPosition::AfterCurrent);
    assert!(session.set_memory_injection(inj1));

    // Consume
    let _ = session.take_memory_injection();

    // Second injection without task_id — also accepted (no dedup)
    let inj2 = MemoryInjection::new("second none".into(), InjectionPosition::AfterCurrent);
    assert!(
        session.set_memory_injection(inj2),
        "None task_id should not trigger dedup"
    );

    let taken = session.take_memory_injection();
    assert!(taken.is_some());
    assert_eq!(taken.unwrap().content, "second none");
}
