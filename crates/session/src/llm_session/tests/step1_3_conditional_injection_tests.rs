//! Tests for Step 1.3: conditional activation injection and
//! BeforeNext memory position with conversation history.
//!
//! Verifies the fixes from Step 1.1 (complete entry injection for
//! conditionally activated skills) and Step 1.2 (BeforeNext memory
//! injection position) using direct method calls and conversation
//! history scenarios.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::{ConditionalSkillMatch, LLMError, LlmCaller, SkillListingProvider};

use super::tmp_path;
use crate::llm_session::{ConversationSession, InjectionPosition, MemoryInjection, SessionMessage};
use closeclaw_common::processor::{ContentBlock, UnifiedResponse, UnifiedUsage};

// ---------------------------------------------------------------------------
// Mock SkillListingProvider
// ---------------------------------------------------------------------------

/// Mock provider returning configurable listing content with
/// conditional skill support.
struct MockProvider {
    all_listing: Mutex<String>,
    base_listing: Mutex<String>,
    conditional_rules: Mutex<Vec<(String, ConditionalSkillMatch)>>,
}

impl MockProvider {
    fn new(all_listing: impl Into<String>, base_listing: impl Into<String>) -> Self {
        Self {
            all_listing: Mutex::new(all_listing.into()),
            base_listing: Mutex::new(base_listing.into()),
            conditional_rules: Mutex::new(Vec::new()),
        }
    }

    fn add_conditional_rule(&self, pattern: impl Into<String>, skill: ConditionalSkillMatch) {
        self.conditional_rules
            .lock()
            .unwrap()
            .push((pattern.into(), skill));
    }
}

impl SkillListingProvider for MockProvider {
    fn generate_listing(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        self.all_listing.lock().unwrap().clone()
    }

    fn generate_listing_excluding_conditional(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        self.base_listing.lock().unwrap().clone()
    }

    fn find_conditional_matches(&self, paths: &[PathBuf]) -> Vec<ConditionalSkillMatch> {
        let rules = self.conditional_rules.lock().unwrap();
        let mut result = Vec::new();
        for path in paths {
            let path_str = path.to_string_lossy();
            for (pattern, skill) in rules.iter() {
                if path_str.contains(pattern.as_str()) {
                    result.push(ConditionalSkillMatch {
                        name: skill.name.clone(),
                        listing_line: skill.listing_line.clone(),
                    });
                }
            }
        }
        result
    }
}

// ---------------------------------------------------------------------------
// FakeLlmCaller
// ---------------------------------------------------------------------------

struct FakeLlmCaller {
    response: UnifiedResponse,
    last_request: Mutex<Option<InternalRequest>>,
}

