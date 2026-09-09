//! Unit tests for `ConversationSession::rebuild_system_prompt`.
//!
//! Covers the normal path (builder rebuilds prompt and replaces),
//! edge cases for the `overrides` parameter, the no-builder path,
//! the activated-conditional-skills clearing behavior, and the
//! InjectionParams contract (§注入链路的参数契约).

use super::super::*;
use closeclaw_common::{PromptOverrides, SessionRole, SkillListingProvider, SystemPromptBuilder};
use std::collections::HashSet;
use std::sync::{Arc, Mutex as StdMutex};

// ── Fake ToolRegistryQuery for testing ────────────────────────────────────

struct FakeToolRegistryQuery;

#[async_trait::async_trait]
impl closeclaw_common::ToolRegistryQuery for FakeToolRegistryQuery {
    async fn list_tool_names(&self) -> Vec<String> {
        Vec::new()
    }
    async fn get_tool_descriptors(
        &self,
        _agent_id: Option<&str>,
        _agent_tools: Option<&[String]>,
        _agent_disallowed_tools: Option<&[String]>,
    ) -> Vec<closeclaw_common::ToolDescriptor> {
        Vec::new()
    }
    async fn has_tool(&self, _name: &str) -> bool {
        false
    }
    async fn get_tool_schema(&self, _name: &str) -> Option<serde_json::Value> {
        None
    }
    async fn get_tool_detail(&self, _name: &str) -> Option<closeclaw_common::ToolDescriptor> {
        None
    }
    async fn list_tool_names_by_group(&self, _group: &str) -> Vec<String> {
        Vec::new()
    }
}

// ── test doubles ──────────────────────────────────────────────────────────

/// Mock builder that returns a fixed prompt string.
struct MockBuilder {
    prompt: String,
}

impl MockBuilder {
    fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
        }
    }
}

#[async_trait::async_trait]
impl SystemPromptBuilder for MockBuilder {
    async fn build_prompt(
        &self,
        _session_id: &str,
        _agent_id: &str,
        _overrides: Option<&PromptOverrides>,
        _bootstrap_mode_override: Option<closeclaw_common::BootstrapMode>,
        _session_role: SessionRole,
    ) -> String {
        self.prompt.clone()
    }

    async fn invalidate_cache(&self) {}
}

/// Builder that returns a prompt including the agent_id and overrides info.
struct CapturingBuilder;

#[async_trait::async_trait]
impl SystemPromptBuilder for CapturingBuilder {
    async fn build_prompt(
        &self,
        _session_id: &str,
        agent_id: &str,
        overrides: Option<&PromptOverrides>,
        _bootstrap_mode_override: Option<closeclaw_common::BootstrapMode>,
        _session_role: SessionRole,
    ) -> String {
        let base = format!("prompt-for-{}", agent_id);
        match overrides {
            Some(o) => format!(
                "{}|override={}",
                base,
                o.override_prompt.as_deref().unwrap_or("none")
            ),
            None => base,
        }
    }

    async fn invalidate_cache(&self) {}
}

/// Builder that records the `session_role` it received, for role-derivation tests.
struct RoleCapturingBuilder {
    captured_role: std::sync::Arc<tokio::sync::Mutex<Option<SessionRole>>>,
}

impl RoleCapturingBuilder {
    fn new(captured: std::sync::Arc<tokio::sync::Mutex<Option<SessionRole>>>) -> Self {
        Self {
            captured_role: captured,
        }
    }
}

#[async_trait::async_trait]
impl SystemPromptBuilder for RoleCapturingBuilder {
    async fn build_prompt(
        &self,
        _session_id: &str,
        _agent_id: &str,
        _overrides: Option<&PromptOverrides>,
        _bootstrap_mode_override: Option<closeclaw_common::BootstrapMode>,
        session_role: SessionRole,
    ) -> String {
        *self.captured_role.lock().await = Some(session_role);
        "prompt".to_string()
    }

    async fn invalidate_cache(&self) {}
}

/// Builder that captures the `activated_skills` vector passed to
/// `build_prompt_with_activated`, so tests can verify the session
/// passes the correct activation set to the builder.
struct ActivatedCapturingBuilder {
    captured: Arc<StdMutex<Vec<String>>>,
}

impl ActivatedCapturingBuilder {
    fn new(captured: Arc<StdMutex<Vec<String>>>) -> Self {
        Self { captured }
    }
}

