//! Unit tests for noBootstrap propagation through the spawn chain.
//!
//! Covers issue #3475 Step 1.6:
//! - `SpawnTargetAgentConfig.no_bootstrap` is cached on the child
//!   `ConversationSession` before the creation-time rebuild;
//! - the creation-time rebuild's InjectionParams carry `no_bootstrap`,
//!   so the injection chain resolves "no bootstrap files" for the
//!   spawned sub-session;
//! - default (no declaration) keeps the existing behavior (`false`).

use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use super::creation::create_child_conversation_session;
use super::creation_tests::MockCreationContext;
use super::types::SpawnMode;
use crate::spawn::controller::SpawnTargetAgentConfig;
use crate::spawn::creation::ChildSessionCreationParams;

fn make_config(id: &str, no_bootstrap: bool) -> SpawnTargetAgentConfig {
    SpawnTargetAgentConfig {
        id: id.to_string(),
        no_bootstrap,
        ..Default::default()
    }
}

fn default_params<'a>() -> ChildSessionCreationParams<'a> {
    ChildSessionCreationParams {
        parent_session_id: "parent-session",
        parent_agent_id: "parent-agent",
        depth: 0,
        task: "test task",
        light_context: false,
        workspace: None,
        mode: SpawnMode::Run,
        fork: false,
        model_override: None,
        parent_subagents_model: None,
        max_spawn_depth: 3,
        prompt_template_prefix: None,
        timeout_warning_secs: None,
        timeout_notify_interval_ratio: None,
        debug_log: None,
        trace_id: "",
        session_key: None,
    }
}

/// Config declaring no_bootstrap=true → child session caches the flag
/// before the creation-time rebuild.
#[tokio::test]
async fn test_no_bootstrap_true_cached_on_child_session() {
    let ctx = MockCreationContext::new();
    let config = make_config("child-agent", true);
    let params = default_params();

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("should succeed");

    let cs = result.conversation_session.read().await;
    assert!(
        cs.no_bootstrap(),
        "noBootstrap agent's child session must cache no_bootstrap=true"
    );
}

/// Default (no declaration) keeps the existing behavior: flag stays false.
#[tokio::test]
async fn test_no_bootstrap_default_false_on_child_session() {
    let ctx = MockCreationContext::new();
    let config = make_config("child-agent", false);
    let params = default_params();

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("should succeed");

    let cs = result.conversation_session.read().await;
    assert!(
        !cs.no_bootstrap(),
        "default spawn must keep no_bootstrap=false"
    );
}

/// Creation-time rebuild (before which the flag is set) must pass
/// `no_bootstrap=true` into InjectionParams — the injection chain then
/// loads no bootstrap files (and no MEMORY.md) for the sub-session.
#[tokio::test]
async fn test_creation_rebuild_injection_params_carry_no_bootstrap_true() {
    let captured = Arc::new(StdMutex::new(None));
    let ctx = MockCreationContext::with_params_capture(captured.clone());
    let config = make_config("child-agent", true);
    let params = default_params();

    let result = create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("should succeed");

    // Session-level flag must already be set (cached before rebuild).
    assert!(result.conversation_session.read().await.no_bootstrap());

    let guard = captured.lock().unwrap();
    let p = guard
        .as_ref()
        .expect("creation-time rebuild must call build_prompt_with_params");
    assert!(
        p.no_bootstrap,
        "creation-time rebuild's InjectionParams must carry no_bootstrap=true"
    );
    assert_eq!(
        p.session_role,
        closeclaw_common::SessionRole::Sub,
        "spawned session must rebuild as Sub role"
    );
}

/// Control: default config → creation-time rebuild's InjectionParams
/// carry no_bootstrap=false (behavior unchanged).
#[tokio::test]
async fn test_creation_rebuild_injection_params_default_false() {
    let captured = Arc::new(StdMutex::new(None));
    let ctx = MockCreationContext::with_params_capture(captured.clone());
    let config = make_config("child-agent", false);
    let params = default_params();

    create_child_conversation_session(&ctx, &config, &params)
        .await
        .expect("should succeed");

    let guard = captured.lock().unwrap();
    let p = guard.as_ref().expect("rebuild must capture params");
    assert!(!p.no_bootstrap, "default must stay false");
}
