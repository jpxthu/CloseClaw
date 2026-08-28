//! Tests for compaction protection of skill listing state.
//!
//! Verifies that `mark_compacted` + `preserve_listing_on_compaction`
//! cause the full skill listing to be re-injected on the first turn
//! after compaction, and that subsequent turns resume normal
//! incremental diff.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::{ConditionalSkillMatch, LLMError, LlmCaller, SkillListingProvider};

use super::tmp_path;
use crate::llm_session::{ConversationSession, SessionMessage};
use crate::run_health::TranscriptOp;
use closeclaw_common::processor::{ContentBlock, UnifiedResponse, UnifiedUsage};

// ---------------------------------------------------------------------------
// Mock SkillListingProvider
// ---------------------------------------------------------------------------

struct MockProvider {
    all_listing: Mutex<String>,
    base_listing: Mutex<String>,
}

impl MockProvider {
    fn new(all: impl Into<String>, base: impl Into<String>) -> Self {
        Self {
            all_listing: Mutex::new(all.into()),
            base_listing: Mutex::new(base.into()),
        }
    }

    fn set_all_listing(&self, listing: impl Into<String>) {
        *self.all_listing.lock().unwrap() = listing.into();
    }

    fn set_base_listing(&self, listing: impl Into<String>) {
        *self.base_listing.lock().unwrap() = listing.into();
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

    fn find_conditional_matches(
        &self,
        _paths: &[std::path::PathBuf],
    ) -> Vec<ConditionalSkillMatch> {
        Vec::new()
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
                    reasoning_tokens: None,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
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

/// Helper: extract the skill listing from system-role messages.
/// Skill listing messages are inserted at position 0 by
/// `build_llm_messages_with_listing`. They contain either `##` headers
/// or `- **` formatted entries, distinguishing them from generic system
/// role messages in conversation history (e.g. `[compacted]` summaries).
fn skill_listing_messages(req: &InternalRequest) -> Vec<&str> {
    req.messages
        .iter()
        .filter(|m| m.role == "system" && (m.content.contains("##") || m.content.contains("- **")))
        .map(|m| m.content.as_str())
        .collect()
}

/// Helper: simulate compaction by rewriting transcript to a summary.
fn simulate_compaction(session: &mut ConversationSession) {
    let summary = SessionMessage {
        role: "system".into(),
        content_blocks: vec![ContentBlock::Text(
            "[compacted] Previous conversation summary.".into(),
        )],
        timestamp: chrono::Utc::now(),
    };
    session.apply_transcript_op(TranscriptOp::Rewrite, vec![summary]);
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 1: Compaction clears snapshot → full listing re-injected
// ═══════════════════════════════════════════════════════════════════════════

/// After compaction, `preserve_listing_on_compaction` keeps the
/// snapshot intact, but `mark_compacted` sets the one-shot flag.
/// The next turn's `prepare_turn_skill_listing` clears the snapshot,
/// causing `compute_skill_listing_for_turn` to enter the "first turn"
/// branch and inject the full listing. Subsequent turns resume normal
/// incremental diff.
#[tokio::test]
async fn test_snapshot_survives_compaction() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a\n- **skill_b**: desc_b",
        "- **skill_a**: desc_a\n- **skill_b**: desc_b",
    ));
    let mut session = ConversationSession::new("s_compact_1".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    // Turn 1: establishes snapshot
    let _ = session.invoke_llm("hello").await.unwrap();
    assert!(
        session.skill_listing_snapshot().is_some(),
        "snapshot should exist after first turn"
    );
    let snapshot_before = session.skill_listing_snapshot().unwrap().to_string();

    // Simulate compaction
    simulate_compaction(&mut session);
    session.mark_compacted();
    session.preserve_listing_on_compaction();

    // Snapshot survives compaction (preserve_listing_on_compaction
    // does not clear it)
    assert_eq!(
        session.skill_listing_snapshot().unwrap(),
        snapshot_before,
        "snapshot should be unchanged after compaction"
    );

    // Turn 2: prepare_turn_skill_listing clears snapshot, so
    // compute_skill_listing_for_turn enters "first turn" branch
    // and injects the full listing
    let _ = session.invoke_llm("turn2").await.unwrap();
    let req = fake_ref.last_request().unwrap();
    let tools = skill_listing_messages(&req);
    assert_eq!(
        tools.len(),
        1,
        "full listing should be injected on first turn after compaction"
    );
    assert!(
        tools[0].contains("skill_a"),
        "injected listing should include skill_a"
    );
    assert!(
        tools[0].contains("skill_b"),
        "injected listing should include skill_b"
    );

    // Turn 3: snapshot exists and listing unchanged → no diff
    let _ = session.invoke_llm("turn3").await.unwrap();
    let req3 = fake_ref.last_request().unwrap();
    assert_eq!(
        skill_listing_messages(&req3).len(),
        0,
        "no listing should be injected when nothing changed"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 2: Compaction + hot-reload → full listing with new skill
// ═══════════════════════════════════════════════════════════════════════════

/// After compaction, a daemon hot-reload adds a new skill.
/// Turn 2 injects the full listing (including the new skill)
/// because the snapshot was cleared. Subsequent turns resume
/// normal incremental diff.
#[tokio::test]
async fn test_new_skill_detected_after_compaction() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("s_compact_2".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider.clone());

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    // Turn 1: baseline
    let _ = session.invoke_llm("hello").await.unwrap();
    assert!(session.skill_listing_snapshot().is_some());

    // Simulate compaction
    simulate_compaction(&mut session);
    session.mark_compacted();
    session.preserve_listing_on_compaction();

    // Daemon hot-reload: new skill_b appears
    provider.set_all_listing("- **skill_a**: desc_a\n- **skill_b**: desc_b");
    provider.set_base_listing("- **skill_a**: desc_a\n- **skill_b**: desc_b");

    // Turn 2: full listing injected (snapshot was cleared)
    let _ = session.invoke_llm("turn2").await.unwrap();
    let req = fake_ref.last_request().unwrap();
    let tools = skill_listing_messages(&req);
    assert_eq!(tools.len(), 1, "should inject full listing");
    assert!(
        tools[0].contains("skill_a"),
        "full listing should include skill_a"
    );
    assert!(
        tools[0].contains("skill_b"),
        "full listing should include new skill_b"
    );

    // Turn 3: snapshot exists, listing unchanged → no diff
    let _ = session.invoke_llm("turn3").await.unwrap();
    let req3 = fake_ref.last_request().unwrap();
    assert_eq!(
        skill_listing_messages(&req3).len(),
        0,
        "no diff expected when listing unchanged after injection"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 3: Edge — compaction when snapshot is empty (None)
// ═══════════════════════════════════════════════════════════════════════════

/// When no skill listing provider is configured, snapshot is None.
/// Compaction should not cause a panic or error.
#[tokio::test]
async fn test_compaction_with_empty_snapshot() {
    let mut session = ConversationSession::new("s_compact_3".into(), "m".into(), tmp_path());
    // No provider set — snapshot is None

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    // Turn 1: no listing injected
    let _ = session.invoke_llm("hello").await.unwrap();
    assert!(session.skill_listing_snapshot().is_none());

    // Simulate compaction
    simulate_compaction(&mut session);
    session.mark_compacted();
    session.preserve_listing_on_compaction();

    // State remains None
    assert!(session.skill_listing_snapshot().is_none());
    assert!(session.activated_conditional_skills().is_empty());

    // Turn 2: still no listing
    let _ = session.invoke_llm("turn2").await.unwrap();
    assert_eq!(
        skill_listing_messages(&fake_ref.last_request().unwrap()).len(),
        0
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 4: Edge — compaction when activated_conditional_skills is empty
// ═══════════════════════════════════════════════════════════════════════════

/// With a provider but no conditional skills activated, compaction
/// triggers full listing re-injection, then subsequent turns resume
/// incremental diff.
#[tokio::test]
async fn test_compaction_with_empty_activated_set() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("s_compact_4".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    // Turn 1
    let _ = session.invoke_llm("hello").await.unwrap();
    assert!(
        session.activated_conditional_skills().is_empty(),
        "no conditional skills should be activated yet"
    );

    // Simulate compaction
    simulate_compaction(&mut session);
    session.mark_compacted();
    session.preserve_listing_on_compaction();

    // State preserved
    assert!(session.skill_listing_snapshot().is_some());
    assert!(session.activated_conditional_skills().is_empty());

    // Turn 2: snapshot cleared → full listing injected
    let _ = session.invoke_llm("turn2").await.unwrap();
    let req = fake_ref.last_request().unwrap();
    let tools = skill_listing_messages(&req);
    assert_eq!(tools.len(), 1, "full listing should be injected");
    assert!(tools[0].contains("skill_a"), "should include skill_a");

    // Turn 3: no diff (listing unchanged)
    let _ = session.invoke_llm("turn3").await.unwrap();
    let req3 = fake_ref.last_request().unwrap();
    assert_eq!(skill_listing_messages(&req3).len(), 0);
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 5a: Compaction after hot-reload — full re-injection then normal
// ═══════════════════════════════════════════════════════════════════════════

/// After a hot-reload adds a skill, compaction triggers full
/// re-injection of the complete listing. The next turn resumes
/// normal incremental diff.
#[tokio::test]
async fn test_compaction_after_hot_reload_no_diff() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a\n- **skill_b**: desc_b",
        "- **skill_a**: desc_a\n- **skill_b**: desc_b",
    ));
    let mut session = ConversationSession::new("s_compact_5a".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider.clone());

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    // Turn 1: establishes snapshot with skill_a + skill_b
    let _ = session.invoke_llm("hello").await.unwrap();
    let snap1 = session.skill_listing_snapshot().unwrap().to_string();
    assert!(snap1.contains("skill_a"));
    assert!(snap1.contains("skill_b"));

    // Turn 2: add skill_c via daemon hot-reload
    let listing_a_b_c = "- **skill_a**: desc_a\n- **skill_b**: desc_b\n- **skill_c**: desc_c";
    provider.set_all_listing(listing_a_b_c);
    provider.set_base_listing(listing_a_b_c);
    let _ = session.invoke_llm("turn2").await.unwrap();
    let req2 = fake_ref.last_request().unwrap();
    let tools2 = skill_listing_messages(&req2);
    assert_eq!(tools2.len(), 1);
    assert!(tools2[0].contains("skill_c"));
    let snap2 = session.skill_listing_snapshot().unwrap().to_string();
    assert!(snap2.contains("skill_c"));

    // Simulate compaction
    simulate_compaction(&mut session);
    session.mark_compacted();
    session.preserve_listing_on_compaction();

    // Snapshot survives compaction with the full current listing
    let snap_after = session.skill_listing_snapshot().unwrap().to_string();
    assert!(snap_after.contains("skill_a"));
    assert!(snap_after.contains("skill_b"));
    assert!(snap_after.contains("skill_c"));

    // Turn 3: snapshot cleared → full listing injected
    let _ = session.invoke_llm("turn3").await.unwrap();
    let req3 = fake_ref.last_request().unwrap();
    let tools3 = skill_listing_messages(&req3);
    assert_eq!(
        tools3.len(),
        1,
        "full listing should be injected on first turn after compaction"
    );
    assert!(tools3[0].contains("skill_a"));
    assert!(tools3[0].contains("skill_b"));
    assert!(tools3[0].contains("skill_c"));

    // Turn 4: no diff (listing unchanged post-injection)
    let _ = session.invoke_llm("turn4").await.unwrap();
    let req4 = fake_ref.last_request().unwrap();
    assert_eq!(
        skill_listing_messages(&req4).len(),
        0,
        "no diff expected when listing unchanged after injection"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 5b: Skill removal after compaction → full re-injection
// ═══════════════════════════════════════════════════════════════════════════

/// After compaction, skill removal by daemon hot-reload results in
/// a full listing re-injection (without the removed skill).
#[tokio::test]
async fn test_removal_detected_after_compaction() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a\n- **skill_b**: desc_b",
        "- **skill_a**: desc_a\n- **skill_b**: desc_b",
    ));
    let mut session = ConversationSession::new("s_compact_5b".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider.clone());

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    // Turn 1: establish snapshot
    let _ = session.invoke_llm("hello").await.unwrap();
    assert!(session.skill_listing_snapshot().is_some());

    // Simulate compaction
    simulate_compaction(&mut session);
    session.mark_compacted();
    session.preserve_listing_on_compaction();

    // Daemon removes skill_b
    provider.set_all_listing("- **skill_a**: desc_a\n- **skill_c**: desc_c");
    provider.set_base_listing("- **skill_a**: desc_a\n- **skill_c**: desc_c");

    // Turn 2: full listing injected (snapshot was cleared)
    let _ = session.invoke_llm("turn2").await.unwrap();
    let req = fake_ref.last_request().unwrap();
    let tools = skill_listing_messages(&req);
    assert_eq!(tools.len(), 1, "full listing should be injected");
    assert!(tools[0].contains("skill_a"), "should include skill_a");
    assert!(tools[0].contains("skill_c"), "should include new skill_c");
    assert!(
        !tools[0].contains("skill_b"),
        "removed skill_b should not appear in full listing"
    );

    // Turn 3: no diff (listing unchanged)
    let _ = session.invoke_llm("turn3").await.unwrap();
    let req3 = fake_ref.last_request().unwrap();
    assert_eq!(skill_listing_messages(&req3).len(), 0);
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 5c: Compaction + hot-reload adds skill → full listing with new skill
// ═══════════════════════════════════════════════════════════════════════════

/// Compaction followed by a daemon hot-reload that adds a new skill.
/// Turn 2 injects the full listing including the new skill.
#[tokio::test]
async fn test_compaction_with_listing_change() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("s_compact_5c".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider.clone());

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    // Turn 1: baseline with skill_a only
    let _ = session.invoke_llm("hello").await.unwrap();
    let snap1 = session.skill_listing_snapshot().unwrap().to_string();
    assert!(snap1.contains("skill_a"));
    assert!(!snap1.contains("skill_b"));

    // Simulate compaction
    simulate_compaction(&mut session);
    session.mark_compacted();
    session.preserve_listing_on_compaction();

    // Daemon hot-reload: new skill_b appears
    provider.set_all_listing("- **skill_a**: desc_a\n- **skill_b**: desc_b");
    provider.set_base_listing("- **skill_a**: desc_a\n- **skill_b**: desc_b");

    // Turn 2: full listing injected (includes both skills)
    let _ = session.invoke_llm("turn2").await.unwrap();
    let req = fake_ref.last_request().unwrap();
    let tools = skill_listing_messages(&req);
    assert_eq!(tools.len(), 1, "full listing should be injected");
    assert!(
        tools[0].contains("skill_a"),
        "should include existing skill_a"
    );
    assert!(
        tools[0].contains("skill_b"),
        "should include new skill_b from hot-reload"
    );

    // Verify snapshot updated with new listing
    let snap2 = session.skill_listing_snapshot().unwrap().to_string();
    assert!(snap2.contains("skill_a"));
    assert!(snap2.contains("skill_b"));

    // Turn 3: no diff (listing unchanged)
    let _ = session.invoke_llm("turn3").await.unwrap();
    let req3 = fake_ref.last_request().unwrap();
    assert_eq!(
        skill_listing_messages(&req3).len(),
        0,
        "no diff expected after full re-injection"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Test 6: `apply_transcript_op` does not touch skill listing fields
// ═══════════════════════════════════════════════════════════════════════════

/// Directly verifies that `apply_transcript_op` (the compaction
/// mechanism) only modifies `messages` and `last_activity_at`,
/// leaving skill listing state untouched.
#[tokio::test]
async fn test_apply_transcript_op_preserves_skill_listing() {
    let mut session = ConversationSession::new("s_compact_6".into(), "m".into(), tmp_path());

    // Manually set skill listing state
    session.skill_listing_snapshot = Some("snapshot_data".into());
    session
        .activated_conditional_skills
        .insert("test_skill".into());

    let snap_before = session.skill_listing_snapshot().unwrap().to_string();
    let activated_before: HashSet<String> = session
        .activated_conditional_skills()
        .iter()
        .cloned()
        .collect();

    // Apply a transcript rewrite (compaction)
    let new_msgs = vec![SessionMessage {
        role: "system".into(),
        content_blocks: vec![ContentBlock::Text("summary".into())],
        timestamp: chrono::Utc::now(),
    }];
    session.apply_transcript_op(TranscriptOp::Rewrite, new_msgs);

    // Skill listing state unchanged
    assert_eq!(
        session.skill_listing_snapshot().unwrap(),
        snap_before,
        "apply_transcript_op should not modify snapshot"
    );
    let activated_after: HashSet<String> = session
        .activated_conditional_skills()
        .iter()
        .cloned()
        .collect();
    assert_eq!(
        activated_before, activated_after,
        "apply_transcript_op should not modify activated_conditional_skills"
    );
}
