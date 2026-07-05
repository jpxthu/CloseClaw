//! Unit tests for `ConversationSession::rebuild_system_prompt`.
//!
//! Covers the normal path (builder rebuilds prompt and replaces),
//! edge cases for the `overrides` parameter, and the no-builder path.

use super::super::*;
use closeclaw_common::{PromptOverrides, SystemPromptBuilder};

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
        .rebuild_system_prompt("sess_rebuild", "agent_1")
        .await;

    assert_eq!(session.system_prompt(), Some("new system prompt"));
}

#[tokio::test]
async fn test_rebuild_system_prompt_overwrites_existing() {
    let mut session = new_session_with_builder(Arc::new(MockBuilder::new("replaced prompt")))
        .with_system_prompt("old prompt");

    session
        .rebuild_system_prompt("sess_rebuild", "agent_1")
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
        .rebuild_system_prompt("sess_rebuild", "agent_1")
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
        .rebuild_system_prompt("sess_rebuild", "agent_1")
        .await;

    assert_eq!(session.system_prompt(), Some("prompt-for-agent_1"));
}

// ── edge case: no builder ────────────────────────────────────────────────

#[tokio::test]
async fn test_rebuild_system_prompt_no_builder_is_noop() {
    let mut session = new_session();
    assert!(!session.has_system_prompt_builder());

    session
        .rebuild_system_prompt("sess_rebuild", "agent_1")
        .await;

    assert!(session.system_prompt().is_none());
}

// ── edge case: empty prompt from builder ─────────────────────────────────

#[tokio::test]
async fn test_rebuild_system_prompt_builder_returns_empty() {
    let mut session = new_session_with_builder(Arc::new(MockBuilder::new("")));

    session
        .rebuild_system_prompt("sess_rebuild", "agent_1")
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
