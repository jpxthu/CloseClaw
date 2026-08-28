//! Tests for incremental skill listing injection and conditional
//! activation in `ConversationSession`.
//!
//! Verifies that `invoke_llm` correctly implements:
//! - Full listing on first turn
//! - Incremental diff on subsequent turns
//! - Conditional skill exclusion from initial listing
//! - Conditional activation via file path matching (current turn
//!   mark, next turn inject)
//! - No re-activation of already-activated skills
//! - No injection when listing is unchanged

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::{ConditionalSkillMatch, LLMError, LlmCaller, SkillListingProvider};

use super::tmp_path;
use crate::llm_session::ConversationSession;
use closeclaw_common::processor::{ContentBlock, UnifiedResponse, UnifiedUsage};

// ---------------------------------------------------------------------------
// Mock SkillListingProvider with conditional skill support
// ---------------------------------------------------------------------------

/// Mock provider that supports conditional skill matching.
struct MockConditionalProvider {
    all_listing: Mutex<String>,
    base_listing: Mutex<String>,
    conditional_rules: Mutex<Vec<(String, ConditionalSkillMatch)>>,
}

impl MockConditionalProvider {
    fn new(all_listing: impl Into<String>, base_listing: impl Into<String>) -> Self {
        Self {
            all_listing: Mutex::new(all_listing.into()),
            base_listing: Mutex::new(base_listing.into()),
            conditional_rules: Mutex::new(Vec::new()),
        }
    }

    fn add_conditional_rule(&self, path_pattern: impl Into<String>, skill: ConditionalSkillMatch) {
        self.conditional_rules
            .lock()
            .unwrap()
            .push((path_pattern.into(), skill));
    }

    fn set_all_listing(&self, listing: impl Into<String>) {
        *self.all_listing.lock().unwrap() = listing.into();
    }

    fn set_base_listing(&self, listing: impl Into<String>) {
        *self.base_listing.lock().unwrap() = listing.into();
    }
}

impl SkillListingProvider for MockConditionalProvider {
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