#[async_trait::async_trait]
impl SystemPromptBuilder for ActivatedCapturingBuilder {
    async fn build_prompt(
        &self,
        _session_id: &str,
        _agent_id: &str,
        _overrides: Option<&PromptOverrides>,
        _bootstrap_mode_override: Option<closeclaw_common::BootstrapMode>,
        _session_role: SessionRole,
    ) -> String {
        "rebuilt-prompt".to_string()
    }

    async fn build_prompt_with_activated(
        &self,
        _session_id: &str,
        _agent_id: &str,
        _overrides: Option<&PromptOverrides>,
        _bootstrap_mode_override: Option<closeclaw_common::BootstrapMode>,
        activated_skills: Vec<String>,
        _session_role: SessionRole,
    ) -> String {
        *self.captured.lock().unwrap() = activated_skills;
        "rebuilt-prompt".to_string()
    }

    async fn invalidate_cache(&self) {}
}

/// Activation-aware `SkillListingProvider` mock.
///
/// - `generate_listing_excluding_conditional` → `base_listing`
///   (excludes conditional skills).
/// - `generate_listing` → `full_listing`
///   (baseline + all conditional skills).
///
/// This mirrors the real provider contract so that
/// `generate_listing_with_activated` on `ConversationSession`
/// produces different results depending on the activated set.
struct MockListingProvider {
    /// Baseline listing (excludes conditional skills).
    base_listing: String,
    /// Full listing (includes all conditional skills).
    full_listing: String,
}

impl MockListingProvider {
    /// Create with `base` for excluding-conditional path and `full`
    /// for the all-skills path.
    fn new(base: impl Into<String>, full: impl Into<String>) -> Self {
        Self {
            base_listing: base.into(),
            full_listing: full.into(),
        }
    }
}

impl SkillListingProvider for MockListingProvider {
    fn generate_listing(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        self.full_listing.clone()
    }

    fn generate_listing_excluding_conditional(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        self.base_listing.clone()
    }

    fn find_conditional_matches(
        &self,
        _paths: &[std::path::PathBuf],
    ) -> Vec<closeclaw_common::ConditionalSkillMatch> {
        Vec::new()
    }
}

// ── helpers ───────────────────────────────────────────────────────────────

fn new_session() -> ConversationSession {
    ConversationSession::new("sess_rebuild".into(), "gpt-4o".into(), tmp_path())
}

fn new_session_with_builder(builder: Arc<dyn SystemPromptBuilder>) -> ConversationSession {
    let mut s = new_session();
    s.set_system_prompt_builder(builder);
    s
}

// ── normal path ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_rebuild_system_prompt_replaces_prompt() {
    let mut session = new_session_with_builder(Arc::new(MockBuilder::new("new system prompt")));
    assert!(session.system_prompt().is_none());

    session
        .rebuild_system_prompt("sess_rebuild", "agent_1", None)
        .await;

    assert_eq!(session.system_prompt(), Some("new system prompt"));
}

#[tokio::test]
async fn test_rebuild_system_prompt_overwrites_existing() {
    let mut session = new_session_with_builder(Arc::new(MockBuilder::new("replaced prompt")))
        .with_system_prompt("old prompt");

    session
        .rebuild_system_prompt("sess_rebuild", "agent_1", None)
        .await;

    assert_eq!(session.system_prompt(), Some("replaced prompt"));
}

// ── edge case: overrides ─────────────────────────────────────────────────

#[tokio::test]
async fn test_rebuild_system_prompt_with_overrides() {
    let mut session = new_session();
    session.set_system_prompt_builder(Arc::new(CapturingBuilder));
    session.set_prompt_overrides(Some(PromptOverrides {
        override_prompt: Some("custom override".to_string()),
        agent_prompt: None,
        custom_prompt: None,
    }));

    session
        .rebuild_system_prompt("sess_rebuild", "agent_1", None)
        .await;

    assert_eq!(
        session.system_prompt(),
        Some("prompt-for-agent_1|override=custom override")
    );
}

#[tokio::test]
async fn test_rebuild_system_prompt_without_overrides() {
    let mut session = new_session_with_builder(Arc::new(CapturingBuilder));

    session
        .rebuild_system_prompt("sess_rebuild", "agent_1", None)
        .await;

    assert_eq!(session.system_prompt(), Some("prompt-for-agent_1"));
}

