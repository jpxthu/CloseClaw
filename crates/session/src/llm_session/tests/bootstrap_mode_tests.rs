//! Tests for `ConversationSession::bootstrap_mode` caching and propagation.
//!
//! Covers:
//! 1. Default bootstrap_mode is Full
//! 2. `with_bootstrap_mode()` builder sets the mode
//! 3. `bootstrap_mode()` getter returns the cached value
//! 4. `rebuild_system_prompt` propagates the explicitly passed mode to the builder
//! 5. Session-level `rebuild_system_prompt_for_session` reads cached mode

use super::super::*;
use closeclaw_common::{BootstrapMode, PromptOverrides, SystemPromptBuilder};
use std::sync::Arc;

// ── test doubles ──────────────────────────────────────────────────────────

/// Builder that records the bootstrap_mode_override it received.
struct ModeCapturingBuilder {
    recorded: std::sync::Mutex<Option<Option<BootstrapMode>>>,
}

impl ModeCapturingBuilder {
    fn new() -> Self {
        Self {
            recorded: std::sync::Mutex::new(None),
        }
    }

    fn captured_mode(&self) -> Option<Option<BootstrapMode>> {
        *self.recorded.lock().unwrap()
    }
}

#[async_trait::async_trait]
impl SystemPromptBuilder for ModeCapturingBuilder {
    async fn build_prompt(
        &self,
        _session_id: &str,
        _agent_id: &str,
        _overrides: Option<&PromptOverrides>,
        bootstrap_mode_override: Option<BootstrapMode>,
    ) -> String {
        *self.recorded.lock().unwrap() = Some(bootstrap_mode_override);
        format!("prompt-{:?}", bootstrap_mode_override)
    }

    async fn invalidate_cache(&self) {}
}

// ── default bootstrap_mode ────────────────────────────────────────────────

#[test]
fn test_bootstrap_mode_default_is_full() {
    let session = ConversationSession::new("s1".into(), "gpt-4o".into(), tmp_path());
    assert_eq!(session.bootstrap_mode(), BootstrapMode::Full);
}

// ── with_bootstrap_mode builder ───────────────────────────────────────────

#[test]
fn test_with_bootstrap_mode_sets_minimal() {
    let session = ConversationSession::new("s2".into(), "gpt-4o".into(), tmp_path())
        .with_bootstrap_mode(BootstrapMode::Minimal);
    assert_eq!(session.bootstrap_mode(), BootstrapMode::Minimal);
}

#[test]
fn test_with_bootstrap_mode_sets_full() {
    let session = ConversationSession::new("s3".into(), "gpt-4o".into(), tmp_path())
        .with_bootstrap_mode(BootstrapMode::Full);
    assert_eq!(session.bootstrap_mode(), BootstrapMode::Full);
}

#[test]
fn test_with_bootstrap_mode_overwrites_previous() {
    let session = ConversationSession::new("s4".into(), "gpt-4o".into(), tmp_path())
        .with_bootstrap_mode(BootstrapMode::Minimal)
        .with_bootstrap_mode(BootstrapMode::Full);
    assert_eq!(session.bootstrap_mode(), BootstrapMode::Full);
}

// ── bootstrap_mode getter returns cached value ────────────────────────────

#[test]
fn test_bootstrap_mode_getter_consistent() {
    let session = ConversationSession::new("s5".into(), "gpt-4o".into(), tmp_path())
        .with_bootstrap_mode(BootstrapMode::Minimal);
    assert_eq!(session.bootstrap_mode(), BootstrapMode::Minimal);
    assert_eq!(session.bootstrap_mode(), BootstrapMode::Minimal);
}

// ── rebuild_system_prompt passes override to builder ──────────────────────

/// When rebuild_system_prompt receives Some(mode), it passes it through
/// to the builder as bootstrap_mode_override.
#[tokio::test]
async fn test_rebuild_passes_full_mode_to_builder() {
    let builder = Arc::new(ModeCapturingBuilder::new());
    let mut session = ConversationSession::new("s6".into(), "gpt-4o".into(), tmp_path())
        .with_system_prompt("old");
    session.set_system_prompt_builder(builder.clone());

    session
        .rebuild_system_prompt("s6", "agent", Some(BootstrapMode::Full))
        .await;

    let captured = builder.captured_mode().expect("builder should be called");
    assert_eq!(captured, Some(BootstrapMode::Full));
}

#[tokio::test]
async fn test_rebuild_passes_minimal_mode_to_builder() {
    let builder = Arc::new(ModeCapturingBuilder::new());
    let mut session = ConversationSession::new("s7".into(), "gpt-4o".into(), tmp_path());
    session.set_system_prompt_builder(builder.clone());

    session
        .rebuild_system_prompt("s7", "agent", Some(BootstrapMode::Minimal))
        .await;

    let captured = builder.captured_mode().expect("builder should be called");
    assert_eq!(captured, Some(BootstrapMode::Minimal));
}

