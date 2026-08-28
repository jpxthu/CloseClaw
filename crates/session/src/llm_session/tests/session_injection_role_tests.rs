//! Tests for session-injection design doc fixes (Step 1.1 & 1.2).
//!
//! Validates the two `must_fix` gaps identified in the design doc scan:
//!
//! - **Gap 1**: `/new` session creation path now calls
//!   `wire_skill_listing_deps`, so the session holds a
//!   `skill_listing_provider` and `agent_skills` after creation.
//!   Tested at the session layer: setter/getter roundtrip, provider
//!   invocation, and conditional activation path.
//!
//! - **Gap 2**: Skill listing is injected as `role: "system"` (not
//!   `role: "tool"`), matching the design doc's "以系统消息形式注入".
//!   Memory injection retains `role: "tool"` (unaffected).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::{ConditionalSkillMatch, LLMError, LlmCaller, SkillListingProvider};

use super::tmp_path;
use crate::llm_session::{ConversationSession, InjectionPosition, MemoryInjection};
use closeclaw_common::processor::{ContentBlock, UnifiedResponse, UnifiedUsage};

// ---------------------------------------------------------------------------
// Mock SkillListingProvider
// ---------------------------------------------------------------------------

/// Mock provider returning configurable listing content.
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
// FakeLlmCaller (captures request for assertion)
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

/// Helper: extract system-role messages (skill listing).
fn system_messages(req: &InternalRequest) -> Vec<&str> {
    req.messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.as_str())
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════
// Gap 1: session holds skill_listing_provider and agent_skills after /new
// ═══════════════════════════════════════════════════════════════════════════

/// After setting provider and agent_skills (simulating the `/new` path's
/// `wire_skill_listing_deps` call), the session holds both fields and
/// the provider is invoked during `invoke_llm`.
#[tokio::test]
async fn test_gap1_session_holds_provider_and_agent_skills() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a\n- **skill_b**: desc_b",
        "- **skill_a**: desc_a\n- **skill_b**: desc_b",
    ));
    let mut session = ConversationSession::new("gap1_1".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider.clone());
    session.set_agent_skills(vec!["skill_a".into(), "skill_b".into()]);

    // Verify fields are set
    assert!(session.skill_listing_provider().is_some());
    assert!(session.agent_skills().is_some());
    assert_eq!(session.agent_skills().unwrap(), &["skill_a", "skill_b"]);

    // Verify provider is invoked during invoke_llm
    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    let systems = system_messages(&req);
    assert_eq!(systems.len(), 1);
    assert!(systems[0].contains("skill_a"));
}

/// When no provider is set (pre-Step-1.1 state), invoke_llm produces
/// no system-role skill listing message.
#[tokio::test]
async fn test_gap1_no_provider_no_listing() {
    let mut session = ConversationSession::new("gap1_2".into(), "m".into(), tmp_path());
    // No provider set — simulates the old /new path before Step 1.1

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    let systems = system_messages(&req);
    assert_eq!(systems.len(), 0, "no system-role message without provider");
}

/// Conditional activation path: after provider and agent_skills are set,
/// a user message containing a file path triggers conditional skill
/// activation and the activated skill appears in the next turn's listing.
#[tokio::test]
async fn test_gap1_conditional_activation_after_provider_set() {
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

    let mut session = ConversationSession::new("gap1_3".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider.clone());
    session.set_agent_skills(vec!["skill_a".into()]);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    // Turn 1: no conditional activation
    let _ = session.invoke_llm("hello").await.unwrap();
    let req1 = fake_ref.last_request().unwrap();
    let systems1 = system_messages(&req1);
    assert_eq!(systems1.len(), 1);
    assert!(!systems1[0].contains("rs_helper"));

    // Turn 2: .rs file triggers activation (applied next turn)
    let _ = session.invoke_llm("edit src/main.rs").await.unwrap();

    // Turn 3: activated skill appears
    let _ = session.invoke_llm("continue").await.unwrap();
    let req3 = fake_ref.last_request().unwrap();
    let systems3 = system_messages(&req3);
    assert_eq!(systems3.len(), 1);
    assert!(
        systems3[0].contains("rs_helper"),
        "activated conditional skill should appear in listing"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Gap 2: skill listing role is "system", memory injection role is "tool"
// ═══════════════════════════════════════════════════════════════════════════

/// Non-empty skill listing is injected as role "system" at position 0.
#[tokio::test]
async fn test_gap2_listing_uses_system_role() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("gap2_1".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    // Skill listing should be the first message with role "system"
    assert!(
        !req.messages.is_empty(),
        "should have at least the listing + user messages"
    );
    assert_eq!(
        req.messages[0].role, "system",
        "skill listing must use role 'system', not 'tool'"
    );
    assert!(req.messages[0].content.contains("skill_a"));
}

/// Empty skill listing produces no system-role message.
#[tokio::test]
async fn test_gap2_empty_listing_no_injection() {
    let provider = Arc::new(MockProvider::new("", ""));
    let mut session = ConversationSession::new("gap2_2".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    let systems = system_messages(&req);
    assert_eq!(
        systems.len(),
        0,
        "empty listing should not inject any system message"
    );
    // Only the user message
    assert_eq!(req.messages.len(), 1);
    assert_eq!(req.messages[0].role, "user");
}

/// Memory injection retains role "tool" (unaffected by Gap 2 fix).
/// Both AfterCurrent and BeforeNext positions use "tool" role.
#[tokio::test]
async fn test_gap2_memory_injection_retains_tool_role() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("gap2_3".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    // AfterCurrent: memory after user message
    let injection = MemoryInjection::new("memory_after".into(), InjectionPosition::AfterCurrent);
    session.set_memory_injection(injection);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    // Expected: [system: listing, user: hello, tool: memory_after]
    assert_eq!(req.messages.len(), 3);
    assert_eq!(req.messages[0].role, "system", "listing is system");
    assert_eq!(req.messages[1].role, "user");
    assert_eq!(
        req.messages[2].role, "tool",
        "memory injection must remain role 'tool'"
    );
    assert_eq!(req.messages[2].content, "memory_after");
}

/// Memory injection with BeforeNext position also uses "tool" role.
#[tokio::test]
async fn test_gap2_memory_injection_before_next_tool_role() {
    let provider = Arc::new(MockProvider::new(
        "- **skill_a**: desc_a",
        "- **skill_a**: desc_a",
    ));
    let mut session = ConversationSession::new("gap2_4".into(), "m".into(), tmp_path());
    session.set_skill_listing_provider(provider);

    let injection = MemoryInjection::new("memory_before".into(), InjectionPosition::BeforeNext);
    session.set_memory_injection(injection);

    let fake = Arc::new(FakeLlmCaller::new("ok"));
    let fake_ref = fake.clone();
    session.set_llm_caller(fake);

    let _ = session.invoke_llm("hello").await.unwrap();

    let req = fake_ref.last_request().unwrap();
    // Expected: [system: listing, tool: memory_before, user: hello]
    assert_eq!(req.messages.len(), 3);
    assert_eq!(req.messages[0].role, "system", "listing is system");
    assert_eq!(
        req.messages[1].role, "tool",
        "memory injection must remain role 'tool'"
    );
    assert_eq!(req.messages[1].content, "memory_before");
    assert_eq!(req.messages[2].role, "user");
}
