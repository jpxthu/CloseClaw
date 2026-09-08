//! Unit tests for `ConversationSession::rebuild_system_prompt`.
//!
//! Covers the normal path (builder rebuilds prompt and replaces),
//! edge cases for the `overrides` parameter, and the no-builder path.

use super::super::*;
use closeclaw_common::{PromptOverrides, SessionRole, SystemPromptBuilder};

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
