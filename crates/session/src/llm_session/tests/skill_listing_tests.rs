//! Tests for per-turn skill listing injection in `ConversationSession`.
//!
//! Verifies that `build_llm_messages` (via `invoke_llm`) correctly
//! injects skill listing attachments based on the `SkillListingProvider`
//! and `agent_skills` whitelist configuration.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::{LLMError, LlmCaller, SkillListingProvider};

use super::tmp_path;
use crate::llm_session::{ConversationSession, InjectionPosition, MemoryInjection};
use closeclaw_common::processor::{ContentBlock, UnifiedResponse, UnifiedUsage};

// ---------------------------------------------------------------------------
// Mock SkillListingProvider
// ---------------------------------------------------------------------------

/// Mock `SkillListingProvider` that returns configurable listing content.
/// Tracks call arguments for assertion.
struct MockSkillListingProvider {
    /// Listing content to return. Protected by `Mutex` so per-turn
    /// refresh tests can swap the value between calls.
    listing: Mutex<String>,
    /// Recorded `(agent_id, agent_skills)` arguments from each call.
    calls: Mutex<Vec<CallArgs>>,
    /// Optional counter for tracking call count.
    call_count: AtomicUsize,
}

type CallArgs = (Option<String>, Option<Vec<String>>);

impl MockSkillListingProvider {
    fn new(listing: impl Into<String>) -> Self {
        Self {
            listing: Mutex::new(listing.into()),
            calls: Mutex::new(Vec::new()),
            call_count: AtomicUsize::new(0),
        }
    }

    /// Replace the listing content (for per-turn refresh tests).
    fn set_listing(&self, listing: impl Into<String>) {
        *self.listing.lock().unwrap() = listing.into();
    }

    /// Return the number of times `generate_listing` was called.
    fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    /// Return recorded arguments for the Nth call (0-indexed).
    fn call_args(&self, idx: usize) -> CallArgs {
        self.calls.lock().unwrap()[idx].clone()
    }
}

impl SkillListingProvider for MockSkillListingProvider {
    fn generate_listing(&self, agent_id: Option<&str>, agent_skills: Option<&[String]>) -> String {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.calls.lock().unwrap().push((
            agent_id.map(|s| s.to_string()),
            agent_skills.map(|s| s.to_vec()),
        ));
        self.listing.lock().unwrap().clone()
    }

    fn generate_listing_excluding_conditional(
        &self,
        agent_id: Option<&str>,
        agent_skills: Option<&[String]>,
    ) -> String {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.calls.lock().unwrap().push((
            agent_id.map(|s| s.to_string()),
            agent_skills.map(|s| s.to_vec()),
        ));
        self.listing.lock().unwrap().clone()
    }

    fn find_conditional_matches(
        &self,
        _paths: &[std::path::PathBuf],
    ) -> Vec<closeclaw_common::ConditionalSkillMatch> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// FakeLlmCaller (captures request for assertion)
// ---------------------------------------------------------------------------

struct FakeLlmCaller {
    response: UnifiedResponse,
    last_request: Mutex<Option<InternalRequest>>,
}

impl FakeLlmCaller {
    fn new(response: UnifiedResponse) -> Self {
        Self {
            response,
            last_request: Mutex::new(None),
        }
    }