// ── edge case: no builder ────────────────────────────────────────────────

#[tokio::test]
async fn test_rebuild_system_prompt_no_builder_is_noop() {
    let mut session = new_session();
    assert!(!session.has_system_prompt_builder());

    session
        .rebuild_system_prompt("sess_rebuild", "agent_1", None)
        .await;

    assert!(session.system_prompt().is_none());
}

// ── edge case: empty prompt from builder ─────────────────────────────────

#[tokio::test]
async fn test_rebuild_system_prompt_builder_returns_empty() {
    let mut session = new_session_with_builder(Arc::new(MockBuilder::new("")));

    session
        .rebuild_system_prompt("sess_rebuild", "agent_1", None)
        .await;

    // Empty string is still set as the prompt
    assert_eq!(session.system_prompt(), Some(""));
}

// ── edge case: replace_system_prompt directly ────────────────────────────

#[test]
fn test_replace_system_prompt_sets_prompt() {
    let mut session = new_session();
    session.replace_system_prompt("direct set");
    assert_eq!(session.system_prompt(), Some("direct set"));
}

#[test]
fn test_replace_system_prompt_overwrites_existing() {
    let mut session = new_session().with_system_prompt("old");
    session.replace_system_prompt("new");
    assert_eq!(session.system_prompt(), Some("new"));
}

// ── setter tests ─────────────────────────────────────────────────────────

#[test]
fn test_has_system_prompt_builder() {
    let mut session = new_session();
    assert!(!session.has_system_prompt_builder());

    session.set_system_prompt_builder(Arc::new(MockBuilder::new("test")));
    assert!(session.has_system_prompt_builder());
}

// ── State transition: session_role derivation from is_sub_agent ───────────

/// When `is_sub_agent == false` (default), `rebuild_system_prompt` must
/// pass `SessionRole::Main` to the builder.
#[tokio::test]
async fn test_rebuild_system_prompt_main_session_passes_role_main() {
    let captured = Arc::new(tokio::sync::Mutex::new(None::<SessionRole>));
    let mut session =
        new_session_with_builder(Arc::new(RoleCapturingBuilder::new(captured.clone())));
    // Default is_sub_agent == false.
    assert!(!session.is_sub_agent());

    session.rebuild_system_prompt("sess", "agent-1", None).await;

    let role = captured.lock().await;
    assert_eq!(*role, Some(SessionRole::Main));
}

/// When `is_sub_agent == true`, `rebuild_system_prompt` must
/// pass `SessionRole::Sub` to the builder.
#[tokio::test]
async fn test_rebuild_system_prompt_sub_session_passes_role_sub() {
    let captured = Arc::new(tokio::sync::Mutex::new(None::<SessionRole>));
    let mut session =
        new_session_with_builder(Arc::new(RoleCapturingBuilder::new(captured.clone())));
    session.set_sub_agent(true);

    session.rebuild_system_prompt("sess", "agent-1", None).await;

    let role = captured.lock().await;
    assert_eq!(*role, Some(SessionRole::Sub));
}

/// Role derivation is independent of bootstrap_mode_override.
#[tokio::test]
async fn test_rebuild_system_prompt_role_independent_of_bootstrap_mode() {
    use closeclaw_common::BootstrapMode;

    let captured = Arc::new(tokio::sync::Mutex::new(None::<SessionRole>));
    let mut session =
        new_session_with_builder(Arc::new(RoleCapturingBuilder::new(captured.clone())));
    // Main session + Minimal bootstrap mode → role must still be Main.
    session.set_sub_agent(false);

    session
        .rebuild_system_prompt("sess", "agent-1", Some(BootstrapMode::Minimal))
        .await;

    let role = captured.lock().await;
    assert_eq!(*role, Some(SessionRole::Main));
}

/// Sub session with Full bootstrap mode → role must be Sub.
#[tokio::test]
async fn test_rebuild_system_prompt_sub_role_with_full_bootstrap() {
    use closeclaw_common::BootstrapMode;

    let captured = Arc::new(tokio::sync::Mutex::new(None::<SessionRole>));
    let mut session =
        new_session_with_builder(Arc::new(RoleCapturingBuilder::new(captured.clone())));
    session.set_sub_agent(true);

    session
        .rebuild_system_prompt("sess", "agent-1", Some(BootstrapMode::Full))
        .await;

    let role = captured.lock().await;
    assert_eq!(*role, Some(SessionRole::Sub));
}