    #[allow(dead_code)]
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
/// The skill listing is always inserted at position 0 in the messages
/// list by `build_llm_messages_with_listing`.
fn skill_listing_messages(req: &InternalRequest) -> Vec<&str> {
    // Skill listing is always at position 0 when present
    if let Some(msg) = req.messages.first() {
        if msg.role == "system" {
            return vec![msg.content.as_str()];
        }
    }
    Vec::new()
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_first_turn_injects_full_listing() {
    let provider = Arc::new(MockConditionalProvider::new(
        "- **skill_a**: desc_a\n- **skill_b**: desc_b",
        "- **skill_a**: desc_a\n- **skill_b**: desc_b",
    ));
    let mut session = ConversationSession::new("s1".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    let tools = skill_listing_messages(&req);
    assert_eq!(tools.len(), 1);
    assert!(tools[0].contains("skill_a"));
    assert!(tools[0].contains("skill_b"));
}

#[tokio::test]
async fn test_no_change_no_injection() {
    let provider = Arc::new(MockConditionalProvider::new(
        "- **skill_a**: desc_a",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("s2".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("turn1").await.unwrap();
    let req1 = fake_ref.last_request().unwrap();
    assert_eq!(skill_listing_messages(&req1).len(), 1);

    let _ = session.invoke_llm("turn2").await.unwrap();
    let req2 = fake_ref.last_request().unwrap();
    assert_eq!(
        skill_listing_messages(&req2).len(),
        0,
        "no listing should be injected when nothing changed"
    );
}

#[tokio::test]
async fn test_new_skill_injected_incrementally() {
    let provider = Arc::new(MockConditionalProvider::new(
        "- **skill_a**: desc_a",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("s3".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider.clone());

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("turn1").await.unwrap();

    provider.set_all_listing("- **skill_a**: desc_a\n- **skill_c**: desc_c");
    provider.set_base_listing("- **skill_a**: desc_a\n- **skill_c**: desc_c");

    let _ = session.invoke_llm("turn2").await.unwrap();
    let req2 = fake_ref.last_request().unwrap();
    let tools = skill_listing_messages(&req2);
    assert_eq!(tools.len(), 1);
    assert!(tools[0].contains("skill_c"));
    assert!(!tools[0].contains("skill_a"));
}

#[tokio::test]
async fn test_conditional_skill_excluded_from_initial() {
    let provider = Arc::new(MockConditionalProvider::new(
        "- **skill_a**: desc_a\n- **rs_helper**: rs desc ⚡ auto-activates on: *.rs",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("s4".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();
    let req = fake_ref.last_request().unwrap();
    let tools = skill_listing_messages(&req);
    assert_eq!(tools.len(), 1);
    assert!(
        !tools[0].contains("rs_helper"),
        "conditional skill should not appear in initial listing"
    );
    assert!(tools[0].contains("skill_a"));
}

#[tokio::test]
async fn test_conditional_activation_next_turn() {
    let provider = Arc::new(MockConditionalProvider::new(
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

    let mut session = ConversationSession::new("s5".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider.clone());

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    // First turn: no file paths
    let _ = session.invoke_llm("hello").await.unwrap();
    let req1 = fake_ref.last_request().unwrap();
    let tools1 = skill_listing_messages(&req1);
    assert_eq!(tools1.len(), 1);
    assert!(!tools1[0].contains("rs_helper"));

    // Second turn: .rs file → marks activation, not injected yet
    let _ = session.invoke_llm("edit src/main.rs please").await.unwrap();
    let req2 = fake_ref.last_request().unwrap();
    let tools2 = skill_listing_messages(&req2);
    assert_eq!(
        tools2.len(),
        0,
        "current turn should not inject conditional skill yet"
    );

    // Third turn: activated skill appears as incremental
    let _ = session.invoke_llm("continue").await.unwrap();
    let req3 = fake_ref.last_request().unwrap();
    let tools3 = skill_listing_messages(&req3);
    assert_eq!(tools3.len(), 1);
    assert!(tools3[0].contains("rs_helper"));
    assert!(tools3[0].contains("⚡"));
}

#[tokio::test]
async fn test_no_reactivation_of_already_activated() {
    let provider = Arc::new(MockConditionalProvider::new(
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

    let mut session = ConversationSession::new("s6".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider.clone());

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();
    let _ = session.invoke_llm("edit src/main.rs").await.unwrap();
    let _ = session.invoke_llm("continue").await.unwrap();
    let req3 = fake_ref.last_request().unwrap();
    assert_eq!(skill_listing_messages(&req3).len(), 1);

    // Same .rs path again → no new injection
    let _ = session.invoke_llm("edit src/lib.rs").await.unwrap();
    let req4 = fake_ref.last_request().unwrap();
    assert_eq!(
        skill_listing_messages(&req4).len(),
        0,
        "already activated skill should not trigger new injection"
    );
}

#[tokio::test]
async fn test_no_provider_no_listing() {
    let mut session = ConversationSession::new("s7".into(), "m".into(), tmp_path());
    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();
    let req = fake_ref.last_request().unwrap();
    assert_eq!(skill_listing_messages(&req).len(), 0);
}

#[tokio::test]
async fn test_empty_listing_no_injection() {
    let provider = Arc::new(MockConditionalProvider::new("", ""));
    let mut session = ConversationSession::new("s8".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();
    let req = fake_ref.last_request().unwrap();
    assert_eq!(skill_listing_messages(&req).len(), 0);
}

#[tokio::test]
async fn test_selective_conditional_activation() {
    let provider = Arc::new(MockConditionalProvider::new(
        "- **skill_a**: desc_a\n\
         - **rs_helper**: rs desc ⚡ auto-activates on: *.rs\n\
         - **py_helper**: py desc ⚡ auto-activates on: *.py",
        "- **skill_a**: desc_a",
    ));
    provider.add_conditional_rule(
        ".rs",
        ConditionalSkillMatch {
            name: "rs_helper".into(),
            listing_line: "- **rs_helper**: rs desc ⚡ auto-activates on: *.rs".into(),
        },
    );
    provider.add_conditional_rule(
        ".py",
        ConditionalSkillMatch {
            name: "py_helper".into(),
            listing_line: "- **py_helper**: py desc ⚡ auto-activates on: *.py".into(),
        },
    );

    let mut session = ConversationSession::new("s9".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider.clone());

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();
    let _ = session.invoke_llm("edit src/main.rs").await.unwrap();
    let _ = session.invoke_llm("continue").await.unwrap();
    let req3 = fake_ref.last_request().unwrap();
    let tools3 = skill_listing_messages(&req3);
    assert_eq!(tools3.len(), 1);
    assert!(tools3[0].contains("rs_helper"));
    assert!(!tools3[0].contains("py_helper"));
}

#[tokio::test]
async fn test_removed_skill_disappears() {
    let provider = Arc::new(MockConditionalProvider::new(
        "- **skill_a**: desc_a\n- **skill_b**: desc_b",
        "- **skill_a**: desc_a\n- **skill_b**: desc_b",
    ));
    let mut session = ConversationSession::new("s10".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider.clone());

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("turn1").await.unwrap();
    let req1 = fake_ref.last_request().unwrap();
    assert!(skill_listing_messages(&req1)[0].contains("skill_b"));

    // Remove skill_b
    provider.set_all_listing("- **skill_a**: desc_a");
    provider.set_base_listing("- **skill_a**: desc_a");

    // Turn 2: deletion of skill_b → injection with `- ` prefix
    let _ = session.invoke_llm("turn2").await.unwrap();
    let req2 = fake_ref.last_request().unwrap();
    let tools2 = skill_listing_messages(&req2);
    assert_eq!(tools2.len(), 1, "should inject deletion notification");
    assert!(
        tools2[0].contains("- - **skill_b**"),
        "deletion line should start with `- - **skill_b**"
    );

    // Turn 3: snapshot updated, stable
    let _ = session.invoke_llm("turn3").await.unwrap();
    let req3 = fake_ref.last_request().unwrap();
    assert_eq!(skill_listing_messages(&req3).len(), 0);
}

// ── Scenario 3: file change + conditional activation simultaneously ──────

/// When a daemon hot-reload changes the base listing AND the user
/// message contains a file path that triggers conditional activation
/// in the same turn, the diff should capture both sources of change:
/// the base listing update (file change) and the newly activated
/// conditional skill.
#[tokio::test]
async fn test_file_change_and_conditional_activation_same_turn() {
    let provider = Arc::new(MockConditionalProvider::new(
        "- **skill_a**: desc_a
- **skill_b**: desc_b",
        "- **skill_a**: desc_a
- **skill_b**: desc_b",
    ));
    provider.add_conditional_rule(
        ".rs",
        ConditionalSkillMatch {
            name: "rs_helper".into(),
            listing_line: "- **rs_helper**: rs desc ⚡ auto-activates on: *.rs".into(),
        },
    );

    let mut session = ConversationSession::new("s_file_cond".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider.clone());

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    // Turn 1: establish baseline
    let _ = session.invoke_llm("hello").await.unwrap();
    let req1 = fake_ref.last_request().unwrap();
    let tools1 = skill_listing_messages(&req1);
    assert_eq!(tools1.len(), 1);
    assert!(tools1[0].contains("skill_a"));
    assert!(tools1[0].contains("skill_b"));
    assert!(!tools1[0].contains("rs_helper"));

    // Simulate daemon hot-reload: skill_b removed, skill_c added
    // Also include rs_helper in the all_listing (simulating the real
    // listing which includes activated conditionals)
    provider.set_all_listing(
        "- **skill_a**: desc_a
- **skill_c**: desc_c
- **rs_helper**: rs desc ⚡ auto-activates on: *.rs",
    );
    provider.set_base_listing(
        "- **skill_a**: desc_a
- **skill_c**: desc_c",
    );

    // Turn 2: file change (listing updated) + .rs path triggers
    // conditional activation in the same turn
    let _ = session
        .invoke_llm("edit src/main.rs for the feature")
        .await
        .unwrap();
    let req2 = fake_ref.last_request().unwrap();
    let tools2 = skill_listing_messages(&req2);
    assert_eq!(
        tools2.len(),
        1,
        "should inject a diff on the turn with changes"
    );
    // The diff should show: skill_b removed, skill_c added
    assert!(
        tools2[0].contains("- - **skill_b**"),
        "diff should include deletion of skill_b"
    );
    assert!(
        tools2[0].contains("skill_c"),
        "diff should include addition of skill_c"
    );
    // rs_helper is NOT yet activated this turn (activation applies
    // next turn), so it should not appear in this turn's listing
    assert!(!tools2[0].contains("rs_helper"));

    // Turn 3: rs_helper activated from last turn's path match
    let _ = session.invoke_llm("continue").await.unwrap();
    let req3 = fake_ref.last_request().unwrap();
    let tools3 = skill_listing_messages(&req3);
    assert_eq!(tools3.len(), 1);
    assert!(tools3[0].contains("rs_helper"));
    // skill_c is already in the snapshot, so it should NOT appear in the diff
    assert!(!tools3[0].contains("skill_c"));
}

// ── File change that deactivates a conditional skill ─────────────────────

/// When the daemon removes a skill from the listing, and a
/// conditional skill was previously activated, the diff should
/// reflect the removal of both the base skill and the conditional.
#[tokio::test]
async fn test_file_change_removes_base_skill_with_conditional_active() {
    let provider = Arc::new(MockConditionalProvider::new(
        "- **skill_a**: desc_a
- **rs_helper**: rs desc ⚡ auto-activates on: *.rs",
        "- **skill_a**: desc_a",
    ));
    provider.add_conditional_rule(
        ".rs",
        ConditionalSkillMatch {
            name: "rs_helper".into(),
            listing_line: "- **skill_a**: desc_a
- **rs_helper**: rs desc ⚡ auto-activates on: *.rs"
                .into(),
        },
    );

    let mut session = ConversationSession::new("s_file_rm".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider.clone());

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    // Turn 1: baseline with skill_a only
    let _ = session.invoke_llm("hello").await.unwrap();
    let req1 = fake_ref.last_request().unwrap();
    let tools1 = skill_listing_messages(&req1);
    assert_eq!(tools1.len(), 1);
    assert!(tools1[0].contains("skill_a"));

    // Turn 2: .rs path → marks rs_helper for activation
    let _ = session.invoke_llm("edit src/lib.rs").await.unwrap();
    // Turn 3: rs_helper now activated → incremental injection
    let _ = session.invoke_llm("continue").await.unwrap();
    let req3 = fake_ref.last_request().unwrap();
    let tools3 = skill_listing_messages(&req3);
    assert_eq!(tools3.len(), 1);
    assert!(tools3[0].contains("rs_helper"));

    // Daemon removes skill_a from listing (but keeps something else
    // so the listing is not completely empty)
    provider.set_all_listing("- **skill_d**: desc_d");
    provider.set_base_listing("- **skill_d**: desc_d");

    // Turn 4: listing changes → diff should show removals
    let _ = session.invoke_llm("turn4").await.unwrap();
    let req4 = fake_ref.last_request().unwrap();
    let tools4 = skill_listing_messages(&req4);
    assert_eq!(tools4.len(), 1, "should inject diff for removed skills");
    assert!(
        tools4[0].contains("- - **skill_a**"),
        "diff should show skill_a removal"
    );
}