    fn last_request(&self) -> Option<InternalRequest> {
        self.last_request.lock().unwrap().clone()
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
        Err(LLMError::ApiError("not implemented in test".into()))
    }
}

fn canned_response(text: &str) -> UnifiedResponse {
    UnifiedResponse {
        content_blocks: vec![ContentBlock::Text(text.into())],
        usage: UnifiedUsage {
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

// ── 1. Normal path: provider with skills → tool role message at position 0 ──

#[tokio::test]
async fn test_skill_listing_injected_at_position_zero() {
    let mock = Arc::new(MockSkillListingProvider::new(
        "## Available Skills\n- skill_a\n- skill_b",
    ));
    let mut session = ConversationSession::new("s1".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(mock.clone());

    let fake = Arc::new(FakeLlmCaller::new(canned_response("ok")));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let result = session.invoke_llm("hello").await.unwrap();
    assert_eq!(result.content_blocks[0], ContentBlock::Text("ok".into()));

    // Inspect captured request messages
    let req = fake_ref.last_request().unwrap();
    assert!(req.messages.len() >= 2, "should have skill listing + user");
    assert_eq!(req.messages[0].role, "tool");
    assert!(req.messages[0].content.contains("## Available Skills"));
    assert_eq!(req.messages[1].role, "user");
    assert_eq!(req.messages[1].content, "hello");

    // Provider was called once
    assert_eq!(mock.call_count(), 1);
}

// ── 2a. Empty provider: None → no tool role message ────────────────────────

#[tokio::test]
async fn test_no_skill_listing_when_provider_is_none() {
    let mut session = ConversationSession::new("s2a".into(), "m".into(), tmp_path());
    // No provider set

    let fake = Arc::new(FakeLlmCaller::new(canned_response("ok")));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    // Only user message, no tool role
    assert_eq!(req.messages.len(), 1);
    assert_eq!(req.messages[0].role, "user");
    assert_eq!(req.messages[0].content, "hello");
}

// ── 2b. Empty provider: provider returns empty string → no tool role ───────

#[tokio::test]
async fn test_no_skill_listing_when_provider_returns_empty() {
    let mock = Arc::new(MockSkillListingProvider::new(""));
    let mut session = ConversationSession::new("s2b".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(mock);

    let fake = Arc::new(FakeLlmCaller::new(canned_response("ok")));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    assert_eq!(req.messages.len(), 1);
    assert_eq!(req.messages[0].role, "user");
}

// ── 3. Whitelist filtering: agent_skills passed to provider ─────────────────

#[tokio::test]
async fn test_whitelist_passed_to_provider() {
    let mock = Arc::new(MockSkillListingProvider::new("filtered skills"));
    let mut session = ConversationSession::new("s3".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(mock.clone());
    session.set_agent_skills(vec!["skill_a".into(), "skill_c".into()]);

    let fake = Arc::new(FakeLlmCaller::new(canned_response("ok")));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();

    // Provider should have been called with the whitelist
    let (_, agent_skills) = mock.call_args(0);
    assert_eq!(
        agent_skills,
        Some(vec!["skill_a".to_string(), "skill_c".to_string()])
    );

    // Message should contain the listing
    let req = fake_ref.last_request().unwrap();
    assert_eq!(req.messages[0].role, "tool");
    assert_eq!(req.messages[0].content, "filtered skills");
}

// ── 4. System prompt does not contain skill listing ─────────────────────────
// (Tested in system_prompt crate — see builder test)

// ── 5. Per-turn refresh: provider content changes between calls ─────────────

#[tokio::test]
async fn test_per_turn_refresh_returns_latest_listing() {
    let mock = Arc::new(MockSkillListingProvider::new("listing_v1"));
    let mut session = ConversationSession::new("s5".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(mock.clone());

    let fake = Arc::new(FakeLlmCaller::new(canned_response("ok")));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    // First call
    let _ = session.invoke_llm("turn1").await.unwrap();
    let req1 = fake_ref.last_request().unwrap();
    assert_eq!(req1.messages[0].content, "listing_v1");

    // Change provider content
    mock.set_listing("listing_v2");

    // Second call
    let _ = session.invoke_llm("turn2").await.unwrap();
    let req2 = fake_ref.last_request().unwrap();
    assert_eq!(req2.messages[0].content, "listing_v2");

    // Provider was called twice
    assert_eq!(mock.call_count(), 2);
}

// ── 6. Injection order: skill listing at position 0, memory after ───────────

#[tokio::test]
async fn test_skill_listing_before_memory_injection() {
    let mock = Arc::new(MockSkillListingProvider::new("skill_data"));
    let mut session = ConversationSession::new("s6".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(mock);

    // Set memory injection (AfterCurrent position)
    let injection = MemoryInjection::new("memory_context".into(), InjectionPosition::AfterCurrent);
    session.set_memory_injection(injection);

    let fake = Arc::new(FakeLlmCaller::new(canned_response("ok")));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    // Expected order: [skill_listing, user, memory_after_current]
    assert_eq!(req.messages.len(), 3);
    assert_eq!(req.messages[0].role, "tool");
    assert_eq!(req.messages[0].content, "skill_data");
    assert_eq!(req.messages[1].role, "user");
    assert_eq!(req.messages[1].content, "hello");
    assert_eq!(req.messages[2].role, "tool");
    assert_eq!(req.messages[2].content, "memory_context");
}

#[tokio::test]
async fn test_skill_listing_before_memory_before_next() {
    let mock = Arc::new(MockSkillListingProvider::new("skill_info"));
    let mut session = ConversationSession::new("s6b".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(mock);

    // Set memory injection (BeforeNext position)
    let injection = MemoryInjection::new("memory_pre".into(), InjectionPosition::BeforeNext);
    session.set_memory_injection(injection);

    let fake = Arc::new(FakeLlmCaller::new(canned_response("ok")));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    // Expected order: [skill_listing, memory_injection, user]
    assert_eq!(req.messages.len(), 3);
    assert_eq!(req.messages[0].role, "tool");
    assert_eq!(req.messages[0].content, "skill_info");
    assert_eq!(req.messages[1].role, "tool");
    assert_eq!(req.messages[1].content, "memory_pre");
    assert_eq!(req.messages[2].role, "user");
}

// ── setter/getter roundtrip ─────────────────────────────────────────────────

#[test]
fn test_set_and_get_skill_listing_provider() {
    let mut session = ConversationSession::new("s_get".into(), "m".into(), tmp_path());
    assert!(session.skill_listing_provider().is_none());

    let mock: Arc<dyn SkillListingProvider> = Arc::new(MockSkillListingProvider::new("test"));
    session.set_skill_listing_provider(mock.clone());
    assert!(session.skill_listing_provider().is_some());

    let got = session.skill_listing_provider().unwrap();
    assert!(Arc::ptr_eq(got, &mock));
}

#[test]
fn test_set_and_get_agent_skills() {
    let mut session = ConversationSession::new("s_as".into(), "m".into(), tmp_path());
    assert!(session.agent_skills().is_none());

    session.set_agent_skills(vec!["a".into(), "b".into()]);
    let skills = session.agent_skills().unwrap();
    assert_eq!(skills, &["a", "b"]);
}