// ── Normal path regression: main session prompt unchanged ────────────────

/// Builder that returns a deterministic prompt and records session_role.
struct RegressionBuilder {
    role_recorded: Arc<tokio::sync::Mutex<Vec<SessionRole>>>,
}

#[async_trait::async_trait]
impl SystemPromptBuilder for RegressionBuilder {
    async fn build_prompt(
        &self,
        _session_id: &str,
        agent_id: &str,
        _overrides: Option<&PromptOverrides>,
        _bootstrap_mode_override: Option<closeclaw_common::BootstrapMode>,
        session_role: SessionRole,
    ) -> String {
        self.role_recorded.lock().await.push(session_role);
        format!("prompt-for-{}", agent_id)
    }

    async fn invalidate_cache(&self) {}
}

/// Builder path: activate 2 conditional skills, rebuild, then verify:
/// (1) the builder receives the 2 activated skill names;
/// (2) `activated_conditional_skills` is empty after rebuild.
///
/// Verifies doc semantics: "标记并入静态层即清除，避免重复渲染"
#[tokio::test]
async fn test_rebuild_clears_activated_conditional_skills() {
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let mut session =
        new_session_with_builder(Arc::new(ActivatedCapturingBuilder::new(captured.clone())));

    // Simulate two conditional skills being activated via
    // apply_skill_listing_update (the per-turn activation path).
    let mut newly: HashSet<String> = HashSet::new();
    newly.insert("skill-a".into());
    newly.insert("skill-b".into());
    session.apply_skill_listing_update(None, &newly);
    assert_eq!(session.activated_conditional_skills().len(), 2);

    // Rebuild SP — builder must receive both activated skills.
    session.rebuild_system_prompt("sess", "agent", None).await;

    let received = captured.lock().unwrap();
    assert_eq!(received.len(), 2, "builder must receive 2 activated skills");
    assert!(received.contains(&"skill-a".to_string()));
    assert!(received.contains(&"skill-b".to_string()));
    drop(received);

    // After rebuild, the activation set must be cleared.
    assert!(
        session.activated_conditional_skills().is_empty(),
        "activated_conditional_skills must be empty after rebuild (标记并入静态层即清除)"
    );
}

/// No-builder path: `activated_conditional_skills` must NOT be cleared
/// because no rendering occurred.
///
/// Verifies doc semantics: only clear when builder exists and completes
/// rendering ("标记并入静态层"前提不存在时不清空)
#[tokio::test]
async fn test_rebuild_no_builder_preserves_activated_skills() {
    let mut session = new_session();
    assert!(!session.has_system_prompt_builder());

    let mut newly: HashSet<String> = HashSet::new();
    newly.insert("skill-x".into());
    session.apply_skill_listing_update(None, &newly);
    assert_eq!(session.activated_conditional_skills().len(), 1);

    let result = session.rebuild_system_prompt("sess", "agent", None).await;

    // Returns empty string (no builder).
    assert!(result.is_empty());
    // Activation set preserved — no rendering happened, so no clear.
    assert_eq!(
        session.activated_conditional_skills().len(),
        1,
        "no-builder path must not clear activated skills"
    );
}

/// State transition: after rebuild clears, re-activating the same skill
/// adds it back to the set.
///
/// Verifies doc semantics: "新激活的技能重新标记"
#[tokio::test]
async fn test_rebuild_then_reactivate_same_skill() {
    let mut session = new_session_with_builder(Arc::new(MockBuilder::new("prompt")));

    // First activation + rebuild cycle.
    let mut newly: HashSet<String> = HashSet::new();
    newly.insert("skill-a".into());
    session.apply_skill_listing_update(None, &newly);
    assert_eq!(session.activated_conditional_skills().len(), 1);

    session.rebuild_system_prompt("sess", "agent", None).await;
    assert!(session.activated_conditional_skills().is_empty());

    // Re-activate the same skill — must be accepted again.
    let mut re: HashSet<String> = HashSet::new();
    re.insert("skill-a".into());
    session.apply_skill_listing_update(None, &re);
    assert_eq!(
        session.activated_conditional_skills().len(),
        1,
        "新激活的技能重新标记: same skill must be re-added after rebuild"
    );
    assert!(session.activated_conditional_skills().contains("skill-a"));
}