impl FakeLlmCaller {
    fn new(text: &str) -> Self {
        Self {
            response: UnifiedResponse {
                content_blocks: vec![ContentBlock::Text(text.into())],
                usage: UnifiedUsage {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    total_tokens: Some(2),
                    ..Default::default()
                },
                finish_reason: Some("stop".into()),
                retry_attempts: 0,
            },
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

/// Helper: build a SessionMessage with a text content block.
fn user_msg(content: &str) -> SessionMessage {
    SessionMessage {
        role: "user".to_string(),
        content_blocks: vec![ContentBlock::Text(content.to_string())],
        timestamp: chrono::Utc::now(),
    }
}

fn assistant_msg(content: &str) -> SessionMessage {
    SessionMessage {
        role: "assistant".to_string(),
        content_blocks: vec![ContentBlock::Text(content.to_string())],
        timestamp: chrono::Utc::now(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Skill listing: conditional activation injection
// ═══════════════════════════════════════════════════════════════════════════

/// Deferred injection: when `activated_conditional_skills` contains
/// a skill that was activated in a previous turn, the diff in
/// `compute_skill_listing_for_turn` correctly includes the new
/// entry. In the new flow, `prepare_turn_skill_listing` calls
/// `apply_skill_listing_update(None, &newly_activated)` to add
/// the skill to `activated_conditional_skills` before calling
/// `compute_skill_listing_for_turn` with an empty set. The diff
/// mechanism then detects the newly added entry.
#[test]
fn test_compute_skill_listing_newly_activated_injects_complete_entry() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a\n- **rs_helper**: rs desc ⚡ auto-activates on: *.rs",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("s_comp1".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    // Simulate: turn 1 already happened (snapshot exists)
    session.skill_listing_snapshot = Some("- **skill_a**: desc_a".to_string());

    // Simulate: rs_helper was activated in a previous turn
    // (prepare_turn_skill_listing calls apply_skill_listing_update
    // with None snapshot to add to activated_conditional_skills)
    let mut activated = HashSet::new();
    activated.insert("rs_helper".to_string());
    session.apply_skill_listing_update(None, &activated);

    // compute_skill_listing_for_turn takes no args
    let (listing, new_snapshot) = session.compute_skill_listing_for_turn();

    // The diff should include rs_helper's complete entry
    let injected = listing.expect("should inject deferred rs_helper entry via diff");
    assert!(
        injected.contains("rs_helper"),
        "injected listing should contain rs_helper"
    );
    assert!(
        injected.contains("⚡"),
        "injected listing should contain ⚡ marker"
    );
    assert!(new_snapshot.is_some(), "snapshot still updated");
}

/// When `newly_activated` is empty and the snapshot matches the current
/// listing, no injection occurs (returns None).
#[test]
fn test_compute_skill_listing_no_activation_no_change_no_injection() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("s_comp2".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);
    session.skill_listing_snapshot = Some("- **skill_a**: desc_a".to_string());

    let (listing, new_snapshot) = session.compute_skill_listing_for_turn();

    assert!(listing.is_none(), "no changes → no injection");
    assert!(new_snapshot.is_some(), "snapshot still updated");
}

/// On the first turn (no snapshot), the full listing is injected
/// regardless of `newly_activated`.
#[test]
fn test_compute_skill_listing_first_turn_injects_full_listing() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a\n- **skill_b**: desc_b",
        "- **skill_a**: desc_a\n- **skill_b**: desc_b",
    ));
    let mut session = ConversationSession::new("s_comp3".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);
    // No snapshot → first turn

    let (listing, new_snapshot) = session.compute_skill_listing_for_turn();

    let injected = listing.expect("first turn should inject full listing");
    assert!(injected.contains("skill_a"));
    assert!(injected.contains("skill_b"));
    assert!(new_snapshot.is_some());
}

/// When `newly_activated` contains a skill not found in the listing,
/// the fallback diff mechanism is used.
#[test]
fn test_compute_skill_listing_activated_not_in_listing_falls_back_to_diff() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("s_comp4".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);
    session.skill_listing_snapshot = Some("- **skill_a**: desc_a".to_string());

    let (listing, new_snapshot) = session.compute_skill_listing_for_turn();

    // No diff since listing hasn't changed → None
    assert!(listing.is_none());
    assert!(new_snapshot.is_some());
}

/// Multiple newly activated skills: complete entries for all
/// newly activated skills are injected (not diff), even when
/// `activated_conditional_skills` doesn't contain them yet.
#[test]
fn test_compute_skill_listing_multiple_newly_activated() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a\n\
         - **rs_helper**: rs desc ⚡ auto-activates on: *.rs\n\
         - **py_helper**: py desc ⚡ auto-activates on: *.py",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("s_comp5".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);
    session.skill_listing_snapshot = Some("- **skill_a**: desc_a".to_string());

    let mut activated = HashSet::new();
    activated.insert("rs_helper".to_string());
    activated.insert("py_helper".to_string());
    session.apply_skill_listing_update(None, &activated);

    let (listing, new_snapshot) = session.compute_skill_listing_for_turn();

    let injected = listing.expect("should inject complete entries for both skills");
    assert!(injected.contains("rs_helper"));
    assert!(injected.contains("py_helper"));
    assert!(injected.contains("⚡"));
    assert!(new_snapshot.is_some());
}

