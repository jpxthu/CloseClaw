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

/// Helper: extract tool-role messages from a request.
fn tool_messages(req: &InternalRequest) -> Vec<&str> {
    req.messages
        .iter()
        .filter(|m| m.role == "tool")
        .map(|m| m.content.as_str())
        .collect()
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
    let tools = tool_messages(&req);
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
    assert_eq!(tool_messages(&req1).len(), 1);

    let _ = session.invoke_llm("turn2").await.unwrap();
    let req2 = fake_ref.last_request().unwrap();
    assert_eq!(
        tool_messages(&req2).len(),
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
    let tools = tool_messages(&req2);
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
    let tools = tool_messages(&req);
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
    let tools1 = tool_messages(&req1);
    assert_eq!(tools1.len(), 1);
    assert!(!tools1[0].contains("rs_helper"));

    // Second turn: .rs file → marks activation, not injected yet
    let _ = session.invoke_llm("edit src/main.rs please").await.unwrap();
    let req2 = fake_ref.last_request().unwrap();
    let tools2 = tool_messages(&req2);
    assert_eq!(
        tools2.len(),
        0,
        "current turn should not inject conditional skill yet"
    );

    // Third turn: activated skill appears as incremental
    let _ = session.invoke_llm("continue").await.unwrap();
    let req3 = fake_ref.last_request().unwrap();
    let tools3 = tool_messages(&req3);
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
    assert_eq!(tool_messages(&req3).len(), 1);

    // Same .rs path again → no new injection
    let _ = session.invoke_llm("edit src/lib.rs").await.unwrap();
    let req4 = fake_ref.last_request().unwrap();
    assert_eq!(
        tool_messages(&req4).len(),
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
    assert_eq!(tool_messages(&req).len(), 0);
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
    assert_eq!(tool_messages(&req).len(), 0);
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
    let tools3 = tool_messages(&req3);
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
    assert!(tool_messages(&req1)[0].contains("skill_b"));

    // Remove skill_b
    provider.set_all_listing("- **skill_a**: desc_a");
    provider.set_base_listing("- **skill_a**: desc_a");

    // Turn 2: no new additions → no injection
    let _ = session.invoke_llm("turn2").await.unwrap();
    let req2 = fake_ref.last_request().unwrap();
    assert_eq!(tool_messages(&req2).len(), 0);

    // Turn 3: snapshot updated, stable
    let _ = session.invoke_llm("turn3").await.unwrap();
    let req3 = fake_ref.last_request().unwrap();
    assert_eq!(tool_messages(&req3).len(), 0);
}