/// State transition: after rebuild clears, `compute_skill_listing_for_turn`
/// generates a listing that no longer contains the previously merged
/// conditional skill entries.
///
/// Verifies doc semantics: "此后不再需要 per-turn 增量注入（针对已并入条目）"
///
/// The activation-aware `MockListingProvider` returns:
/// - `generate_listing_excluding_conditional` → baseline only
/// - `generate_listing` → baseline + conditional skills
///
/// This ensures the test exercises the real `generate_listing_with_activated`
/// filtering logic on `ConversationSession` and verifies the causal
/// relationship between rebuild-clear and listing exclusion.
#[tokio::test]
async fn test_rebuild_listing_excludes_previously_merged_skills() {
    let mut session = new_session_with_builder(Arc::new(MockBuilder::new("prompt")));

    // Activation-aware provider: generate_listing_excluding_conditional
    // returns baseline only; generate_listing returns baseline + conditional
    // skills in the `**name**` format expected by generate_listing_with_activated.
    let provider: Arc<dyn SkillListingProvider> = Arc::new(MockListingProvider::new(
        "base-skill",
        "base-skill\n- **cond-skill**: test conditional",
    ));
    session.skill_listing_provider = Some(provider);

    // ── Step 1: activate + snapshot ──────────────────────────────────
    // Activate a conditional skill and generate the listing for this
    // turn so the snapshot includes the conditional entry.
    let mut newly: HashSet<String> = HashSet::new();
    newly.insert("cond-skill".into());
    session.apply_skill_listing_update(None, &newly);
    assert_eq!(session.activated_conditional_skills().len(), 1);

    // First turn: full listing (base + cond-skill), snapshot established.
    let (listing1, snap1) = session.compute_skill_listing_for_turn();
    let text1 = listing1.unwrap_or_default();
    assert_eq!(text1, "base-skill\n- **cond-skill**: test conditional");
    // Apply snapshot so the next call computes a diff.
    session.apply_skill_listing_update(snap1, &HashSet::new());

    // ── Step 2: rebuild clears activation ────────────────────────────
    session.rebuild_system_prompt("sess", "agent", None).await;
    assert!(
        session.activated_conditional_skills().is_empty(),
        "activated set must be empty after rebuild"
    );

    // Listing now excludes cond-skill → diff shows deletion.
    let (listing2, snap2) = session.compute_skill_listing_for_turn();
    let text2 = listing2.unwrap_or_default();
    assert_eq!(
        text2, "- - **cond-skill**: test conditional",
        "after rebuild-clear, listing diff must show cond-skill removal"
    );
    // Apply snapshot so the next call computes against the cleared state.
    session.apply_skill_listing_update(snap2, &HashSet::new());

    // ── Step 3: re-activate same skill ──────────────────────────────
    // "新激活的技能重新标记": re-activating the same skill adds it back.
    let mut re: HashSet<String> = HashSet::new();
    re.insert("cond-skill".into());
    session.apply_skill_listing_update(None, &re);
    assert_eq!(session.activated_conditional_skills().len(), 1);

    // Listing re-includes cond-skill → diff shows addition.
    let (listing3, _snap3) = session.compute_skill_listing_for_turn();
    let text3 = listing3.unwrap_or_default();
    assert_eq!(
        text3, "- **cond-skill**: test conditional",
        "after re-activation, listing diff must show cond-skill addition"
    );
}

/// Boundary: empty activation set → rebuild doesn't panic and
/// behaviour is unchanged.
///
/// Verifies that clearing an empty set is a safe no-op.
#[tokio::test]
async fn test_rebuild_empty_activation_set_no_panic() {
    let mut session = new_session_with_builder(Arc::new(MockBuilder::new("ok")));
    assert!(session.activated_conditional_skills().is_empty());

    let prompt = session.rebuild_system_prompt("sess", "agent", None).await;

    assert_eq!(prompt, "ok");
    assert!(session.activated_conditional_skills().is_empty());
}

/// Boundary: rebuild twice in a row with no activation in between
/// is idempotent.
#[tokio::test]
async fn test_rebuild_twice_idempotent() {
    let mut session = new_session_with_builder(Arc::new(MockBuilder::new("prompt")));

    let p1 = session.rebuild_system_prompt("sess", "agent", None).await;
    let p2 = session.rebuild_system_prompt("sess", "agent", None).await;

    assert_eq!(p1, p2);
    assert!(session.activated_conditional_skills().is_empty());
}

