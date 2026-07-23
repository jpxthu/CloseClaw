//! Step 1.4 tests — label auto-generation for sessions_spawn.

use super::spawn::SpawnMode;
use super::tests::{clear_global_prompt_state, test_config};
use super::{ChildSessionConfig, SessionManager};
use closeclaw_session::persistence::{PersistenceService, ReasoningLevel};
use serial_test::serial;
use std::sync::Arc;

use super::spawn_tests::{register_parent_session, test_resolved_config};

// ── Unit tests: default_spawn_label ─────────────────────────────────────

/// Verify `default_spawn_label` returns the expected format
/// `"spawn-{nanosecond_timestamp}"`.
#[test]
fn test_default_spawn_label_format() {
    use super::spawn::default_spawn_label;
    let label = default_spawn_label();
    assert!(
        label.starts_with("spawn-"),
        "default label should start with 'spawn-': {}",
        label
    );
    let ts_part = &label[6..]; // strip "spawn-"
    let ts: u128 = ts_part
        .parse()
        .expect("timestamp part should be a valid u128");
    assert!(
        ts > 1_700_000_000_000_000_000,
        "timestamp should be a reasonable nanosecond value: {}",
        ts
    );
}

/// Verify two consecutive calls produce different labels (uniqueness).
#[test]
fn test_default_spawn_label_uniqueness() {
    use super::spawn::default_spawn_label;
    let a = default_spawn_label();
    let b = default_spawn_label();
    // Extremely unlikely to be equal since nanosecond timestamps differ
    assert_ne!(a, b, "two labels should not be equal");
}

// ── Integration tests: label in checkpoint ──────────────────────────────

/// When label is `None`, `create_child_session` auto-generates a label
/// in the checkpoint with format `"spawn-{nanosecond_timestamp}"`.
#[tokio::test]
#[serial]
async fn test_spawn_label_auto_generated_when_none() {
    clear_global_prompt_state();
    let tmp = tempfile::TempDir::new().unwrap();
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        Some(tmp.path().to_path_buf()),
        ReasoningLevel::default(),
    );
    let config = test_resolved_config("label-child", None);
    register_parent_session(&mgr, "parent-label", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-label",
            1,
            "label test task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None, // spawn_timeout
            None, // label
            None, // prompt_template_prefix
        )
        .await
        .expect("create_child_session should succeed");

    let child_cp = storage
        .load_checkpoint(&child_id)
        .await
        .expect("storage should be accessible")
        .expect("child checkpoint should exist");
    let label = child_cp
        .label
        .as_ref()
        .expect("auto-generated label should be present");
    assert!(
        label.starts_with("spawn-"),
        "auto-generated label should start with 'spawn-': {}",
        label
    );
    let ts_part = &label[6..];
    assert!(
        ts_part.parse::<u128>().is_ok(),
        "timestamp part should be a valid u128: {}",
        ts_part
    );
}

/// When label is `Some("my-label")`, the provided label is preserved
/// in the checkpoint.
#[tokio::test]
#[serial]
async fn test_spawn_label_explicit_value_preserved() {
    clear_global_prompt_state();
    let tmp = tempfile::TempDir::new().unwrap();
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        Some(tmp.path().to_path_buf()),
        ReasoningLevel::default(),
    );
    let config = test_resolved_config("label-child-explicit", None);
    register_parent_session(&mgr, "parent-label-explicit", tmp.path().to_path_buf()).await;

    let child_id = mgr
        .create_child_session(
            &config,
            "parent-label-explicit",
            1,
            "explicit label task",
            false,
            None,
            SpawnMode::Run,
            false,
            None,
            None,
            None,
            3,
            None,                    // spawn_timeout
            Some("my-custom-label"), // label
            None,                    // prompt_template_prefix
        )
        .await
        .expect("create_child_session should succeed");

    let child_cp = storage
        .load_checkpoint(&child_id)
        .await
        .expect("storage should be accessible")
        .expect("child checkpoint should exist");
    assert_eq!(
        child_cp.label.as_deref(),
        Some("my-custom-label"),
        "explicit label should be preserved in checkpoint"
    );
}

/// Verify `ChildSessionConfig.label` auto-generation path works
/// (label=None → auto-generated label in checkpoint).
#[tokio::test]
#[serial]
async fn test_spawn_label_via_config_none_generates() {
    clear_global_prompt_state();
    let tmp = tempfile::TempDir::new().unwrap();
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());
    let mgr = SessionManager::new(
        &test_config(),
        Some(storage.clone()),
        Some(tmp.path().to_path_buf()),
        ReasoningLevel::default(),
    );
    let config = test_resolved_config("label-config-child", None);
    register_parent_session(&mgr, "parent-label-config", tmp.path().to_path_buf()).await;

    let child_config = ChildSessionConfig {
        config,
        parent_session_id: "parent-label-config".to_string(),
        depth: 1,
        task: "config label test".to_string(),
        light_context: false,
        workspace: None,
        mode: SpawnMode::Run,
        fork: false,
        allowed_tools: None,
        model_override: None,
        parent_subagents_model: None,
        max_spawn_depth: 3,
        spawn_timeout: None,
        label: None, // auto-generate
        prompt_template_prefix: None,
    };
    let child_id = mgr
        .create_child_session_with_config(child_config)
        .await
        .expect("create_child_session_with_config should succeed");

    let child_cp = storage
        .load_checkpoint(&child_id)
        .await
        .expect("storage should be accessible")
        .expect("child checkpoint should exist");
    let label = child_cp
        .label
        .as_ref()
        .expect("auto-generated label should be present via config path");
    assert!(
        label.starts_with("spawn-"),
        "auto-generated label should start with 'spawn-': {}",
        label
    );
}