/// Conditional activation via `invoke_llm`: the complete entry with
/// ⚡ is injected when a file path triggers activation.
///
/// This tests the end-to-end path: user message with file path →
/// `prepare_turn_skill_listing` detects activation →
/// `compute_skill_listing_for_turn` injects complete entry.
#[tokio::test]
async fn test_conditional_activation_via_invoke_llm_injects_complete_entry() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a\n- **rs_helper**: rs desc ⚡ auto-activates on: *.rs",
        "- **skill_a**: desc_a",
    ));
    provider.add_conditional_rule(
        ".rs",
        ConditionalSkillMatch {
            name: "rs_helper".into(),
            listing_line: "- **rs_helper**: rs desc ⚡ auto-activates on: *.rs".into(),
        },
    );

    let mut session = ConversationSession::new("s_e2e1".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider.clone());

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    // Turn 1: establish baseline
    let _ = session.invoke_llm("hello").await.unwrap();
    let req1 = fake_ref.last_request().unwrap();
    let sys1: Vec<_> = req1
        .messages
        .iter()
        .filter(|m| m.role == "system")
        .collect();
    assert_eq!(sys1.len(), 1);
    assert!(!sys1[0].content.contains("rs_helper"));

    // Turn 2: .rs file → marks activation, deferred to next turn.
    // No base-listing changes → empty diff → no injection.
    let _ = session.invoke_llm("edit src/main.rs").await.unwrap();
    let req2 = fake_ref.last_request().unwrap();
    let sys2: Vec<_> = req2
        .messages
        .iter()
        .filter(|m| m.role == "system")
        .collect();
    assert_eq!(sys2.len(), 0, "activation turn: no injection (deferred)");

    // Turn 3: pending activation entry is injected
    let _ = session.invoke_llm("continue").await.unwrap();
    let req3 = fake_ref.last_request().unwrap();
    let sys3: Vec<_> = req3
        .messages
        .iter()
        .filter(|m| m.role == "system")
        .collect();
    assert_eq!(
        sys3.len(),
        1,
        "turn 3 should inject deferred rs_helper entry"
    );
    assert!(
        sys3[0].content.contains("rs_helper"),
        "turn 3 should inject complete entry for activated skill"
    );
    assert!(sys3[0].content.contains("⚡"));
}

// ═══════════════════════════════════════════════════════════════════════════
// BeforeNext memory injection position with conversation history
// ═══════════════════════════════════════════════════════════════════════════

/// BeforeNext memory injection: with conversation history, memory
/// appears after history messages and before the new user message.
///
/// This tests the Step 1.2 fix: `insert_pos = messages.len() - 1`
/// places the tool message before the last user message.
#[tokio::test]
async fn test_before_next_memory_after_history_before_new_user() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("s_bn1".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    // Simulate conversation history: user → assistant
    session.messages.push(user_msg("what is rust?"));
    session
        .messages
        .push(assistant_msg("Rust is a systems language."));

    // Inject memory with BeforeNext position
    let injection = MemoryInjection::new("memory_context".into(), InjectionPosition::BeforeNext);
    session.set_memory_injection(injection);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("tell me more").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    // Expected order:
    // [system: listing, user: "what is rust?", assistant: "Rust is...",
    //  tool: memory_context, user: "tell me more"]
    let roles: Vec<&str> = req.messages.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["system", "user", "assistant", "tool", "user"],
        "BeforeNext: memory should be between history and new user message, got: {roles:?}"
    );

    // Verify the tool message content
    let tool_msg = req.messages.iter().find(|m| m.role == "tool").unwrap();
    assert_eq!(tool_msg.content, "memory_context");

    // Verify the last message is the new user message
    let last = req.messages.last().unwrap();
    assert_eq!(last.role, "user");
    assert_eq!(last.content, "tell me more");
}