/// Main session rebuild produces the same prompt output regardless of the
/// new `session_role` field — no behavioral regression.
#[tokio::test]
async fn test_rebuild_system_prompt_main_session_output_unchanged() {
    let role_recorded = Arc::new(tokio::sync::Mutex::new(Vec::<SessionRole>::new()));
    let mut session = new_session_with_builder(Arc::new(RegressionBuilder {
        role_recorded: role_recorded.clone(),
    }));

    // First rebuild (main session).
    let prompt1 = session
        .rebuild_system_prompt("sess", "agent-regression", None)
        .await;
    assert_eq!(prompt1, "prompt-for-agent-regression");

    // Second rebuild — output must be identical.
    let prompt2 = session
        .rebuild_system_prompt("sess", "agent-regression", None)
        .await;
    assert_eq!(prompt1, prompt2);

    // Both rebuilds used Main role.
    let roles = role_recorded.lock().await;
    assert_eq!(roles.len(), 2);
    assert!(roles.iter().all(|r| *r == SessionRole::Main));
}

// ═══════════════════════════════════════════════════════════════════════════
// InjectionParams contract (§注入链路的参数契约)
// ═══════════════════════════════════════════════════════════════════════════

/// Builder that captures the [`InjectionParams`] passed to
/// `build_prompt_with_params`, so tests can verify the contract
/// fields without depending on the real builder implementation.
struct ParamsCapturingBuilder {
    captured: Arc<StdMutex<Option<InjectionParams>>>,
}

impl ParamsCapturingBuilder {
    fn new(captured: Arc<StdMutex<Option<InjectionParams>>>) -> Self {
        Self { captured }
    }
}

#[async_trait::async_trait]
impl SystemPromptBuilder for ParamsCapturingBuilder {
    async fn build_prompt(
        &self,
        _session_id: &str,
        _agent_id: &str,
        _overrides: Option<&PromptOverrides>,
        _bootstrap_mode_override: Option<closeclaw_common::BootstrapMode>,
        _session_role: SessionRole,
    ) -> String {
        "default-prompt".to_string()
    }

    async fn build_prompt_with_params(
        &self,
        params: &closeclaw_common::injection_params::InjectionParams,
    ) -> String {
        *self.captured.lock().unwrap() = Some(params.clone());
        "params-prompt".to_string()
    }

    async fn invalidate_cache(&self) {}
}

/// rebuild_system_prompt passes tool_registry from setter into
/// InjectionParams.tool_registry.
#[tokio::test]
async fn test_rebuild_passes_tool_registry_to_injection_params() {
    let captured = Arc::new(StdMutex::new(None::<InjectionParams>));
    let mut session =
        new_session_with_builder(Arc::new(ParamsCapturingBuilder::new(captured.clone())));

    // Inject a tool_registry via the setter.
    let fake_registry: Arc<dyn closeclaw_common::ToolRegistryQuery> =
        Arc::new(FakeToolRegistryQuery);
    let registry_ptr = Arc::as_ptr(&fake_registry);
    session.set_tool_registry(fake_registry);

    session.rebuild_system_prompt("sess", "agent-1", None).await;

    let params = captured.lock().unwrap();
    let p = params
        .as_ref()
        .expect("build_prompt_with_params should have been called");
    let reg = p
        .tool_registry
        .as_ref()
        .expect("tool_registry should be Some");
    assert_eq!(
        Arc::as_ptr(reg),
        registry_ptr,
        "tool_registry in params must point to the same Arc as the setter"
    );
}

/// rebuild_system_prompt passes session_id and agent_id into params.
#[tokio::test]
async fn test_rebuild_passes_session_id_and_agent_id() {
    let captured = Arc::new(StdMutex::new(None::<InjectionParams>));
    let mut session =
        new_session_with_builder(Arc::new(ParamsCapturingBuilder::new(captured.clone())));

    session
        .rebuild_system_prompt("my-session", "my-agent", None)
        .await;

    let p = captured.lock().unwrap();
    let p = p.as_ref().unwrap();
    assert_eq!(p.session_id, "my-session");
    assert_eq!(p.agent_id, "my-agent");
}

