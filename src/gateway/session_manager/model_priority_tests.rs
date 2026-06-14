//! Tests for the model priority chain in `create_child_session`.
//!
//! Verifies the 4-layer priority:
//!   explicit model param > parent agent.subagents.model
//!   > target agent.model > system default
//!
//! These are separated from `spawn_tests.rs` to keep both files
//! under the 500-line file-size limit.

use super::spawn::SpawnMode;
use super::tests::{clear_global_prompt_state, make_test_mgr};
use super::SessionManager;
use crate::agent::config::SubagentsConfig;
use crate::config::agents::{ConfigSource, ResolvedAgentConfig};
use crate::session::bootstrap::BootstrapMode;
use serial_test::serial;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::llm::session::ConversationSession;

fn test_resolved_config(
    id: &str,
    model: Option<&str>,
    workspace: Option<PathBuf>,
) -> ResolvedAgentConfig {
    ResolvedAgentConfig {
        id: id.to_string(),
        name: id.to_string(),
        parent_id: None,
        model: model.map(String::from),
        workspace,
        agent_dir: None,
        bootstrap_mode: BootstrapMode::Full,
        skills: vec![],
        tools: vec![],
        disallowed_tools: vec![],
        subagents: SubagentsConfig::default(),
        source: ConfigSource::Merged,
    }
}

async fn register_parent_session(mgr: &SessionManager, parent_id: &str, workdir: PathBuf) {
    let cs = ConversationSession::new(parent_id.to_string(), "test-model".to_string(), workdir);
    let arc = Arc::new(RwLock::new(cs));
    let mut conv = mgr.conversation_sessions.write().await;
    conv.insert(parent_id.to_string(), arc);
}

/// Verify the model chosen on the ConversationSession matches the
/// expected value.
async fn assert_child_model(mgr: &SessionManager, child_id: &str, expected_model: &str) {
    let cs = mgr
        .get_conversation_session(child_id)
        .await
        .expect("child ConversationSession should exist");
    let guard = cs.read().await;
    let actual = guard.model();
    assert_eq!(
        actual, expected_model,
        "child model mismatch: expected '{}', got '{}'",
        expected_model, actual
    );
}

// ── 1. Explicit model_override wins over everything ────────────────────────

/// When `model_override` is `Some("override-model")`, it should be
/// used regardless of config values or parent_subagents_model.
#[tokio::test]
#[serial]
async fn test_model_priority_explicit_override_wins() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("target-a", Some("target-model"), None);

    register_parent_session(&mgr, "p1", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "p1",
            1,
            "task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            Some("override-model"),   // model_override
            Some("parent-sub-model"), // parent_subagents_model
        )
        .await
        .expect("create_child_session should succeed");

    assert_child_model(&mgr, &child_id, "override-model").await;
}

// ── 2. Parent subagents.model wins over target agent.model ────────────────

/// When `model_override` is `None` but `parent_subagents_model` is set,
/// the parent's choice should win over the target agent's model.
#[tokio::test]
#[serial]
async fn test_model_priority_parent_subagents_wins() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("target-b", Some("target-model"), None);

    register_parent_session(&mgr, "p2", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "p2",
            1,
            "task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,                    // model_override
            Some("parent-override"), // parent_subagents_model
        )
        .await
        .expect("create_child_session should succeed");

    assert_child_model(&mgr, &child_id, "parent-override").await;
}

// ── 3. Target agent.model used when no override/subagents.model ───────────

/// When neither `model_override` nor `parent_subagents_model` is set,
/// the target agent's own `model` field should be used.
#[tokio::test]
#[serial]
async fn test_model_priority_target_agent_model() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("target-c", Some("my-custom-model"), None);

    register_parent_session(&mgr, "p3", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "p3",
            1,
            "task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None, // model_override
            None, // parent_subagents_model
        )
        .await
        .expect("create_child_session should succeed");

    assert_child_model(&mgr, &child_id, "my-custom-model").await;
}

// ── 4. System default used when nothing else is set ───────────────────────

/// When `model_override`, `parent_subagents_model`, and `config.model`
/// are all `None`, the system default `"default"` should be used.
#[tokio::test]
#[serial]
async fn test_model_priority_system_default() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("target-d", None, None);

    register_parent_session(&mgr, "p4", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "p4",
            1,
            "task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None, // model_override
            None, // parent_subagents_model
        )
        .await
        .expect("create_child_session should succeed");

    assert_child_model(&mgr, &child_id, "default").await;
}

// ── 5. Parent subagents.model beats target model when no override ─────────

/// When `model_override` is `None` but `parent_subagents_model` is set,
/// it should take priority over the target agent's `model`.
#[tokio::test]
#[serial]
async fn test_model_priority_parent_subagents_beats_target() {
    clear_global_prompt_state();

    let tmp = tempfile::TempDir::new().unwrap();
    let mgr = make_test_mgr(Some(tmp.path()));
    let config = test_resolved_config("target-e", Some("target-model"), None);

    register_parent_session(&mgr, "p5", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "p5",
            1,
            "task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,                       // model_override
            Some("subagents-override"), // parent_subagents_model
        )
        .await
        .expect("create_child_session should succeed");

    // parent_subagents_model should win over target model
    assert_child_model(&mgr, &child_id, "subagents-override").await;
}