// ── rebuild_system_prompt with None override ──────────────────────────────

/// When rebuild_system_prompt receives None, the builder sees None.
#[tokio::test]
async fn test_rebuild_none_override_passes_none_to_builder() {
    let builder = Arc::new(ModeCapturingBuilder::new());
    let mut session = ConversationSession::new("s8".into(), "gpt-4o".into(), tmp_path());
    session.set_system_prompt_builder(builder.clone());

    session.rebuild_system_prompt("s8", "agent", None).await;

    let captured = builder.captured_mode().expect("builder should be called");
    assert_eq!(captured, None);
}

// ── no builder → rebuild is noop, bootstrap_mode unchanged ────────────────

#[tokio::test]
async fn test_rebuild_no_builder_preserves_bootstrap_mode() {
    let mut session = ConversationSession::new("s9".into(), "gpt-4o".into(), tmp_path())
        .with_bootstrap_mode(BootstrapMode::Minimal);
    assert!(!session.has_system_prompt_builder());

    session.rebuild_system_prompt("s9", "agent", None).await;

    assert_eq!(session.bootstrap_mode(), BootstrapMode::Minimal);
}

// ── bootstrap_mode survives clone (Clone derive) ──────────────────────────

#[test]
fn test_bootstrap_mode_clone_preserves_value() {
    let session = ConversationSession::new("s10".into(), "gpt-4o".into(), tmp_path())
        .with_bootstrap_mode(BootstrapMode::Minimal);
    let cloned = session.clone();
    assert_eq!(cloned.bootstrap_mode(), BootstrapMode::Minimal);
}

// ── Non-spawn path: cache from registry, then rebuild ─────────────────────

/// Simulate the non-spawn resolve path: AgentRegistry provides bootstrap_mode,
/// session caches it, and rebuild_system_prompt is called with cached value.
#[tokio::test]
async fn test_non_spawn_path_caches_and_rebuilds() {
    let builder = Arc::new(ModeCapturingBuilder::new());
    // Simulate: AgentRegistry returns Minimal for this agent
    let bootstrap_mode = BootstrapMode::Minimal;

    let mut session = ConversationSession::new("s11".into(), "gpt-4o".into(), tmp_path())
        .with_bootstrap_mode(bootstrap_mode);
    session.set_system_prompt_builder(builder.clone());

    // Rebuild: caller reads cached mode and passes it
    session
        .rebuild_system_prompt("s11", "agent", Some(bootstrap_mode))
        .await;

    assert_eq!(session.bootstrap_mode(), BootstrapMode::Minimal);
    let captured = builder.captured_mode().expect("builder should be called");
    assert_eq!(captured, Some(BootstrapMode::Minimal));
}

// ── Spawn path: cache from config, then rebuild ───────────────────────────

/// Simulate the spawn path: child session gets bootstrap_mode from config,
/// caches it, and rebuild_system_prompt is called with cached value.
#[tokio::test]
async fn test_spawn_path_caches_config_bootstrap_mode() {
    let builder = Arc::new(ModeCapturingBuilder::new());
    // Spawn path: config.bootstrap_mode = Full
    let config_mode = BootstrapMode::Full;

    let mut session = ConversationSession::new("s12".into(), "gpt-4o".into(), tmp_path())
        .with_bootstrap_mode(config_mode);
    session.set_system_prompt_builder(builder.clone());

    session
        .rebuild_system_prompt("s12", "child-agent", Some(config_mode))
        .await;

    assert_eq!(session.bootstrap_mode(), BootstrapMode::Full);
    let captured = builder.captured_mode().expect("builder should be called");
    assert_eq!(captured, Some(BootstrapMode::Full));
}

// ── rebuild_system_prompt updates cached system_prompt ────────────────────

#[tokio::test]
async fn test_rebuild_updates_system_prompt_string() {
    let builder = Arc::new(ModeCapturingBuilder::new());
    let mut session = ConversationSession::new("s13".into(), "gpt-4o".into(), tmp_path())
        .with_system_prompt("old-prompt");
    session.set_system_prompt_builder(builder.clone());

    session
        .rebuild_system_prompt("s13", "agent", Some(BootstrapMode::Full))
        .await;

    // The system_prompt should be updated to the builder's return value
    let prompt = session.system_prompt().unwrap();
    assert!(
        prompt.contains("Full"),
        "prompt should reflect the mode: {}",
        prompt
    );
}