/// rebuild_system_prompt passes overrides from setter into params.
#[tokio::test]
async fn test_rebuild_passes_overrides_to_params() {
    let captured = Arc::new(StdMutex::new(None::<InjectionParams>));
    let mut session =
        new_session_with_builder(Arc::new(ParamsCapturingBuilder::new(captured.clone())));
    session.set_prompt_overrides(Some(PromptOverrides {
        override_prompt: Some("custom-ov".to_string()),
        agent_prompt: None,
        custom_prompt: None,
    }));

    session.rebuild_system_prompt("sess", "agent", None).await;

    let p = captured.lock().unwrap();
    let p = p.as_ref().unwrap();
    assert!(p.overrides.is_some());
    assert_eq!(
        p.overrides.as_ref().unwrap().override_prompt.as_deref(),
        Some("custom-ov")
    );
}

/// rebuild_system_prompt derives session_role from is_sub_agent flag.
#[tokio::test]
async fn test_rebuild_passes_session_role_main() {
    let captured = Arc::new(StdMutex::new(None::<InjectionParams>));
    let mut session =
        new_session_with_builder(Arc::new(ParamsCapturingBuilder::new(captured.clone())));
    // Default is_sub_agent == false → Main.

    session.rebuild_system_prompt("sess", "agent", None).await;

    let p = captured.lock().unwrap();
    let p = p.as_ref().unwrap();
    assert_eq!(p.session_role, SessionRole::Main);
}

/// rebuild_system_prompt derives Sub role when is_sub_agent is true.
#[tokio::test]
async fn test_rebuild_passes_session_role_sub() {
    let captured = Arc::new(StdMutex::new(None::<InjectionParams>));
    let mut session =
        new_session_with_builder(Arc::new(ParamsCapturingBuilder::new(captured.clone())));
    session.set_sub_agent(true);

    session.rebuild_system_prompt("sess", "agent", None).await;

    let p = captured.lock().unwrap();
    let p = p.as_ref().unwrap();
    assert_eq!(p.session_role, SessionRole::Sub);
}

/// rebuild_system_prompt passes activated_conditional_skills into params.
#[tokio::test]
async fn test_rebuild_passes_activated_skills_to_params() {
    let captured = Arc::new(StdMutex::new(None::<InjectionParams>));
    let mut session =
        new_session_with_builder(Arc::new(ParamsCapturingBuilder::new(captured.clone())));

    // Simulate two activated conditional skills.
    let mut newly: HashSet<String> = HashSet::new();
    newly.insert("skill-a".into());
    newly.insert("skill-b".into());
    session.apply_skill_listing_update(None, &newly);
    assert_eq!(session.activated_conditional_skills().len(), 2);

    session.rebuild_system_prompt("sess", "agent", None).await;

    let p = captured.lock().unwrap();
    let p = p.as_ref().unwrap();
    assert_eq!(p.activated_skills.len(), 2);
    assert!(p.activated_skills.contains(&"skill-a".to_string()));
    assert!(p.activated_skills.contains(&"skill-b".to_string()));
}

/// rebuild_system_prompt passes bootstrap_mode_override into params.
#[tokio::test]
async fn test_rebuild_passes_bootstrap_mode_override_to_params() {
    use closeclaw_common::BootstrapMode;

    let captured = Arc::new(StdMutex::new(None::<InjectionParams>));
    let mut session =
        new_session_with_builder(Arc::new(ParamsCapturingBuilder::new(captured.clone())));

    session
        .rebuild_system_prompt("sess", "agent", Some(BootstrapMode::Minimal))
        .await;

    let p = captured.lock().unwrap();
    let p = p.as_ref().unwrap();
    assert_eq!(p.bootstrap_mode_override, Some(BootstrapMode::Minimal));
}

/// Boundary: session without tool_registry (None) — params.tool_registry is
/// None, chain must not panic and falls back to default.
#[tokio::test]
async fn test_rebuild_no_registry_params_none() {
    let captured = Arc::new(StdMutex::new(None::<InjectionParams>));
    let mut session =
        new_session_with_builder(Arc::new(ParamsCapturingBuilder::new(captured.clone())));
    // No set_tool_registry call — registry stays None.

    let prompt = session.rebuild_system_prompt("sess", "agent", None).await;

    assert_eq!(prompt, "params-prompt");
    let p = captured.lock().unwrap();
    let p = p.as_ref().unwrap();
    assert!(
        p.tool_registry.is_none(),
        "tool_registry must be None when not set"
    );
}