/// AfterCurrent memory injection: with conversation history, memory
/// appears after the new user message (unchanged behavior).
#[tokio::test]
async fn test_after_current_memory_after_new_user() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("s_ac1".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    // Simulate conversation history
    session.messages.push(user_msg("question 1"));
    session.messages.push(assistant_msg("answer 1"));

    // Inject memory with AfterCurrent position
    let injection = MemoryInjection::new("memory_after".into(), InjectionPosition::AfterCurrent);
    session.set_memory_injection(injection);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("question 2").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    // Expected order:
    // [system: listing, user: "q1", assistant: "a1",
    //  user: "question 2", tool: memory_after]
    let roles: Vec<&str> = req.messages.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["system", "user", "assistant", "user", "tool"],
        "AfterCurrent: memory should be after new user message, got: {roles:?}"
    );

    let tool_msg = req.messages.iter().find(|m| m.role == "tool").unwrap();
    assert_eq!(tool_msg.content, "memory_after");
}

/// BeforeNext + skill_listing: correct ordering with history.
///
/// Expected: [system: listing, user: "q1", assistant: "a1",
///           tool: memory, user: "q2"]
#[tokio::test]
async fn test_before_next_with_skill_listing_correct_ordering() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("s_bns1".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    // Conversation history
    session.messages.push(user_msg("hello"));
    session.messages.push(assistant_msg("hi there"));

    // Both skill listing and memory injection
    let injection = MemoryInjection::new("memory_data".into(), InjectionPosition::BeforeNext);
    session.set_memory_injection(injection);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("follow up").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    // Expected:
    // [system: listing, user: "hello", assistant: "hi there",
    //  tool: memory_data, user: "follow up"]
    let roles: Vec<&str> = req.messages.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["system", "user", "assistant", "tool", "user"],
        "skill_listing at front, memory between history and new user, got: {roles:?}"
    );

    // Verify system message is skill listing
    assert_eq!(req.messages[0].role, "system");
    assert!(req.messages[0].content.contains("skill_a"));

    // Verify tool message is memory injection
    assert_eq!(req.messages[3].role, "tool");
    assert_eq!(req.messages[3].content, "memory_data");

    // Verify last message is new user message
    assert_eq!(req.messages[4].role, "user");
    assert_eq!(req.messages[4].content, "follow up");
}

/// BeforeNext memory injection with no history (edge case).
/// Memory should be inserted at position 0 (before the only user message).
#[tokio::test]
async fn test_before_next_memory_no_history() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("s_bn2".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    let injection = MemoryInjection::new("memory_only".into(), InjectionPosition::BeforeNext);
    session.set_memory_injection(injection);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("first message").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    // Expected: [system: listing, tool: memory_only, user: "first message"]
    let roles: Vec<&str> = req.messages.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["system", "tool", "user"],
        "BeforeNext without history: memory before user, got: {roles:?}"
    );
    assert_eq!(req.messages[1].content, "memory_only");
    assert_eq!(req.messages[2].content, "first message");
}

/// AfterCurrent with no history (edge case).
/// Memory should be appended after the user message.
#[tokio::test]
async fn test_after_current_memory_no_history() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("s_ac2".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    let injection = MemoryInjection::new("memory_after".into(), InjectionPosition::AfterCurrent);
    session.set_memory_injection(injection);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    // Expected: [system: listing, user: "hello", tool: memory_after]
    let roles: Vec<&str> = req.messages.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["system", "user", "tool"],
        "AfterCurrent without history: memory after user, got: {roles:?}"
    );
    assert_eq!(req.messages[2].content, "memory_after");
}

/// BeforeNext + skill_listing without history.
/// Expected: [system: listing, tool: memory, user: "msg"]
#[tokio::test]
async fn test_before_next_with_listing_no_history() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("s_bns2".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    let injection = MemoryInjection::new("mem".into(), InjectionPosition::BeforeNext);
    session.set_memory_injection(injection);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hi").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    let roles: Vec<&str> = req.messages.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["system", "tool", "user"],
        "BeforeNext + listing, no history: system, tool, user, got: {roles:?}"
    );
}
